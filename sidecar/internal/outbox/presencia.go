package outbox

import (
	"context"
	"fmt"

	"go.mau.fi/whatsmeow/types"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// ClienteDePresencia es el subconjunto de *whatsmeow.Client que EmisorDePresenciaWhatsmeow necesita.
// Se declara como interfaz propia para permitir sustitución por dobles en tests sin levantar
// una sesión real de whatsmeow.
type ClienteDePresencia interface {
	SendChatPresence(ctx context.Context, jid types.JID, state types.ChatPresence, media types.ChatPresenceMedia) error
}

// EmisorDePresencia define el contrato para emitir el indicador de presencia ("escribiendo") hacia el canal.
//
// [precautorio] Emitir el indicador de "escribiendo" es higiene documentada de coste cero, no una defensa.
// Limitaciones documentadas en adr-0015:
//  1. La entrega hacia el destinatario es inobservable desde el sidecar.
//  2. Un retorno exitoso solo prueba que el nodo de transporte fue emitido sin fallo de conexión.
//  3. La presencia de chat depende de que el cliente posea un nombre público (push name) y disponibilidad anunciada.
type EmisorDePresencia interface {
	EmitirEscribiendo(ctx context.Context, idConversacion string) error
}

// EmisorDePresenciaWhatsmeow implementa EmisorDePresencia sobre whatsmeow.
// Reside dentro del paquete internal/outbox para que el centinela de rutas de envío no requiera
// exenciones de directorio adicionales.
type EmisorDePresenciaWhatsmeow struct {
	cliente   ClienteDePresencia
	resolutor ResolutorDeDirecciones
	registro  *registro.Registro
}

// NuevoEmisorDePresenciaWhatsmeow construye el emisor de presencia a partir del cliente de whatsmeow,
// el resolutor de direcciones (almacén de identidad) y el registrador estructurado.
func NuevoEmisorDePresenciaWhatsmeow(cliente ClienteDePresencia, resolutor ResolutorDeDirecciones, reg *registro.Registro) *EmisorDePresenciaWhatsmeow {
	return &EmisorDePresenciaWhatsmeow{
		cliente:   cliente,
		resolutor: resolutor,
		registro:  reg,
	}
}

// EmitirEscribiendo resuelve la dirección JID del contacto y emite el estado ChatPresenceComposing.
// Un fallo se registra a nivel aviso y devuelve el error para trazabilidad; el llamador nunca
// aborta el mensaje ni incrementa reintentos ante un fallo de presencia.
func (e *EmisorDePresenciaWhatsmeow) EmitirEscribiendo(ctx context.Context, idConversacion string) error {
	if e.cliente == nil || e.resolutor == nil {
		return nil
	}

	destino, err := e.resolutor.DireccionDe(ctx, idConversacion)
	if err != nil {
		return fmt.Errorf("emisor de presencia: no se pudo resolver la dirección de la conversación: %w", err)
	}

	if err := e.cliente.SendChatPresence(ctx, destino, types.ChatPresenceComposing, types.ChatPresenceMediaText); err != nil {
		if e.registro != nil {
			e.registro.Aviso("outbox.error_presencia", registro.Campos{
				IdEvento: idConversacion,
				Detalle:  err.Error(),
			})
		}
		return fmt.Errorf("emisor de presencia: fallo al emitir presencia: %w", err)
	}

	return nil
}
