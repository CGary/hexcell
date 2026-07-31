// Binario del sidecar de HexCell: el proceso Go que acompaña al núcleo Rust dentro de una célula
// sobre canal propio y que habla el protocolo de WhatsApp a través de whatsmeow.
//
// El sidecar es un **coste permanente** del canal propio (adr-0014), no un andamio de transición.
//
// Este archivo es cableado y nada más: carga la configuración, construye el registro, construye
// la sesión de whatsmeow, engancha el manejador de eventos crudos y espera una señal de parada
// para cerrar de forma ordenada. La conexión real al canal está fuera de esta tarea (tarea 2 del
// plan de la etapa A-3): sin credenciales emparejadas —tareas 4 y 5— whatsmeow no puede completar
// un inicio de sesión, así que llamar a Conectar aquí solo produciría un fallo garantizado en el
// arranque. El punto de entrada existe en internal/canal.
//
// El servidor del socket IPC de docs/protocolo-ipc-nucleo-sidecar.md tampoco se abre aquí: es la
// tarea 3, junto con el outbox durable que le da sentido. Lo que sí existe ya es la representación
// tipada de sus mensajes, en internal/ipc.
package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Nombres fijos de suceso del arranque y de la parada del proceso.
const (
	eventoArranque = "sidecar.arrancado"
	eventoParada   = "sidecar.detenido"
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
			"protocolo IPC versión %d; socket previsto en %s; sin conexión al canal todavía",
			ipc.VersionProtocolo, cfg.RutaSocket,
		),
	})

	sesion, err := canal.NuevaSesion(reg)
	if err != nil {
		reg.Error(eventoParada, registro.Campos{Detalle: err.Error()})
		os.Exit(1)
	}
	sesion.RegistrarManejador()

	// Parada ordenada: SIGTERM es la señal con la que un runtime de contenedores detiene el
	// proceso y SIGINT la de una ejecución en terminal. Las dos cierran la sesión antes de salir,
	// en lugar de dejar que el proceso muera con el cliente a medio desmontar.
	senales := make(chan os.Signal, 1)
	signal.Notify(senales, syscall.SIGTERM, syscall.SIGINT)
	senal := <-senales

	sesion.Cerrar()
	reg.Info(eventoParada, registro.Campos{Detalle: senal.String()})
}
