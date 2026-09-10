// Package main es el binario del sidecar de HexCell: el proceso Go que acompaña al núcleo Rust dentro de una célula
// sobre canal propio y que habla el protocolo de WhatsApp a través de whatsmeow.
//
// El sidecar es un **coste permanente** del canal propio (adr-0014), no un andamio de transición.
//
// Este archivo es cableado: carga la configuración, abre el almacén de dispositivo, construye
// el registro, construye la sesión de whatsmeow, abre el servidor del socket IPC de dominio Unix
// (docs/protocolo-ipc-nucleo-sidecar.md) y conecta los sumideros de eventos, estado y acuses.
package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"github.com/CGary/hexcell/sidecar/internal/servidor"
)

// Nombres fijos de suceso del arranque y de la parada del proceso.
const (
	eventoArranque   = "sidecar.arrancado"
	eventoParada     = "sidecar.detenido"
	eventoErrorDreno = "outbox.error_drenaje"
)

func main() {
	cfg, err := configuracion.Cargar(os.LookupEnv)
	if err != nil {
		// El registro todavía no existe: la configuración es justo lo que lo parametriza. Un
		// fallo aquí va a stderr y termina el proceso con código distinto de cero.
		fmt.Fprintf(os.Stderr, "hexcell-sidecar: configuración inválida: %v\n", err)
		os.Exit(1)
	}

	reg := registro.Nuevo(os.Stdout, cfg.NivelDeRegistro, cfg.IdCelula)
	reg.Info(eventoArranque, registro.Campos{
		Detalle: fmt.Sprintf(
			"protocolo IPC versión %d; socket en %s; sqlstore en %s",
			ipc.VersionProtocolo, cfg.RutaSocket, cfg.RutaSqlstore,
		),
	})

	ctx := context.Background()

	// Apertura de los seis recursos fríos (sqlstore, sesión, respaldo de sqlstore, identidad,
	// respaldo de identidad, outbox) extraída a abrirRecursosDeArranque para poder probarla
	// contra un directorio temporal. El orden interno garantiza que identidad.db se crea antes
	// que su conexión de respaldo de solo lectura; ver arranque.go y bitácora D-43.
	recursos, err := abrirRecursosDeArranque(ctx, cfg, reg)
	if err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	defer recursos.Contenedor.Close()
	defer canal.CerrarDB(recursos.DBRespaldo)
	defer canal.CerrarDB(recursos.DBRespaldoIdentidad)
	defer recursos.AlmacenIdentidad.Cerrar()
	defer recursos.Buzon.Cerrar()

	// La ColaDeSalida comparte el archivo y la conexión con el outbox.
	transmisor := outbox.NuevoTransmisorWhatsmeow(recursos.Sesion.Cliente(), recursos.AlmacenIdentidad)
	disciplina := outbox.NuevaDisciplinaDeSalida(cfg.Disciplina)
	emisorPresencia := outbox.NuevoEmisorDePresenciaWhatsmeow(recursos.Sesion.Cliente(), recursos.AlmacenIdentidad, reg)
	colaSalida := outbox.NuevaColaDeSalida(recursos.Buzon.DB(), cfg.TtlSalidaMs, cfg.IntentosMaximosSalida, reg, transmisor, recursos.AlmacenIdentidad).
		ConDisciplina(disciplina, emisorPresencia).
		ConCortacircuitos(recursos.AlmacenIdentidad)
	portero := outbox.NuevoPorteroDeSalida(colaSalida, recursos.AlmacenIdentidad, recursos.AlmacenIdentidad, recursos.AlmacenIdentidad, reg)

	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket:          cfg.RutaSocket,
		IdCelula:            cfg.IdCelula,
		Registro:            reg,
		Buzon:               recursos.Buzon,
		Portero:             portero,
		DBRespaldo:          recursos.DBRespaldo,
		DBRespaldoIdentidad: recursos.DBRespaldoIdentidad,
		Sesion:              recursos.Sesion,
		TelefonoCelula:      cfg.TelefonoCelula,
	})

	colaSalida.ConSumideroDeAcuse(srv.EnviarAcuseEnvio)

	supervisor := canal.NuevoSupervisor(reg, cfg.Retroceso, recursos.Sesion.Conectar, srv.EnviarEstadoSesion)
	recursos.Sesion.RegistrarManejador(supervisor)
	go supervisor.Arrancar(ctx, recursos.Sesion.EstaEmparejada())

	detectorBaja := canal.NuevoDetectorDeBaja(cfg.PalabrasDeBaja, cfg.TextoConfirmacionDeBaja, recursos.AlmacenIdentidad, portero)
	detectorCortacircuitos := canal.NuevoDetectorDeCortacircuitos(
		cfg.Cortacircuitos.UmbralRepeticion,
		cfg.Cortacircuitos.PalabrasFrustracion,
		cfg.Cortacircuitos.TextoTraspaso,
		recursos.AlmacenIdentidad,
		portero,
	)
	generadorPresentacion := canal.NuevoGeneradorDePresentacion(
		cfg.Presentacion.Variantes,
		cfg.Presentacion.TextoIdentificacion,
		portero,
	)

	intervaloDrenaje := time.Duration(cfg.Disciplina.IntervaloDrenajeMs) * time.Millisecond
	ctxDrenaje, detenerDrenaje := context.WithCancel(ctx)
	defer detenerDrenaje()
	go bucleDeDrenajeSalida(ctxDrenaje, colaSalida, intervaloDrenaje, reg)

	sumideroEvento := func(evento ipc.EventoEntrante) {
		reg.Info("canal.evento_entrante_listo", registro.Campos{
			IdEvento: evento.IdDeduplicacion,
		})
		srv.NotificarTrabajo()
	}
	traductor := canal.NuevoTraductor(recursos.AlmacenIdentidad, recursos.Buzon, sumideroEvento, nil, reg, detectorBaja, detectorCortacircuitos, generadorPresentacion)
	recursos.Sesion.RegistrarTraductor(traductor)

	if err := srv.Escuchar(ctx); err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	defer srv.Cerrar()
	go srv.Aceptar(ctx)

	// Parada ordenada: SIGTERM y SIGINT cierran el servidor IPC y la sesión de whatsmeow.
	senales := make(chan os.Signal, 1)
	signal.Notify(senales, syscall.SIGTERM, syscall.SIGINT)
	senal := <-senales

	srv.Cerrar()
	recursos.Sesion.Cerrar()
	reg.Info(eventoParada, registro.Campos{Detalle: senal.String()})
}

// bucleDeDrenajeSalida ejecuta ColaDeSalida.Drenar a intervalos regulares hasta que ctx se
// cancela en la parada ordenada. Cada vuelta expira lo vencido e intenta transmitir lo
// pendiente; un error de una vuelta se registra y no detiene el bucle, exactamente como el
// resto del proceso sobrevive a un fallo de un solo evento.
func bucleDeDrenajeSalida(ctx context.Context, cola *outbox.ColaDeSalida, intervalo time.Duration, reg *registro.Registro) {
	ticker := time.NewTicker(intervalo)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			if err := cola.Drenar(ctx, time.Now().UnixMilli()); err != nil && reg != nil {
				reg.Error(eventoErrorDreno, registro.Campos{Detalle: err.Error()})
			}
		}
	}
}
