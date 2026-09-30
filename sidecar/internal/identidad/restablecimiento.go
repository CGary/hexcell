package identidad

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
)

// ErrIdInternoInvalido señala un id_interno que no tiene la forma «ct-» + 32 hexadecimales en
// minúscula. Se devuelve antes de tocar SQL.
var ErrIdInternoInvalido = errors.New("id interno invalido")

// ResultadoDeRestablecimiento es el informe de un restablecimiento de contacto. Existe es el
// discriminante explícito de si el contacto figura en `identidad`; nunca se infiere de los
// contadores. Los contadores son las filas borradas por tabla.
type ResultadoDeRestablecimiento struct {
	Existe                     bool
	BajaIncluida               bool
	Cortacircuitos             int64
	PresentacionDeConversacion int64
	BajaDeContacto             int64
}

// EsIdInternoValido comprueba la forma del id_interno comparando bytes del sufijo ASCII, sin
// cortes por índice que puedan entrar en pánico con entrada multibyte.
func EsIdInternoValido(id string) bool {
	if len(id) != len(PrefijoIdentidad)+32 || id[:len(PrefijoIdentidad)] != PrefijoIdentidad {
		return false
	}
	for _, b := range []byte(id[len(PrefijoIdentidad):]) {
		if !(b >= '0' && b <= '9') && !(b >= 'a' && b <= 'f') {
			return false
		}
	}
	return true
}

// RestablecerContacto borra las filas del contacto en cortacircuitos y presentacion_de_conversacion
// (y en baja_de_contacto solo con incluirBaja) dentro de UNA transacción, comprobando antes la
// existencia en `identidad` dentro de la misma transacción. Un id ausente devuelve Existe=false
// sin borrar nada; cualquier error revierte todos los DELETE. Nunca toca `identidad` ni `direccion`.
func (a *Almacen) RestablecerContacto(ctx context.Context, idInterno string, incluirBaja bool) (ResultadoDeRestablecimiento, error) {
	if !EsIdInternoValido(idInterno) {
		return ResultadoDeRestablecimiento{}, ErrIdInternoInvalido
	}
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return ResultadoDeRestablecimiento{}, ErrAlmacenCerrado
	}
	tx, err := a.db.BeginTx(ctx, nil)
	if err != nil {
		return ResultadoDeRestablecimiento{}, fmt.Errorf("identidad: iniciar transaccion: %w", err)
	}
	defer tx.Rollback()
	var existe int
	if err := tx.QueryRowContext(ctx, `SELECT 1 FROM identidad WHERE id_interno = ?`, idInterno).Scan(&existe); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			if err := tx.Commit(); err != nil {
				return ResultadoDeRestablecimiento{}, err
			}
			return ResultadoDeRestablecimiento{BajaIncluida: incluirBaja}, nil
		}
		return ResultadoDeRestablecimiento{}, err
	}
	r := ResultadoDeRestablecimiento{Existe: existe == 1, BajaIncluida: incluirBaja}
	for _, op := range []struct {
		q   string
		out *int64
	}{
		{`DELETE FROM cortacircuitos WHERE id_interno = ?`, &r.Cortacircuitos},
		{`DELETE FROM presentacion_de_conversacion WHERE id_interno = ?`, &r.PresentacionDeConversacion},
	} {
		res, err := tx.ExecContext(ctx, op.q, idInterno)
		if err != nil {
			return ResultadoDeRestablecimiento{}, err
		}
		*op.out, err = res.RowsAffected()
		if err != nil {
			return ResultadoDeRestablecimiento{}, err
		}
	}
	if incluirBaja {
		res, err := tx.ExecContext(ctx, `DELETE FROM baja_de_contacto WHERE id_interno = ?`, idInterno)
		if err != nil {
			return ResultadoDeRestablecimiento{}, err
		}
		r.BajaDeContacto, err = res.RowsAffected()
		if err != nil {
			return ResultadoDeRestablecimiento{}, err
		}
	}
	if err := tx.Commit(); err != nil {
		return ResultadoDeRestablecimiento{}, err
	}
	return r, nil
}
