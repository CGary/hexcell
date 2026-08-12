// Package identidad implementa el almacén de identidades para el sidecar HexCell.
// Esta es la cuarta base del respaldo A-2.
//
// Razón del esquema: consta de cuatro tablas (identidad, direccion, baja_de_contacto, cortacircuitos), ancladas en PN (Phone Number)
// con LID como alias. Esta separación de sqlstore existe para que las identidades
// y los registros de baja sobrevivan a eventos LoggedOut o dispositivos removidos (device_removed).
// El script de respaldo debe ser capaz de leer este esquema desde el código.
package identidad

import (
	"context"
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/registro"
	"go.mau.fi/whatsmeow/types"
	_ "modernc.org/sqlite"
)

const (
	RutaPorOmision   = "/var/lib/hexcell/identidad.db"
	PrefijoIdentidad = "ct-"

	EstadoProvisional = "provisional"
	EstadoAnclada     = "anclada"
	EstadoFusionada   = "fusionada"

	DireccionPN  = "pn"
	DireccionLID = "lid"

	EventoIdentidadCreada    = "identidad_creada"
	EventoIdentidadAnclada   = "identidad_anclada"
	EventoIdentidadFusionada = "identidad_fusionada"
	EventoConflictoDeAlias   = "conflicto_de_alias"
)

var (
	ErrAlmacenCerrado   = errors.New("almacen cerrado")
	ErrObservacionVacia = errors.New("observacion vacia")
)

type Observacion struct {
	PN  types.JID
	LID types.JID
}

type Identidad struct {
	IdInterno        string
	Estado           string
	ConflictoDeAlias bool
}

type Opciones struct {
	Ruta     string
	Registro *registro.Registro
}

type Almacen struct {
	db       *sql.DB
	registro *registro.Registro
	mu       sync.RWMutex
	cerrado  bool
}

const esquema = `
CREATE TABLE IF NOT EXISTS identidad (
  id_interno TEXT PRIMARY KEY,
  estado TEXT NOT NULL CHECK (estado IN ('provisional','anclada','fusionada')),
  fusionada_en TEXT NULL REFERENCES identidad(id_interno) ON DELETE RESTRICT,
  creada_en_ms INTEGER NOT NULL,
  actualizada_en_ms INTEGER NOT NULL,
  CHECK ((estado = 'fusionada') = (fusionada_en IS NOT NULL))
);
CREATE TABLE IF NOT EXISTS direccion (
  tipo TEXT NOT NULL CHECK (tipo IN ('pn','lid')),
  usuario TEXT NOT NULL,
  servidor TEXT NOT NULL,
  id_interno TEXT NOT NULL REFERENCES identidad(id_interno) ON DELETE CASCADE,
  observada_en_ms INTEGER NOT NULL,
  PRIMARY KEY (tipo, usuario, servidor)
);
CREATE TABLE IF NOT EXISTS baja_de_contacto (
  id_interno TEXT PRIMARY KEY REFERENCES identidad(id_interno) ON DELETE CASCADE,
  dada_de_baja_en_ms INTEGER NOT NULL,
  id_mensaje_confirmacion TEXT NULL UNIQUE,
  confirmacion_encolada_en_ms INTEGER NULL
);
CREATE TABLE IF NOT EXISTS cortacircuitos (
  id_interno TEXT PRIMARY KEY REFERENCES identidad(id_interno) ON DELETE CASCADE,
  repeticiones INTEGER NOT NULL,
  ultimo_texto_normalizado TEXT NOT NULL,
  disparado_en_ms INTEGER NULL,
  motivo_disparo TEXT NULL,
  id_mensaje_traspaso TEXT NULL UNIQUE,
  traspaso_encolado_en_ms INTEGER NULL
);
CREATE INDEX IF NOT EXISTS idx_direccion_identidad ON direccion(id_interno);
CREATE INDEX IF NOT EXISTS idx_identidad_fusionada ON identidad(fusionada_en);
`

func construirDSN(ruta string) string {
	return fmt.Sprintf("file:%s?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)&_pragma=busy_timeout(5000)", ruta)
}

func generarIdInterno() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return PrefijoIdentidad + hex.EncodeToString(b), nil
}

