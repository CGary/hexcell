// Package canal construye la sesión de whatsmeow del sidecar y recibe sus eventos crudos.
//
// # Qué hace y qué no hace todavía
//
// Esta es la tarea 2 del plan de la etapa A-3: arranque, cableado del cliente, recepción de
// eventos crudos y registro estructurado. Deliberadamente **no** hay emparejamiento por QR ni por
// código (tarea 4), **no** hay persistencia de sesión en el `sqlstore` (tarea 5), **no** hay
// reconexión con retroceso (tarea 6) y **no** hay traducción al formato canónico del puerto
// (tarea 8). El almacén de dispositivo es `store.NoopDevice`, que no persiste nada.
//
// La consecuencia es que este esqueleto **no puede completar un inicio de sesión**: whatsmeow
// necesita credenciales emparejadas y aquí no las hay. Eso no es una carencia que corregir con
// prisa, es lo que hace que toda la batería de tests corra sin ningún número de WhatsApp, sin
// teléfono y sin red.
//
// # Por qué el manejador de eventos solo registra el tipo
//
// Un evento de whatsmeow puede llevar el texto de un mensaje. El manejador escribe el **tipo** del
// evento y nada de su contenido, que es la misma frontera estructural de privacidad que adr-0019
// impone al registro del núcleo. La traducción del contenido ocurre en la tarea 8 y va al outbox y
// al socket, nunca a un log.
package canal

import (
	"context"
	"errors"
	"fmt"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Nombres fijos de suceso que este paquete emite. Son constantes por el motivo de adr-0019:
// ningún valor construido en tiempo de ejecución puede acabar en el campo `evento`.
const (
	EventoSesionConstruida = "canal.sesion_construida"
	EventoCrudoRecibido    = "canal.evento_crudo_recibido"
	EventoSesionCerrada    = "canal.sesion_cerrada"
)

// ModuloWhatsmeow es el nombre de módulo raíz con el que la biblioteca aparece en el registro.
const ModuloWhatsmeow = "whatsmeow"

// ErrRegistroNoEspecificado se devuelve si se intenta construir una sesión sin registro.
var ErrRegistroNoEspecificado = errors.New("canal: la sesión necesita un registro")

// Sesion es el cliente de whatsmeow del sidecar junto al registro con el que informa.
type Sesion struct {
	cliente  *whatsmeow.Client
	registro *registro.Registro
}

// NuevaSesion construye el cliente de whatsmeow sin abrir ninguna conexión. El almacén de
// dispositivo es `store.NoopDevice`: no persiste nada y no tiene credenciales, de modo que la
// sesión puede construirse, inspeccionarse y cerrarse en un test sin tocar la red.
func NuevaSesion(reg *registro.Registro) (*Sesion, error) {
	if reg == nil {
		return nil, ErrRegistroNoEspecificado
	}

	puente := registro.NuevoAdaptadorWaLog(reg, ModuloWhatsmeow)
	cliente := whatsmeow.NewClient(store.NoopDevice, puente)

	reg.Info(EventoSesionConstruida, registro.Campos{
		Detalle: "cliente whatsmeow construido sobre almacén no persistente; sin conexión",
	})
	return &Sesion{cliente: cliente, registro: reg}, nil
}

// Cliente devuelve el cliente de whatsmeow subyacente.
//
// Se expone para que las tareas posteriores de la etapa —emparejamiento, persistencia,
// reconexión— construyan sobre él sin que este paquete tenga que anticipar su superficie.
func (s *Sesion) Cliente() *whatsmeow.Client {
	return s.cliente
}

// RegistrarManejador engancha el manejador de eventos crudos y devuelve su identificador.
//
// El manejador registra el **tipo** de cada evento recibido y nada más. La traducción al formato
// canónico del puerto, con su identificador de deduplicación, es la tarea 8; el paso previo —
// persistir en el outbox durable antes de cualquier otra cosa— es la tarea 3, y este manejador
// será el punto donde se enganche.
func (s *Sesion) RegistrarManejador() uint32 {
	return s.cliente.AddEventHandler(func(evento any) {
		s.registro.Info(EventoCrudoRecibido, registro.Campos{
			Detalle: fmt.Sprintf("%T", evento),
		})
	})
}

// Conectar abre el websocket saliente hacia WhatsApp.
//
// Ningún test de esta tarea la llama: sin credenciales emparejadas whatsmeow no puede completar el
// inicio de sesión, y probar contra el canal real es la tarea 15 del plan. El punto de entrada
// existe para que la tarea 4 tenga dónde engancharse.
func (s *Sesion) Conectar(ctx context.Context) error {
	return s.cliente.ConnectContext(ctx)
}

// Cerrar desconecta el cliente de forma ordenada.
func (s *Sesion) Cerrar() {
	s.cliente.Disconnect()
	s.registro.Info(EventoSesionCerrada, registro.Campos{})
}
