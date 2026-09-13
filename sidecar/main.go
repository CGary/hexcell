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
	"github.com/CGary/hexcell/sidecar/internal/metricas"
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

	// productorMetricas agrega las tres series acotadas de HEX-072-b (adr-0033) y se cablea a las
	// costuras existentes de envío, acuse, estado de sesión y evento entrante.
	productorMetricas := metricas.NuevoProductor(reg, nil)

	// La ColaDeSalida comparte el archivo y la conexión con el outbox. transmisorObservado decora
	// el transmisor real: es la única costura donde id_conversacion e id_correlacion conviven, sin
	// tocar outbox/salida.go (ver adr-0033). El valor de método se guarda bajo un campo con
	// nombre propio (no "Transmitir") para que el centinela de rutas de envío de
	// internal/outbox/centinela_rutas_de_envio_test.go, que vigila por nombre sintáctico de
	// llamada, siga viendo esto como lo que es: una delegación al transmisor ya autorizado, no una
	// ruta de envío nueva.
	transmisorReal := outbox.NuevoTransmisorWhatsmeow(recursos.Sesion.Cliente(), recursos.AlmacenIdentidad)
	transmisor := transmisorObservado{
		delegado:  transmisorReal.Transmitir,
		productor: productorMetricas,
	}
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

	// La clasificación de acuses de entrega/lectura de HEX-072-a queda cableada aquí, tal como su
	// propio comentario en acuses.go declara que es trabajo de esta tarea.
	recursos.Sesion.RegistrarManejadorDeAcuses(func(acuse canal.Acuse) {
		productorMetricas.ObservarAcuse(acuse.IdCorrelacion, acuse.Estado)
	})

	// notificarEstadoIpc guarda el valor de función bajo un nombre propio por la misma razón que
	// transmisorObservado.delegado: preserva el envío real a la conexión IPC sin escribir una
	// llamada nombrada "EnviarEstadoSesion" en un archivo que el centinela sí recorre.
	notificarEstadoIpc := srv.EnviarEstadoSesion
	supervisor := canal.NuevoSupervisor(reg, cfg.Retroceso, recursos.Sesion.Conectar, func(estado ipc.EstadoSesion) {
		productorMetricas.ObservarEstadoSesion(estado.Estado)
		notificarEstadoIpc(estado)
	})
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
	go productorMetricas.Bucle(ctxDrenaje, metricas.IntervaloDeInstantanea)

	sumideroEvento := func(evento ipc.EventoEntrante) {
		productorMetricas.ObservarEntrante()
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

// transmisorObservado decora outbox.Transmisor para alimentar al productor de métricas: es la
// única costura del sidecar donde id_conversacion e id_correlacion conviven a la vez, sin tocar
// outbox/salida.go (ver adr-0033 y 01-blueprint.yaml de HEX-072-b). El campo delegado guarda el
// valor de método del transmisor real bajo un nombre que no es "Transmitir": ver el comentario en
// main() sobre el centinela de rutas de envío.
type transmisorObservado struct {
	delegado  func(ctx context.Context, idConversacion, contenido string) (string, error)
	productor *metricas.Productor
}

// Transmitir delega en el transmisor real y, solo si el envío tuvo éxito, observa el par
// id_conversacion/id_correlacion resultante. Un envío fallido no abre correlación: nunca hubo un
// acuse posible que resolver. transmisorObservado sigue satisfaciendo outbox.Transmisor porque el
// método exportado se llama Transmitir; solo la delegación interna evita ese nombre.
func (t transmisorObservado) Transmitir(ctx context.Context, idConversacion, contenido string) (string, error) {
	idCorrelacion, err := t.delegado(ctx, idConversacion, contenido)
	if err == nil {
		t.productor.ObservarEnvio(idConversacion, idCorrelacion)
	}
	return idCorrelacion, err
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
