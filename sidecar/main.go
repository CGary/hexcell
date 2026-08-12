// Binario del sidecar de HexCell: el proceso Go que acompaña al núcleo Rust dentro de una célula
// sobre canal propio y que habla el protocolo de WhatsApp a través de whatsmeow.
//
// El sidecar es un **coste permanente** del canal propio (adr-0014), no un andamio de transición.
//
// Este archivo es cableado y nada más: carga la configuración, abre el almacén de dispositivo,
// construye el registro, construye la sesión de whatsmeow y engancha el manejador de eventos
// crudos. La conexión real al canal está fuera de esta tarea (tarea 2 del plan de la etapa A-3):
// sin credenciales emparejadas whatsmeow no puede completar un inicio de sesión.
//
// El servidor del socket IPC de docs/protocolo-ipc-nucleo-sidecar.md tampoco se abre aquí: es la
// tarea 3, junto con el outbox durable que le da sentido. Lo que sí existe ya es la representación
// tipada de sus mensajes, en internal/ipc.
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
	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
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
			"protocolo IPC versión %d; socket previsto en %s; sqlstore en %s",
			ipc.VersionProtocolo, cfg.RutaSocket, cfg.RutaSqlstore,
		),
	})

	ctx := context.Background()

	contenedor, err := canal.AbrirAlmacenDeDispositivo(ctx, cfg.RutaSqlstore, reg)
	if err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	defer contenedor.Close()

	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	supervisor := canal.NuevoSupervisor(reg, cfg.Retroceso, sesion.Conectar, nil)
	sesion.RegistrarManejador(supervisor)

	almacenIdentidad, err := identidad.Abrir(identidad.Opciones{
		Ruta:     cfg.RutaIdentidad,
		Registro: reg,
	})
	if err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	defer almacenIdentidad.Cerrar()

	buzon, err := outbox.Abrir(outbox.Opciones{Ruta: outbox.RutaPorOmision, Registro: reg})
	if err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	defer buzon.Cerrar()

	// La ColaDeSalida comparte el archivo y la conexión con el outbox.
	// El transmisor real vive dentro del paquete outbox (transmisor.go); aquí se inyecta el
	// cliente whatsmeow de la sesión activa y el almacén de identidad como resolutor de
	// direcciones y control de baja.
	transmisor := outbox.NuevoTransmisorWhatsmeow(sesion.Cliente(), almacenIdentidad)
	disciplina := outbox.NuevaDisciplinaDeSalida(cfg.Disciplina)
	emisorPresencia := outbox.NuevoEmisorDePresenciaWhatsmeow(sesion.Cliente(), almacenIdentidad, reg)
	colaSalida := outbox.NuevaColaDeSalida(buzon.DB(), cfg.TtlSalidaMs, cfg.IntentosMaximosSalida, reg, transmisor, almacenIdentidad).ConDisciplina(disciplina, emisorPresencia).ConCortacircuitos(almacenIdentidad)
	portero := outbox.NuevoPorteroDeSalida(colaSalida, almacenIdentidad, almacenIdentidad, reg)
	detectorBaja := canal.NuevoDetectorDeBaja(cfg.PalabrasDeBaja, cfg.TextoConfirmacionDeBaja, almacenIdentidad, portero)
	detectorCortacircuitos := canal.NuevoDetectorDeCortacircuitos(
		cfg.Cortacircuitos.UmbralRepeticion,
		cfg.Cortacircuitos.PalabrasFrustracion,
		cfg.Cortacircuitos.TextoTraspaso,
		almacenIdentidad,
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
	}
	traductor := canal.NuevoTraductor(almacenIdentidad, buzon, sumideroEvento, nil, reg, detectorBaja, detectorCortacircuitos)
	sesion.RegistrarTraductor(traductor)

	// Parada ordenada: SIGTERM es la señal con la que un runtime de contenedores detiene el
	// proceso y SIGINT la de una ejecución en terminal. Las dos cierran la sesión antes de salir,
	// en lugar de dejar que el proceso muera con el cliente a medio desmontar.
	senales := make(chan os.Signal, 1)
	signal.Notify(senales, syscall.SIGTERM, syscall.SIGINT)
	senal := <-senales

	sesion.Cerrar()
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
