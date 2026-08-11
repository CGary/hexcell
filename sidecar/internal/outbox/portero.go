package outbox

import (
	"context"
	"errors"
	"fmt"
	"sync/atomic"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Nombres fijos de suceso del registro estructurado, como en el resto del paquete.
const (
	// EventoEnvioBloqueadoPorBaja se emite al rechazar un mensaje en la admisión por baja.
	EventoEnvioBloqueadoPorBaja = "outbox.envio_bloqueado_por_baja"
	// EventoSalidaDescartadaPorBaja se emite al descartar en el drenaje una fila ya encolada.
	EventoSalidaDescartadaPorBaja = "outbox.salida_descartada_por_baja"
)

var (
	// ErrContactoDadoDeBaja se devuelve en la admisión cuando el destinatario está dado de baja.
	ErrContactoDadoDeBaja = errors.New("outbox: contacto dado de baja")

	// ContadorBloqueadasPorBaja cuenta mensajes rechazados en el punto de admisión antes de encolar.
	ContadorBloqueadasPorBaja atomic.Int64

	// ContadorDescartadasPorBaja cuenta mensajes descartados con dureza en el drenaje por haber
	// recibido la baja mientras estaban encolados.
	ContadorDescartadasPorBaja atomic.Int64
)

// ControlDeBaja define la autoridad de consulta y reclamo de baja para la ruta de salida.
// Satisfecho por identidad.Almacen sin acoplar outbox con el paquete identidad.
type ControlDeBaja interface {
	EnvioPermitido(ctx context.Context, idConversacion, idMensaje string) (bool, error)
	ReclamarConfirmacionDeBaja(ctx context.Context, idConversacion, idMensajeConfirmacion string, ahoraMs int64) (bool, error)
}

// PorteroDeSalida es el servicio de aplicación que custodia la entrada a la cola de salida,
// aplicando el control de baja en el punto más temprano posible de la ruta de envío.
type PorteroDeSalida struct {
	cola     *ColaDeSalida
	control  ControlDeBaja
	registro *registro.Registro
}

// NuevoPorteroDeSalida construye el portero custodiando la cola de salida inyectada.
func NuevoPorteroDeSalida(cola *ColaDeSalida, control ControlDeBaja, reg *registro.Registro) *PorteroDeSalida {
	return &PorteroDeSalida{
		cola:     cola,
		control:  control,
		registro: reg,
	}
}

// Admitir verifica si el envío está permitido antes de encolar el mensaje.
func (p *PorteroDeSalida) Admitir(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) error {
	// COSTURA: aquí se insertará el futuro cortacircuitos (circuit breaker, tarea 14 de A-3)
	// antes de la comprobación de baja, sin necesidad de reubicar esta última.

	if p.control != nil {
		permitido, err := p.control.EnvioPermitido(ctx, idConversacion, idMensaje)
		if err != nil {
			return fmt.Errorf("portero de salida: error al consultar control de baja: %w", err)
		}
		if !permitido {
			ContadorBloqueadasPorBaja.Add(1)
			if p.registro != nil {
				p.registro.Aviso(EventoEnvioBloqueadoPorBaja, registro.Campos{
					IdEvento: idMensaje,
				})
			}
			return ErrContactoDadoDeBaja
		}
	}

	return p.cola.Encolar(ctx, idMensaje, idConversacion, contenido, marcaTemporalOrigenMs)
}

// AdmitirConfirmacionDeBaja es la única exención permitida para un contacto dado de baja:
// reclama atómicamente la confirmación y sólo si gana el reclamo procede a encolarla.
func (p *PorteroDeSalida) AdmitirConfirmacionDeBaja(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) (bool, error) {
	if p.control != nil {
		reclamada, err := p.control.ReclamarConfirmacionDeBaja(ctx, idConversacion, idMensaje, marcaTemporalOrigenMs)
		if err != nil {
			return false, fmt.Errorf("portero de salida: error al reclamar confirmacion: %w", err)
		}
		if !reclamada {
			return false, nil
		}
	}

	if err := p.cola.Encolar(ctx, idMensaje, idConversacion, contenido, marcaTemporalOrigenMs); err != nil {
		return false, err
	}
	return true, nil
}
