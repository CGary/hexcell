// Acuses de entrega y lectura: clasificación de los events.Receipt de whatsmeow.
//
// whatsmeow emite un *events.Receipt cuando un mensaje saliente es entregado al dispositivo o
// leído por el destinatario. Este archivo es la primera mitad del cableado de HEX-072: filtra esos
// eventos y expone la clasificación —entregado o leído— a través de un sumidero **en proceso**,
// reutilizando las constantes [ipc.EstadoEnvioEntregado] y [ipc.EstadoEnvioLeido] ya declaradas.
//
// El acuse NUNCA se convierte en un mensaje IPC: no hay socket, ni outbox, ni codificación aquí.
// El sumidero es una función que vive dentro del propio proceso del sidecar; la señal cruda que
// produce será consumida por el productor de métricas periódicas de HEX-072-b, no por el núcleo.
// Por eso docs/protocolo-ipc-nucleo-sidecar.md, su versión de cable y todo crate Rust quedan
// intactos (ver D-45 en docs/bitacora-de-descartes.md).
//
// # La trampa del tipo entregado
//
// types.ReceiptTypeDelivered es la cadena vacía (""). Comparar Type contra la constante nombrada
// es necesario pero no suficiente: un events.Receipt{} de valor cero también tiene Type=="". El
// discriminador real es len(r.MessageIDs) > 0: todo camino de whatsmeow que despacha un
// *events.Receipt le asigna al menos un MessageIDs antes del dispatch (receipt.go: parseReceipt y
// handleGroupedReceipt), de modo que un struct de valor cero nunca es un acuse genuino.
package canal

import (
	"fmt"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// EventoAcuseClasificado es el nombre fijo de suceso que se emite cuando un acuse se clasifica.
// Lleva el estado clasificado y nada más: nunca el contenido del mensaje ni ningún JID, la misma
// frontera de privacidad que EventoCrudoRecibido documenta en canal.go.
const EventoAcuseClasificado = "canal.acuse_clasificado"

// Acuse es el objeto de valor de la clasificación de un acuse, local a este paquete.
//
// Es deliberadamente NO un ipc.AcuseEnvio: vive solo dentro del proceso y no se puede codificar
// por IPC, de modo que la frontera en-proceso es un hecho de tipo y no una convención. IdCorrelacion
// es el MessageID de whatsmeow (events.Receipt.MessageIDs[0]), el mismo valor que
// outbox.ColaDeSalida.MarcarEnviado guarda como id_correlacion —nunca el id_mensaje interno del
// outbox, que whatsmeow no observa y por tanto jamás aparece en un Receipt.
type Acuse struct {
	IdCorrelacion   string
	Estado          string
	MarcaTemporalMs int64
}

// SumideroDeAcuses es la firma del sumidero en-proceso que recibe cada acuse clasificado.
type SumideroDeAcuses func(Acuse)

// clasificarAcuse filtra un *events.Receipt y devuelve el estado de envío correspondiente.
//
// Requiere AMBAS condiciones: r.Type en {ReceiptTypeDelivered, ReceiptTypeRead} Y
// len(r.MessageIDs) > 0. Todo otro caso —incluido un events.Receipt{} de valor cero, cuyo Type==""
// coincide con ReceiptTypeDelivered pero cuyo MessageIDs es nil— devuelve ("", false). La condición
// de MessageIDs es la guarda portante contra la trampa de la cadena vacía: comparar Type contra la
// constante nombrada basta para distinguir entregado de leído, pero no un acuse entregado intencional
// de un struct de valor cero.
func clasificarAcuse(r *events.Receipt) (string, bool) {
	if r == nil || len(r.MessageIDs) == 0 {
		return "", false
	}
	switch r.Type {
	case types.ReceiptTypeDelivered:
		return ipc.EstadoEnvioEntregado, true
	case types.ReceiptTypeRead:
		return ipc.EstadoEnvioLeido, true
	default:
		return "", false
	}
}

// manejarEventoDeAcuse es la lógica de despacho sin cliente: recibe un evento crudo, descarta
// cualquier tipo que no sea *events.Receipt, clasifica y, en caso de acierto, registra el suceso y
// entrega el Acuse al sumidero de forma nil-segura.
//
// Se factoriza fuera del cierre de AddEventHandler a propósito: el despacho interno de whatsmeow
// (Client.dispatchEvent) no se exporta, así que una prueba nunca puede disparar un manejador
// registrado desde fuera del paquete whatsmeow; solo puede llamar a esta función directamente.
func manejarEventoDeAcuse(evento any, sumidero SumideroDeAcuses, reg *registro.Registro) {
	recepcion, esAcuse := evento.(*events.Receipt)
	if !esAcuse {
		return
	}
	estado, ok := clasificarAcuse(recepcion)
	if !ok {
		return
	}
	if reg != nil {
		reg.Info(EventoAcuseClasificado, registro.Campos{
			Detalle: fmt.Sprintf("estado=%s", estado),
		})
	}
	if sumidero == nil {
		return
	}
	sumidero(Acuse{
		IdCorrelacion:   recepcion.MessageIDs[0],
		Estado:          estado,
		MarcaTemporalMs: recepcion.Timestamp.UnixMilli(),
	})
}

// RegistrarManejadorDeAcuses registra un manejador propio para los acuses de entrega y lectura y
// devuelve su identificador.
//
// Es un registro independiente del de RegistrarManejador (el enrutado por el supervisor): cada
// llamada a AddEventHandler añade un manejador sin alterar los ya registrados. Componer esta
// llamada en sidecar/main.go para que deje de ser código muerto es trabajo de HEX-072-b; hasta
// entonces el cableado es deliberadamente inalcanzable desde cualquier punto de entrada, igual que
// una función de biblioteca antes de que exista su primer llamador.
func (s *Sesion) RegistrarManejadorDeAcuses(sumidero SumideroDeAcuses) uint32 {
	return s.cliente.AddEventHandler(func(evento any) {
		manejarEventoDeAcuse(evento, sumidero, s.registro)
	})
}
