package outbox

import (
	"context"
	"errors"
	"fmt"
	"sync/atomic"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Cortacircuitos conversacional [causa documentada]
//
// Nombres fijos de suceso del registro estructurado, como en el resto del paquete.
const (
	// EventoEnvioBloqueadoPorCortacircuitos se emite al suprimir un mensaje en la admisión por cortacircuitos disparado.
	EventoEnvioBloqueadoPorCortacircuitos = "outbox.envio_bloqueado_por_cortacircuitos"
	// EventoErrorConsultaCortacircuitos se emite cuando la consulta del cortacircuitos falla (fallo cerrado).
	EventoErrorConsultaCortacircuitos = "outbox.error_consulta_cortacircuitos"
	// EventoEnvioBloqueadoPorBaja se emite al rechazar un mensaje en la admisión por baja.
	EventoEnvioBloqueadoPorBaja = "outbox.envio_bloqueado_por_baja"
	// EventoSalidaDescartadaPorBaja se emite al descartar en el drenaje una fila ya encolada.
	EventoSalidaDescartadaPorBaja = "outbox.salida_descartada_por_baja"
	// EventoErrorReclamoPresentacion se emite cuando el reclamo de la presentación falla (fallo cerrado).
	EventoErrorReclamoPresentacion = "outbox.error_reclamo_presentacion"
)

var (
	// ErrConversacionEnTraspaso se devuelve en la admisión cuando el cortacircuitos está disparado.
	ErrConversacionEnTraspaso = errors.New("outbox: conversacion en traspaso a humano por cortacircuitos")

	// ErrContactoDadoDeBaja se devuelve en la admisión cuando el destinatario está dado de baja.
	ErrContactoDadoDeBaja = errors.New("outbox: contacto dado de baja")

	// ContadorBloqueadasPorCortacircuitos cuenta mensajes rechazados en admisión por cortacircuitos disparado.
	ContadorBloqueadasPorCortacircuitos atomic.Int64

	// ContadorErroresCortacircuitos cuenta fallos al consultar cortacircuitos en admisión (fallo cerrado).
	ContadorErroresCortacircuitos atomic.Int64

	// ContadorBloqueadasPorBaja cuenta mensajes rechazados en el punto de admisión antes de encolar.
	ContadorBloqueadasPorBaja atomic.Int64

	// ContadorDescartadasPorBaja cuenta mensajes descartados con dureza en el drenaje por haber
	// recibido la baja mientras estaban encolados.
	ContadorDescartadasPorBaja atomic.Int64

	// ContadorErroresReclamoPresentacion cuenta fallos al reclamar la presentación (fallo cerrado).
	ContadorErroresReclamoPresentacion atomic.Int64
)

// ControlDeBaja define la autoridad de consulta y reclamo de baja para la ruta de salida.
// Satisfecho por identidad.Almacen sin acoplar outbox con el paquete identidad.
type ControlDeBaja interface {
	EnvioPermitido(ctx context.Context, idConversacion, idMensaje string) (bool, error)
	ReclamarConfirmacionDeBaja(ctx context.Context, idConversacion, idMensajeConfirmacion string, ahoraMs int64) (bool, error)
}

// ControlDeCortacircuitos define la autoridad de consulta y reclamo del cortacircuitos para la salida.
// Satisfecho por identidad.Almacen sin acoplar outbox con el paquete identidad.
// [causa documentada]
type ControlDeCortacircuitos interface {
	SalidaPermitida(ctx context.Context, idConversacion, idMensaje string) (bool, error)
	ReclamarMensajeDeTraspaso(ctx context.Context, idConversacion, idMensajeTraspaso string, ahoraMs int64) (bool, error)
}

// ControlDePresentacion define la autoridad de reclamo del mensaje de presentación para la salida.
// Satisfecho por identidad.Almacen sin acoplar outbox con el paquete identidad.
// [causa documentada]
type ControlDePresentacion interface {
	ReclamarPresentacion(ctx context.Context, idConversacion, idMensajePresentacion string, ahoraMs int64) (bool, error)
}

// PorteroDeSalida es el servicio de aplicación que custodia la entrada a la cola de salida,
// aplicando el cortacircuitos y el control de baja en el punto más temprano posible de la ruta de envío.
type PorteroDeSalida struct {
	cola           *ColaDeSalida
	controlBaja    ControlDeBaja
	cortacircuitos ControlDeCortacircuitos
	presentacion   ControlDePresentacion
	registro       *registro.Registro
}

// NuevoPorteroDeSalida construye el portero custodiando la cola de salida inyectada.
func NuevoPorteroDeSalida(cola *ColaDeSalida, controlBaja ControlDeBaja, cortacircuitos ControlDeCortacircuitos, presentacion ControlDePresentacion, reg *registro.Registro) *PorteroDeSalida {
	return &PorteroDeSalida{
		cola:           cola,
		controlBaja:    controlBaja,
		cortacircuitos: cortacircuitos,
		presentacion:   presentacion,
		registro:       reg,
	}
}

// Admitir verifica si el envío está permitido antes de encolar el mensaje.
// [causa documentada]
func (p *PorteroDeSalida) Admitir(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) error {
	// 1. Cortacircuitos primero (COSTURA de tarea 14 de A-3)
	if p.cortacircuitos != nil {
		permitido, err := p.cortacircuitos.SalidaPermitida(ctx, idConversacion, idMensaje)
		if err != nil {
			ContadorErroresCortacircuitos.Add(1)
			if p.registro != nil {
				p.registro.Error(EventoErrorConsultaCortacircuitos, registro.Campos{
					IdEvento: idMensaje,
					Detalle:  err.Error(),
				})
			}
			return fmt.Errorf("portero de salida: error al consultar cortacircuitos: %w", err)
		}
		if !permitido {
			ContadorBloqueadasPorCortacircuitos.Add(1)
			if p.registro != nil {
				p.registro.Aviso(EventoEnvioBloqueadoPorCortacircuitos, registro.Campos{
					IdEvento: idMensaje,
				})
			}
			return ErrConversacionEnTraspaso
		}
	}

	// 2. Control de baja (STOP) - intacto e inalterado
	if p.controlBaja != nil {
		permitido, err := p.controlBaja.EnvioPermitido(ctx, idConversacion, idMensaje)
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
	if p.controlBaja != nil {
		reclamada, err := p.controlBaja.ReclamarConfirmacionDeBaja(ctx, idConversacion, idMensaje, marcaTemporalOrigenMs)
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

// AdmitirMensajeDeTraspaso es la vía de admisión del único mensaje de traspaso a humano permitido
// tras dispararse el cortacircuitos: reclama atómicamente el traspaso y, sólo si gana el reclamo,
// lo admite a través de Admitir para respetar la precedencia de baja/STOP sobre el cortacircuitos.
// [causa documentada]
func (p *PorteroDeSalida) AdmitirMensajeDeTraspaso(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) (bool, error) {
	if p.cortacircuitos != nil {
		reclamada, err := p.cortacircuitos.ReclamarMensajeDeTraspaso(ctx, idConversacion, idMensaje, marcaTemporalOrigenMs)
		if err != nil {
			return false, fmt.Errorf("portero de salida: error al reclamar traspaso: %w", err)
		}
		if !reclamada {
			return false, nil
		}
	}

	if err := p.Admitir(ctx, idMensaje, idConversacion, contenido, marcaTemporalOrigenMs); err != nil {
		return false, err
	}
	return true, nil
}

// AdmitirMensajeDePresentacion es la vía de admisión del único mensaje de presentación permitido
// en el primer turno de la conversación: reclama atómicamente la presentación y, sólo si gana el reclamo,
// lo admite a través de Admitir para respetar la precedencia de baja/STOP y cortacircuitos sobre la presentación.
// [causa documentada]
func (p *PorteroDeSalida) AdmitirMensajeDePresentacion(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) (bool, error) {
	if p.presentacion != nil {
		reclamada, err := p.presentacion.ReclamarPresentacion(ctx, idConversacion, idMensaje, marcaTemporalOrigenMs)
		if err != nil {
			ContadorErroresReclamoPresentacion.Add(1)
			if p.registro != nil {
				p.registro.Error(EventoErrorReclamoPresentacion, registro.Campos{
					IdEvento: idMensaje,
					Detalle:  err.Error(),
				})
			}
			return false, fmt.Errorf("portero de salida: error al reclamar presentacion: %w", err)
		}
		if !reclamada {
			return false, nil
		}
	}

	if err := p.Admitir(ctx, idMensaje, idConversacion, contenido, marcaTemporalOrigenMs); err != nil {
		return false, err
	}
	return true, nil
}
