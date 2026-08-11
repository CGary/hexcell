package identidad

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Nombres fijos de suceso del registro estructurado, como en el resto del paquete.
const (
	// EventoBajaRegistrada se emite al crear la fila de baja de un contacto.
	EventoBajaRegistrada = "identidad.baja_registrada"
	// EventoConfirmacionDeBajaReclamada se emite al ganar el reclamo único de la confirmación.
	EventoConfirmacionDeBajaReclamada = "identidad.confirmacion_baja_reclamada"
)

var (
	// ErrBajaNoRegistrada se devuelve al reclamar la confirmación de un contacto sin fila de baja.
	ErrBajaNoRegistrada = errors.New("identidad: contacto no registrado como dado de baja")
)

// RegistrarBaja inserta de forma idempotente el registro de baja para el contacto.
// Devuelve creada=true únicamente si esta llamada creó la fila (RowsAffected > 0).
func (a *Almacen) RegistrarBaja(ctx context.Context, idInterno string, dadaDeBajaEnMs int64) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	query := `INSERT INTO baja_de_contacto (id_interno, dada_de_baja_en_ms) VALUES (?, ?) ON CONFLICT(id_interno) DO NOTHING`
	res, err := a.db.ExecContext(ctx, query, idInterno, dadaDeBajaEnMs)
	if err != nil {
		return false, fmt.Errorf("identidad: error al registrar baja: %w", err)
	}

	filas, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("identidad: error al consultar filas afectadas al registrar baja: %w", err)
	}

	creada := filas > 0
	if creada && a.registro != nil {
		a.registro.Info(EventoBajaRegistrada, registro.Campos{
			IdEvento: idInterno,
		})
	}
	return creada, nil
}

// EnvioPermitido es la autoridad única que determina si un mensaje saliente puede enviarse al contacto.
// Sin registro de baja devuelve true; con registro de baja devuelve true únicamente si idMensaje
// coincide con id_mensaje_confirmacion asignado.
func (a *Almacen) EnvioPermitido(ctx context.Context, idInterno, idMensaje string) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	query := `SELECT id_mensaje_confirmacion FROM baja_de_contacto WHERE id_interno = ?`
	var idConfirmacion sql.NullString
	err := a.db.QueryRowContext(ctx, query, idInterno).Scan(&idConfirmacion)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return true, nil
		}
		return false, fmt.Errorf("identidad: error al verificar permiso de envio: %w", err)
	}

	if idConfirmacion.Valid && idMensaje != "" && idConfirmacion.String == idMensaje {
		return true, nil
	}

	return false, nil
}

// ReclamarConfirmacionDeBaja reserva de forma atómica el envío del único mensaje de confirmación permitido.
// Devuelve reclamada=true solo para el primer llamador que gane la contienda.
func (a *Almacen) ReclamarConfirmacionDeBaja(ctx context.Context, idInterno, idMensajeConfirmacion string, ahoraMs int64) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	var confirmadaEn sql.NullInt64
	err := a.db.QueryRowContext(ctx, `SELECT confirmacion_encolada_en_ms FROM baja_de_contacto WHERE id_interno = ?`, idInterno).Scan(&confirmadaEn)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return false, ErrBajaNoRegistrada
		}
		return false, fmt.Errorf("identidad: error al consultar estado de baja: %w", err)
	}

	if confirmadaEn.Valid {
		return false, nil
	}

	query := `UPDATE baja_de_contacto SET id_mensaje_confirmacion = ?, confirmacion_encolada_en_ms = ? WHERE id_interno = ? AND confirmacion_encolada_en_ms IS NULL`
	res, err := a.db.ExecContext(ctx, query, idMensajeConfirmacion, ahoraMs, idInterno)
	if err != nil {
		return false, fmt.Errorf("identidad: error al reclamar confirmacion: %w", err)
	}

	filas, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("identidad: error al consultar filas afectadas en reclamo: %w", err)
	}

	reclamada := filas > 0
	if reclamada && a.registro != nil {
		a.registro.Info(EventoConfirmacionDeBajaReclamada, registro.Campos{
			IdEvento: idMensajeConfirmacion,
		})
	}
	return reclamada, nil
}