func Abrir(opc Opciones) (*Almacen, error) {
	dsn := construirDSN(opc.Ruta)
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("error al abrir bd: %w", err)
	}

	db.SetMaxOpenConns(1)

	if _, err := db.Exec(esquema); err != nil {
		db.Close()
		return nil, fmt.Errorf("error al aplicar esquema: %w", err)
	}

	return &Almacen{
		db:       db,
		registro: opc.Registro,
	}, nil
}

func (a *Almacen) Cerrar() error {
	a.mu.Lock()
	defer a.mu.Unlock()

	if a.cerrado {
		return nil
	}
	a.cerrado = true
	return a.db.Close()
}

func buscarPorDireccion(tx *sql.Tx, tipo, usuario, servidor string) (string, int64, error) {
	var idInterno string
	var estado string
	var fusionadaEn sql.NullString
	var creadaEnMs int64

	q := `
		SELECT i.id_interno, i.estado, i.fusionada_en, i.creada_en_ms
		FROM direccion d
		JOIN identidad i ON d.id_interno = i.id_interno
		WHERE d.tipo = ? AND d.usuario = ? AND d.servidor = ?
	`
	err := tx.QueryRow(q, tipo, usuario, servidor).Scan(&idInterno, &estado, &fusionadaEn, &creadaEnMs)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return "", 0, nil // No existe
		}
		return "", 0, err
	}

	if estado == EstadoFusionada && fusionadaEn.Valid {
		// Seguir fusionada_en un salto
		q2 := `SELECT creada_en_ms FROM identidad WHERE id_interno = ?`
		err = tx.QueryRow(q2, fusionadaEn.String).Scan(&creadaEnMs)
		if err != nil {
			return "", 0, err
		}
		return fusionadaEn.String, creadaEnMs, nil
	}

	return idInterno, creadaEnMs, nil
}

func tieneDireccionPN(tx *sql.Tx, idInterno string) (bool, error) {
	var c int
	err := tx.QueryRow(`SELECT count(*) FROM direccion WHERE id_interno = ? AND tipo = ?`, idInterno, DireccionPN).Scan(&c)
	return c > 0, err
}

func obtenerPNDe(tx *sql.Tx, idInterno string) (string, string, error) {
	var u, s string
	err := tx.QueryRow(`SELECT usuario, servidor FROM direccion WHERE id_interno = ? AND tipo = ? LIMIT 1`, idInterno, DireccionPN).Scan(&u, &s)
	if errors.Is(err, sql.ErrNoRows) {
		return "", "", nil
	}
	return u, s, err
}

