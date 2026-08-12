package identidad

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Cortacircuitos conversacional [causa documentada]
//
// Nombres fijos de suceso del registro estructurado para el cortacircuitos.
const (
	// EventoCortacircuitosDisparado se emite al dispararse el cortacircuitos de un contacto.
	EventoCortacircuitosDisparado = "identidad.cortacircuitos_disparado"
	// EventoMensajeTraspasoReclamado se emite al ganar el reclamo único del mensaje de traspaso.
	EventoMensajeTraspasoReclamado = "identidad.mensaje_traspaso_reclamado"
	// EventoCortacircuitosRestablecido se emite al restablecer el cortacircuitos de un contacto.
	EventoCortacircuitosRestablecido = "identidad.cortacircuitos_restablecido"

	// MotivoDisparoRepeticion indica disparo por mensajes idénticos consecutivos.
	MotivoDisparoRepeticion = "repeticion"
	// MotivoDisparoFrustracion indica disparo por palabras de frustración o solicitud humana.
	MotivoDisparoFrustracion = "frustracion"
)

var (
	// ErrCortacircuitosNoDisparado se devuelve al reclamar el traspaso de un contacto sin cortacircuitos disparado.
	ErrCortacircuitosNoDisparado = errors.New("identidad: cortacircuitos no disparado para el contacto")
)

// RegistrarObservacion registra una observación de mensaje entrante para un contacto,
// actualizando el contador de repetición o disparando el cortacircuitos si se alcanza el umbral
// o se detecta frustración. Es idempotente una vez disparado: si el contacto ya estaba disparado,
// devuelve disparado=false (no fue este llamado el que lo disparó) y nil error.
// [causa documentada]
func (a *Almacen) RegistrarObservacion(ctx context.Context, idInterno, textoNormalizado string, esFrustracion bool, umbralRepeticion int64, ahoraMs int64) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	tx, err := a.db.BeginTx(ctx, nil)
	if err != nil {
		return false, fmt.Errorf("identidad: error al iniciar transaccion para registrar observacion: %w", err)
	}
	defer tx.Rollback()

	var repeticiones int64
	var ultimoTexto string
	var disparadoEnMs sql.NullInt64
	var motivoDisparo sql.NullString

	query := `SELECT repeticiones, ultimo_texto_normalizado, disparado_en_ms, motivo_disparo FROM cortacircuitos WHERE id_interno = ?`
	err = tx.QueryRowContext(ctx, query, idInterno).Scan(&repeticiones, &ultimoTexto, &disparadoEnMs, &motivoDisparo)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return false, fmt.Errorf("identidad: error al consultar cortacircuitos: %w", err)
	}

	// Caso 1: Ya estaba disparado previamente
	if err == nil && disparadoEnMs.Valid {
		if err := tx.Commit(); err != nil {
			return false, fmt.Errorf("identidad: error al confirmar transaccion: %w", err)
		}
		return false, nil
	}

	// Caso 2: No existía fila previa
	if errors.Is(err, sql.ErrNoRows) {
		dispara := esFrustracion || (umbralRepeticion <= 1)
		var dispMs sql.NullInt64
		var motDisp sql.NullString
		if dispara {
			dispMs = sql.NullInt64{Int64: ahoraMs, Valid: true}
			motivo := MotivoDisparoRepeticion
			if esFrustracion {
				motivo = MotivoDisparoFrustracion
			}
			motDisp = sql.NullString{String: motivo, Valid: true}
		}

		insertQuery := `INSERT INTO cortacircuitos (id_interno, repeticiones, ultimo_texto_normalizado, disparado_en_ms, motivo_disparo) VALUES (?, 1, ?, ?, ?)`
		if _, err := tx.ExecContext(ctx, insertQuery, idInterno, textoNormalizado, dispMs, motDisp); err != nil {
			return false, fmt.Errorf("identidad: error al insertar observacion de cortacircuitos: %w", err)
		}

		if err := tx.Commit(); err != nil {
			return false, fmt.Errorf("identidad: error al confirmar transaccion: %w", err)
		}

		if dispara && a.registro != nil {
			a.registro.Info(EventoCortacircuitosDisparado, registro.Campos{
				IdEvento: idInterno,
				Detalle:  "motivo=" + motDisp.String,
			})
		}
		return dispara, nil
	}

	// Caso 3: Fila existía pero aún no estaba disparada
	var nuevasRepeticiones int64
	if textoNormalizado == ultimoTexto {
		nuevasRepeticiones = repeticiones + 1
	} else {
		nuevasRepeticiones = 1
	}

	dispara := esFrustracion || (nuevasRepeticiones >= umbralRepeticion)
	var dispMs sql.NullInt64
	var motDisp sql.NullString
	if dispara {
		dispMs = sql.NullInt64{Int64: ahoraMs, Valid: true}
		motivo := MotivoDisparoRepeticion
		if esFrustracion {
			motivo = MotivoDisparoFrustracion
		}
		motDisp = sql.NullString{String: motivo, Valid: true}
	}

	updateQuery := `UPDATE cortacircuitos SET repeticiones = ?, ultimo_texto_normalizado = ?, disparado_en_ms = ?, motivo_disparo = ? WHERE id_interno = ? AND disparado_en_ms IS NULL`
	res, err := tx.ExecContext(ctx, updateQuery, nuevasRepeticiones, textoNormalizado, dispMs, motDisp, idInterno)
	if err != nil {
		return false, fmt.Errorf("identidad: error al actualizar observacion de cortacircuitos: %w", err)
	}
	filas, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("identidad: error al consultar filas afectadas: %w", err)
	}

	if err := tx.Commit(); err != nil {
		return false, fmt.Errorf("identidad: error al confirmar transaccion: %w", err)
	}

	disparadoEnEstaLlamada := dispara && filas > 0
	if disparadoEnEstaLlamada && a.registro != nil {
		a.registro.Info(EventoCortacircuitosDisparado, registro.Campos{
			IdEvento: idInterno,
			Detalle:  "motivo=" + motDisp.String,
		})
	}
	return disparadoEnEstaLlamada, nil
}

