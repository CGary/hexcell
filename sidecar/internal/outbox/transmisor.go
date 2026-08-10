package outbox

import (
	"context"
	"fmt"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
)

// Transmisor define el contrato para enviar mensajes hacia el canal subyacente.
//
// Al residir dentro del paquete outbox, las llamadas a sus implementaciones (o llamadas
// pasadas a través de él) pueden ocurrir sin ser bloqueadas por el centinela, asegurando
// que todo mensaje saliente pase por la ColaDeSalida.
type Transmisor interface {
	// Transmitir envía un mensaje al canal y devuelve el identificador
	// de correlación asignado por el transporte subyacente (ej. SendResponse.ID).
	// Nunca devuelve o maneja un Sender JID.
	Transmitir(ctx context.Context, idConversacion, contenido string) (idCorrelacion string, err error)
}

// ClienteWhatsmeow es el subconjunto de *whatsmeow.Client que TransmisorWhatsmeow necesita.
// Se declara como interfaz propia, en vez de referenciar el tipo concreto en la firma, para
// que los tests sustituyan el cliente real por un doble sin levantar una sesión de whatsmeow.
type ClienteWhatsmeow interface {
	SendMessage(ctx context.Context, to types.JID, message *waE2E.Message, extra ...whatsmeow.SendRequestExtra) (whatsmeow.SendResponse, error)
}

// ResolutorDeDirecciones traduce el id_conversacion opaco del protocolo IPC (el identificador
// interno de identidad.Almacen) a la dirección JID que el transporte necesita para enviar.
// *identidad.Almacen ya satisface esta interfaz con su método DireccionDe: la resolución
// inversa de identidad no es responsabilidad nueva de esta tarea, solo se reutiliza aquí.
type ResolutorDeDirecciones interface {
	DireccionDe(ctx context.Context, idInterno string) (types.JID, error)
}

// TransmisorWhatsmeow es la implementación de Transmisor respaldada por whatsmeow: el único
// lugar del sidecar donde un mensaje saliente cruza de verdad hacia el canal de WhatsApp.
// Vive dentro del paquete outbox a propósito, para que el centinela de rutas de envío exima
// esta llamada real sin tener que eximir ningún otro directorio.
type TransmisorWhatsmeow struct {
	cliente   ClienteWhatsmeow
	resolutor ResolutorDeDirecciones
}

// NuevoTransmisorWhatsmeow construye el transmisor real a partir del cliente whatsmeow de la
// sesión activa (Sesion.Cliente()) y del resolutor de direcciones (el almacén de identidad).
func NuevoTransmisorWhatsmeow(cliente ClienteWhatsmeow, resolutor ResolutorDeDirecciones) *TransmisorWhatsmeow {
	return &TransmisorWhatsmeow{cliente: cliente, resolutor: resolutor}
}

// Transmitir resuelve la dirección JID del destinatario y envía el contenido como mensaje de
// texto libre (el canal propio no admite plantillas). Devuelve únicamente el identificador de
// correlación asignado por el transporte (SendResponse.ID); el Sender JID de la respuesta
// nunca se copia ni se propaga fuera de esta función.
func (t *TransmisorWhatsmeow) Transmitir(ctx context.Context, idConversacion, contenido string) (string, error) {
	destino, err := t.resolutor.DireccionDe(ctx, idConversacion)
	if err != nil {
		return "", fmt.Errorf("transmisor whatsmeow: no se pudo resolver la dirección de la conversación: %w", err)
	}

	texto := contenido
	respuesta, err := t.cliente.SendMessage(ctx, destino, &waE2E.Message{Conversation: &texto})
	if err != nil {
		return "", fmt.Errorf("transmisor whatsmeow: error al enviar: %w", err)
	}

	return respuesta.ID, nil
}
