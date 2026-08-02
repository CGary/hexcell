// Package outbox implementa el buzón de salida durable respaldado por SQLite para el sidecar en Go.
//
// # Elección del controlador de SQLite y justificación de durabilidad
//
// Este paquete utiliza `modernc.org/sqlite` (controlador de Go puro registrado en database/sql
// bajo el nombre "sqlite") por las siguientes razones de diseño y arquitectura:
//
//  1. Compatibilidad con whatsmeow: El paquete `sqlstore` de whatsmeow recibe un nombre de dialecto
//     en cadena y llama a `dbutil.ParseDialect`, que acepta cualquier motor cuyo nombre en minúsculas
//     comience con "sqlite". El nombre de controlador "sqlite" que registra `modernc` se resuelve por
//     tanto como `dbutil.SQLite`, permitiendo que la tarea 5 reutilice este mismo controlador con
//     `sqlstore.New(ctx, "sqlite", dsn, log)`.
//
//  2. Independencia de CGO: La alternativa en CGO `github.com/mattn/go-sqlite3` registra su controlador
//     bajo `CGO_ENABLED=0`, pero cada conexión falla en `Ping` con "go-sqlite3 requires cgo to work. This is a stub".
//     Esto obligaría a mantener `CGO_ENABLED=1`, una cadena de herramientas de C en la imagen de construcción A-6
//     y la pérdida del binario estático. El lado Rust eligió `rusqlite` con la función `bundled` exactamente
//     por la misma razón (Cargo.toml raíz, adr-0003): no depender de la biblioteca `libsqlite3` del sistema operativo.
//     El uso de Go puro es el equivalente en Go de esa decisión.
//
//  3. Costo transparente de Go puro: El controlador de Go puro añade aproximadamente 6.9 MB al binario final
//     (un binario de Go "hola mundo" pasa de 2.4 MB a 9.3 MB). Este es un costo de tamaño de imagen en A-6,
//     NO un costo de NFR-01, dado que NFR-01 presupone memoria residente en ejecución y no bytes de imagen.
//
// # Pragmas de durabilidad y rendimiento
//
// La base de datos se abre con los pragmas `journal_mode(WAL)` y `synchronous(FULL)`.
//
// `synchronous=FULL` en modo WAL fuerza un `fsync` real del registro WAL por cada transacción confirmada.
// `synchronous=NORMAL` en modo WAL solo realiza `fsync` en los puntos de control (checkpoints), por lo que
// un evento confirmado podría perderse ante un corte de energía, convirtiendo la garantía de "persistir con fsync primero"
// en una afirmación falsa.
//
// En pruebas de rendimiento en btrfs (200 transacciones de una sola fila):
//   - WAL + NORMAL: 0.010 ms/transacción
//   - WAL + FULL: 0.909 ms/transacción
//   - DELETE + FULL: 3.169 ms/transacción
//
// El salto de ~90x entre NORMAL y FULL representa la ejecución real del `fsync`. El factor de 3.5x entre WAL y DELETE
// refleja las sincronizaciones adicionales del diario de reversión y la serialización entre lectores y escritores.
// Se mantiene WAL para que el bucle de entrega posterior pueda leer entradas pendientes mientras el receptor websocket escribe.
//
// NOTA CRÍTICA: `journal_mode` se persiste en el archivo de la base de datos, pero `synchronous` es una configuración
// POR CONEXIÓN. Dado que `database/sql` administra un grupo de conexiones, los pragmas DEBEN incluirse en la DSN
// para que se apliquen automáticamente a cada conexión nueva del grupo. Además, se fija `SetMaxOpenConns(1)` para evitar
// que el buzón de salida de un solo escritor produzca errores `SQLITE_BUSY` contra sí mismo.
//
// # Estructura y garantía de monotonicidad
//
// La tabla utiliza `secuencia INTEGER PRIMARY KEY AUTOINCREMENT`. El uso explícito de `AUTOINCREMENT` evita que
// SQLite reutilice valores de secuencia liberados por la purga (que ocurriría con un simple `INTEGER PRIMARY KEY` al asignar
// `max(rowid)+1`), garantizando que la monotonicidad del orden de entrega sea una propiedad estructural del esquema.
package outbox

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"sync"
	"time"

	_ "modernc.org/sqlite"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