// SalidaPermitida es la autoridad del cortacircuitos que determina si un mensaje saliente puede enviarse al contacto.
// Sin disparo de cortacircuitos devuelve true; con cortacircuitos disparado devuelve true únicamente si idMensaje
// coincide con id_mensaje_traspaso asignado.
// [causa documentada]
func (a *Almacen) SalidaPermitida(ctx context.Context, idInterno, idMensaje string) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	query := `SELECT disparado_en_ms, id_mensaje_traspaso FROM cortacircuitos WHERE id_interno = ?`
	var disparadoEnMs sql.NullInt64
	var idTraspaso sql.NullString
	err := a.db.QueryRowContext(ctx, query, idInterno).Scan(&disparadoEnMs, &idTraspaso)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return true, nil
		}
		return false, fmt.Errorf("identidad: error al verificar permiso de salida en cortacircuitos: %w", err)
	}

	if !disparadoEnMs.Valid {
		return true, nil
	}

	if idTraspaso.Valid && idMensaje != "" && idTraspaso.String == idMensaje {
		return true, nil
	}

	return false, nil
}

// ReclamarMensajeDeTraspaso reserva de forma atómica el envío del único mensaje de traspaso a humano permitido.
// Devuelve reclamada=true solo para el primer llamador que gane la contienda.
// [causa documentada]
func (a *Almacen) ReclamarMensajeDeTraspaso(ctx context.Context, idInterno, idMensajeTraspaso string, ahoraMs int64) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	var disparadoEnMs sql.NullInt64
	var traspasoEncoladoEnMs sql.NullInt64
	err := a.db.QueryRowContext(ctx, `SELECT disparado_en_ms, traspaso_encolado_en_ms FROM cortacircuitos WHERE id_interno = ?`, idInterno).Scan(&disparadoEnMs, &traspasoEncoladoEnMs)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return false, ErrCortacircuitosNoDisparado
		}
		return false, fmt.Errorf("identidad: error al consultar estado de cortacircuitos: %w", err)
	}

	if !disparadoEnMs.Valid {
		return false, ErrCortacircuitosNoDisparado
	}

	if traspasoEncoladoEnMs.Valid {
		return false, nil
	}

	query := `UPDATE cortacircuitos SET id_mensaje_traspaso = ?, traspaso_encolado_en_ms = ? WHERE id_interno = ? AND disparado_en_ms IS NOT NULL AND traspaso_encolado_en_ms IS NULL`
	res, err := a.db.ExecContext(ctx, query, idMensajeTraspaso, ahoraMs, idInterno)
	if err != nil {
		return false, fmt.Errorf("identidad: error al reclamar traspaso: %w", err)
	}

	filas, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("identidad: error al consultar filas afectadas en reclamo de traspaso: %w", err)
	}

	reclamada := filas > 0
	if reclamada && a.registro != nil {
		a.registro.Info(EventoMensajeTraspasoReclamado, registro.Campos{
			IdEvento: idMensajeTraspaso,
		})
	}
	return reclamada, nil
}

// Restablecer elimina el estado del cortacircuitos para el contacto, restableciendo la comunicación normal.
// [causa documentada]
func (a *Almacen) Restablecer(ctx context.Context, idInterno string) error {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return ErrAlmacenCerrado
	}

	query := `DELETE FROM cortacircuitos WHERE id_interno = ?`
	_, err := a.db.ExecContext(ctx, query, idInterno)
	if err != nil {
		return fmt.Errorf("identidad: error al restablecer cortacircuitos: %w", err)
	}

	if a.registro != nil {
		a.registro.Info(EventoCortacircuitosRestablecido, registro.Campos{
			IdEvento: idInterno,
		})
	}
	return nil
}
