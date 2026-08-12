package identidad

import (
	"context"
	"fmt"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Presentación e identificación de bot [causa documentada]
//
// Nombres fijos de suceso del registro estructurado para la presentación de primer turno.
const (
	// EventoPresentacionReclamada se emite al ganar el reclamo único del mensaje de presentación.
	EventoPresentacionReclamada = "identidad.presentacion_reclamada"
)

// ReclamarPresentacion reserva de forma atómica el envío del único mensaje de presentación permitido
// en el primer turno de la conversación para un contacto.
// Devuelve reclamada=true solo para el primer llamador que gane la contienda (RowsAffected > 0).
// [causa documentada]
func (a *Almacen) ReclamarPresentacion(ctx context.Context, idInterno, idMensajePresentacion string, ahoraMs int64) (bool, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return false, ErrAlmacenCerrado
	}

	query := `INSERT INTO presentacion_de_conversacion (id_interno, id_mensaje_presentacion, presentacion_encolada_en_ms) VALUES (?, ?, ?) ON CONFLICT(id_interno) DO NOTHING`
	res, err := a.db.ExecContext(ctx, query, idInterno, idMensajePresentacion, ahoraMs)
	if err != nil {
		return false, fmt.Errorf("identidad: error al reclamar presentacion: %w", err)
	}

	filas, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("identidad: error al consultar filas afectadas al reclamar presentacion: %w", err)
	}

	reclamada := filas > 0
	if reclamada && a.registro != nil {
		a.registro.Info(EventoPresentacionReclamada, registro.Campos{
			IdEvento: idMensajePresentacion,
		})
	}
	return reclamada, nil
}