const (
	// RutaPorOmision es la ruta predeterminada del buzón de salida en el volumen compartido de la célula.
	RutaPorOmision = "/var/lib/hexcell/outbox.db"

	// RetencionPorOmision es la ventana de retención por omisión para entradas confirmadas (7 días).
	// Es un valor inicial para calibrar, exactamente como se trata el TTL de la tarea 12.
	RetencionPorOmision = 7 * 24 * time.Hour

	// EventoPersistido es el nombre fijo de evento al persistir una entrada.
	EventoPersistido = "outbox.evento_persistido"

	// EventoConfirmado es el nombre fijo de evento al confirmar una entrada.
	EventoConfirmado = "outbox.evento_confirmado"

	// EventoPurgado es el nombre fijo de evento al purgar entradas confirmadas.
	EventoPurgado = "outbox.evento_purgado"
)

var (
	// ErrOutboxCerrado se devuelve cuando se intenta operar sobre un Outbox cerrado.
	ErrOutboxCerrado = errors.New("outbox: el buzón de salida está cerrado")

	// ErrIdDeduplicacionVacio se devuelve cuando se intenta persistir un evento sin identificador de deduplicación.
	ErrIdDeduplicacionVacio = errors.New("outbox: el identificador de deduplicación no puede estar vacío")
)

// Opciones configura los parámetros de apertura del buzón de salida.
type Opciones struct {
	Ruta      string
	Retencion time.Duration
	Registro  *registro.Registro
}

// Entrada representa un evento almacenado en el buzón de salida.
type Entrada struct {
	Secuencia       int64
	IdDeduplicacion string
	Carga           []byte
	PersistidoEnMs  int64
	ConfirmadoEnMs  *int64
}

// Outbox administra la persistencia durable y el ciclo de vida de los eventos en SQLite.
type Outbox struct {
	db        *sql.DB
	retencion time.Duration
	registro  *registro.Registro
	mu        sync.RWMutex
	cerrado   bool
}

// Abrir inicializa o conecta con el buzón de salida SQLite en la ruta especificada.
func Abrir(opciones Opciones) (*Outbox, error) {
	ruta := opciones.Ruta
	if ruta == "" {
		ruta = RutaPorOmision
	}
	retencion := opciones.Retencion
	if retencion <= 0 {
		retencion = RetencionPorOmision
	}

	dsn := fmt.Sprintf("file:%s?_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)&_pragma=busy_timeout(5000)", ruta)

	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("outbox: error al abrir base de datos: %w", err)
	}

	db.SetMaxOpenConns(1)

	if err := db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("outbox: error al verificar conexión: %w", err)
	}

	esquema := `
	CREATE TABLE IF NOT EXISTS outbox (
		secuencia INTEGER PRIMARY KEY AUTOINCREMENT,
		id_deduplicacion TEXT NOT NULL UNIQUE,
		carga BLOB NOT NULL,
		persistido_en_ms INTEGER NOT NULL,
		confirmado_en_ms INTEGER NULL
	);
	`
	if _, err := db.Exec(esquema); err != nil {
		db.Close()
		return nil, fmt.Errorf("outbox: error al crear esquema: %w", err)
	}

	return &Outbox{
		db:        db,
		retencion: retencion,
		registro:  opciones.Registro,
	}, nil
}

// Persistir almacena de forma durable un nuevo evento en SQLite con fsync antes de retornar.
func (o *Outbox) Persistir(ctx context.Context, idDeduplicacion string, carga []byte) error {
	if idDeduplicacion == "" {
		return ErrIdDeduplicacionVacio
	}

	o.mu.RLock()
	if o.cerrado {
		o.mu.RUnlock()
		return ErrOutboxCerrado
	}
	o.mu.RUnlock()

	ahoraMs := time.Now().UnixMilli()
	query := `INSERT INTO outbox (id_deduplicacion, carga, persistido_en_ms) VALUES (?, ?, ?) ON CONFLICT(id_deduplicacion) DO NOTHING`

	res, err := o.db.ExecContext(ctx, query, idDeduplicacion, carga, ahoraMs)
	if err != nil {
		return fmt.Errorf("outbox: error al persistir entrada: %w", err)
	}

	if o.registro != nil {
		filas, _ := res.RowsAffected()
		if filas > 0 {
			o.registro.Info(EventoPersistido, registro.Campos{
				IdEvento: idDeduplicacion,
			})
		}
	}

	return nil
}