func (a *Almacen) Resolver(ctx context.Context, obs Observacion) (Identidad, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return Identidad{}, ErrAlmacenCerrado
	}

	var conPN, conLID bool
	var pn, lid types.JID

	if !obs.PN.IsEmpty() {
		conPN = true
		pn = obs.PN.ToNonAD()
	}
	if !obs.LID.IsEmpty() {
		conLID = true
		lid = obs.LID.ToNonAD()
	}

	if !conPN && !conLID {
		return Identidad{}, ErrObservacionVacia
	}

	tx, err := a.db.BeginTx(ctx, nil)
	if err != nil {
		return Identidad{}, err
	}
	defer tx.Rollback()

	ahora := time.Now().UnixMilli()

	var idPn, idLid string
	var creadaPn, creadaLid int64

	if conPN {
		idPn, creadaPn, err = buscarPorDireccion(tx, DireccionPN, pn.User, pn.Server)
		if err != nil {
			return Identidad{}, err
		}
	}
	if conLID {
		idLid, creadaLid, err = buscarPorDireccion(tx, DireccionLID, lid.User, lid.Server)
		if err != nil {
			return Identidad{}, err
		}
	}

	// Caso 1: Ninguna resuelve
	if idPn == "" && idLid == "" {
		idNuevo, err := generarIdInterno()
		if err != nil {
			return Identidad{}, err
		}
		estado := EstadoProvisional
		if conPN {
			estado = EstadoAnclada
		}

		_, err = tx.ExecContext(ctx, `INSERT INTO identidad (id_interno, estado, creada_en_ms, actualizada_en_ms) VALUES (?, ?, ?, ?)`, idNuevo, estado, ahora, ahora)
		if err != nil {
			return Identidad{}, err
		}

		if conPN {
			_, err = tx.ExecContext(ctx, `INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING`, DireccionPN, pn.User, pn.Server, idNuevo, ahora)
			if err != nil {
				return Identidad{}, err
			}
		}
		if conLID {
			_, err = tx.ExecContext(ctx, `INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING`, DireccionLID, lid.User, lid.Server, idNuevo, ahora)
			if err != nil {
				return Identidad{}, err
			}
		}

		if a.registro != nil {
			a.registro.Info(EventoIdentidadCreada, registro.Campos{Detalle: "estado=" + estado})
		}

		// re-select y adoptar por si concurrencia
		if conPN {
			idPn, _, err = buscarPorDireccion(tx, DireccionPN, pn.User, pn.Server)
		} else {
			idLid, _, err = buscarPorDireccion(tx, DireccionLID, lid.User, lid.Server)
		}
		if err != nil {
			return Identidad{}, err
		}
		idFinal := idPn
		if idFinal == "" {
			idFinal = idLid
		}

		var estadoFinal string
		if err := tx.QueryRowContext(ctx, `SELECT estado FROM identidad WHERE id_interno = ?`, idFinal).Scan(&estadoFinal); err != nil {
			return Identidad{}, err
		}

		if err := tx.Commit(); err != nil {
			return Identidad{}, err
		}
		return Identidad{IdInterno: idFinal, Estado: estadoFinal}, nil
	}

	// Caso 2: Sólo una resuelve
	if idPn != "" && idLid == "" || idLid != "" && idPn == "" {
		resueltoId := idPn
		if resueltoId == "" {
			resueltoId = idLid
		}

		// Añadir dirección faltante
		if conPN && idPn == "" {
			uPNexistente, sPNexistente, err := obtenerPNDe(tx, resueltoId)
			if err != nil {
				return Identidad{}, err
			}
			if uPNexistente != "" && (uPNexistente != pn.User || sPNexistente != pn.Server) {
				idNuevo, err := generarIdInterno()
				if err != nil {
					return Identidad{}, err
				}
				if _, err = tx.ExecContext(ctx, `INSERT INTO identidad (id_interno, estado, creada_en_ms, actualizada_en_ms) VALUES (?, ?, ?, ?)`, idNuevo, EstadoAnclada, ahora, ahora); err != nil {
					return Identidad{}, err
				}
				if _, err = tx.ExecContext(ctx, `INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING`, DireccionPN, pn.User, pn.Server, idNuevo, ahora); err != nil {
					return Identidad{}, err
				}
				if a.registro != nil {
					a.registro.Aviso(EventoConflictoDeAlias, registro.Campos{})
				}
				if err := tx.Commit(); err != nil {
					return Identidad{}, err
				}
				return Identidad{IdInterno: idNuevo, Estado: EstadoAnclada, ConflictoDeAlias: true}, nil
			}
			_, err = tx.ExecContext(ctx, `INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING`, DireccionPN, pn.User, pn.Server, resueltoId, ahora)
			if err != nil {
				return Identidad{}, err
			}
			// Actualizar estado a anclada
			_, err = tx.ExecContext(ctx, `UPDATE identidad SET estado = ?, actualizada_en_ms = ? WHERE id_interno = ? AND estado = ?`, EstadoAnclada, ahora, resueltoId, EstadoProvisional)
			if err != nil {
				return Identidad{}, err
			}
			if a.registro != nil {
				a.registro.Info(EventoIdentidadAnclada, registro.Campos{})
			}
		} else if conLID && idLid == "" {
			// El PN es el ancla y rechaza un segundo valor distinto (arriba); el LID
			// es un alias que puede cambiar al re-registrarse el contacto, por eso no
			// hay aquí un rechazo simétrico al de PN.
			_, err = tx.ExecContext(ctx, `INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING`, DireccionLID, lid.User, lid.Server, resueltoId, ahora)
			if err != nil {
				return Identidad{}, err
			}
		}

		var est string
		if err := tx.QueryRowContext(ctx, `SELECT estado FROM identidad WHERE id_interno = ?`, resueltoId).Scan(&est); err != nil {
			return Identidad{}, err
		}
		if err := tx.Commit(); err != nil {
			return Identidad{}, err
		}
		return Identidad{IdInterno: resueltoId, Estado: est}, nil
	}

	// Caso 3: Ambas resuelven a la misma identidad
	if idPn == idLid {
		if err := tx.Commit(); err != nil {
			return Identidad{}, err
		}
		return Identidad{IdInterno: idPn, Estado: EstadoAnclada}, nil // Debe estar anclada si tiene ambas
	}

	// Caso 4: Resuelven a distintas identidades (FUSIÓN)
	superviviente := idPn
	absorbida := idLid

	masAntiguo := creadaLid < creadaPn
	if creadaLid == creadaPn {
		// 02-contract.yaml línea 102: rowid es el orden real de inserción; un desempate
		// lexicográfico por id_interno (crypto/rand) sería arbitrario.
		var rowidLid, rowidPn int64
		if err := tx.QueryRowContext(ctx, `SELECT rowid FROM identidad WHERE id_interno = ?`, idLid).Scan(&rowidLid); err != nil {
			return Identidad{}, err
		}
		if err := tx.QueryRowContext(ctx, `SELECT rowid FROM identidad WHERE id_interno = ?`, idPn).Scan(&rowidPn); err != nil {
			return Identidad{}, err
		}
		masAntiguo = rowidLid < rowidPn
	}
	if masAntiguo {
		superviviente = idLid
		absorbida = idPn
	}

	uPNsur, sPNsur, err := obtenerPNDe(tx, superviviente)
	if err != nil {
		return Identidad{}, err
	}
	uPNabs, sPNabs, err := obtenerPNDe(tx, absorbida)
	if err != nil {
		return Identidad{}, err
	}

	if uPNsur != "" && uPNabs != "" && (uPNsur != uPNabs || sPNsur != sPNabs) {
		if a.registro != nil {
			a.registro.Aviso(EventoConflictoDeAlias, registro.Campos{})
		}
		if err := tx.Commit(); err != nil {
			return Identidad{}, err
		}
		return Identidad{IdInterno: idPn, Estado: EstadoAnclada, ConflictoDeAlias: true}, nil
	}

	_, err = tx.ExecContext(ctx, `UPDATE direccion SET id_interno = ? WHERE id_interno = ?`, superviviente, absorbida)
	if err != nil {
		return Identidad{}, err
	}

	_, err = tx.ExecContext(ctx, `UPDATE identidad SET fusionada_en = ?, actualizada_en_ms = ? WHERE fusionada_en = ?`, superviviente, ahora, absorbida)
	if err != nil {
		return Identidad{}, err
	}

	_, err = tx.ExecContext(ctx, `UPDATE identidad SET estado = ?, fusionada_en = ?, actualizada_en_ms = ? WHERE id_interno = ?`, EstadoFusionada, superviviente, ahora, absorbida)
	if err != nil {
		return Identidad{}, err
	}

	tienePN, err := tieneDireccionPN(tx, superviviente)
	if err != nil {
		return Identidad{}, err
	}
	nuevoEst := EstadoProvisional
	if tienePN {
		nuevoEst = EstadoAnclada
	}
	_, err = tx.ExecContext(ctx, `UPDATE identidad SET estado = ?, actualizada_en_ms = ? WHERE id_interno = ?`, nuevoEst, ahora, superviviente)
	if err != nil {
		return Identidad{}, err
	}

	if a.registro != nil {
		a.registro.Info(EventoIdentidadFusionada, registro.Campos{Detalle: "estado=" + nuevoEst})
	}

	if err := tx.Commit(); err != nil {
		return Identidad{}, err
	}
	return Identidad{IdInterno: superviviente, Estado: nuevoEst}, nil
}

func (a *Almacen) DireccionDe(ctx context.Context, idInterno string) (types.JID, error) {
	a.mu.RLock()
	cerrado := a.cerrado
	a.mu.RUnlock()
	if cerrado {
		return types.JID{}, ErrAlmacenCerrado
	}

	var tipo, u, s string
	q := `
		SELECT tipo, usuario, servidor
		FROM direccion
		WHERE id_interno = ?
		ORDER BY CASE WHEN tipo = 'pn' THEN 0 ELSE 1 END
		LIMIT 1
	`
	err := a.db.QueryRowContext(ctx, q, idInterno).Scan(&tipo, &u, &s)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return types.JID{}, sql.ErrNoRows
		}
		return types.JID{}, err
	}
	return types.NewJID(u, s), nil
}