// Pendientes recupera todas las entradas no confirmadas en estricto orden de persistencia.
func (o *Outbox) Pendientes(ctx context.Context) ([]Entrada, error) {
	o.mu.RLock()
	if o.cerrado {
		o.mu.RUnlock()
		return nil, ErrOutboxCerrado
	}
	o.mu.RUnlock()

	query := `SELECT secuencia, id_deduplicacion, carga, persistido_en_ms, confirmado_en_ms FROM outbox WHERE confirmado_en_ms IS NULL ORDER BY secuencia ASC`

	filas, err := o.db.QueryContext(ctx, query)
	if err != nil {
		return nil, fmt.Errorf("outbox: error al consultar pendientes: %w", err)
	}
	defer filas.Close()

	var entradas []Entrada
	for filas.Next() {
		var e Entrada
		var confirmado sql.NullInt64
		if err := filas.Scan(&e.Secuencia, &e.IdDeduplicacion, &e.Carga, &e.PersistidoEnMs, &confirmado); err != nil {
			return nil, fmt.Errorf("outbox: error al leer entrada pendiente: %w", err)
		}
		if confirmado.Valid {
			v := confirmado.Int64
			e.ConfirmadoEnMs = &v
		}
		entradas = append(entradas, e)
	}
	if err := filas.Err(); err != nil {
		return nil, fmt.Errorf("outbox: error en iteración de pendientes: %w", err)
	}

	if entradas == nil {
		entradas = []Entrada{}
	}

	return entradas, nil
}

// Confirmar marca como procesada la entrada asociada al identificador de deduplicación.
func (o *Outbox) Confirmar(ctx context.Context, idDeduplicacion string) (bool, error) {
	if idDeduplicacion == "" {
		return false, nil
	}

	o.mu.RLock()
	if o.cerrado {
		o.mu.RUnlock()
		return false, ErrOutboxCerrado
	}
	o.mu.RUnlock()

	ahoraMs := time.Now().UnixMilli()
	query := `UPDATE outbox SET confirmado_en_ms = ? WHERE id_deduplicacion = ? AND confirmado_en_ms IS NULL`

	res, err := o.db.ExecContext(ctx, query, ahoraMs, idDeduplicacion)
	if err != nil {
		return false, fmt.Errorf("outbox: error al confirmar entrada: %w", err)
	}

	filas, err := res.RowsAffected()
	if err != nil {
		return false, fmt.Errorf("outbox: error al obtener filas afectadas: %w", err)
	}

	confirmado := filas > 0
	if confirmado && o.registro != nil {
		o.registro.Info(EventoConfirmado, registro.Campos{
			IdEvento: idDeduplicacion,
		})
	}

	return confirmado, nil
}

// Purgar elimina las entradas confirmadas cuya antigüedad supere la ventana de retención configurada.
func (o *Outbox) Purgar(ctx context.Context, ahora time.Time) (int64, error) {
	o.mu.RLock()
	if o.cerrado {
		o.mu.RUnlock()
		return 0, ErrOutboxCerrado
	}
	o.mu.RUnlock()

	limiteMs := ahora.Add(-o.retencion).UnixMilli()
	query := `DELETE FROM outbox WHERE confirmado_en_ms IS NOT NULL AND confirmado_en_ms < ?`

	res, err := o.db.ExecContext(ctx, query, limiteMs)
	if err != nil {
		return 0, fmt.Errorf("outbox: error al purgar entradas: %w", err)
	}

	filas, err := res.RowsAffected()
	if err != nil {
		return 0, fmt.Errorf("outbox: error al obtener filas purgadas: %w", err)
	}

	if filas > 0 && o.registro != nil {
		o.registro.Info(EventoPurgado, registro.Campos{
			Detalle: fmt.Sprintf("entradas purgadas: %d", filas),
		})
	}

	return filas, nil
}

// Cerrar cierra de forma limpia e idempotente la conexión con la base de datos SQLite.
func (o *Outbox) Cerrar() error {
	o.mu.Lock()
	defer o.mu.Unlock()

	if o.cerrado {
		return nil
	}
	o.cerrado = true
	return o.db.Close()
}
