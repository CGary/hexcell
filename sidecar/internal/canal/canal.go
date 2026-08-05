// Package canal construye la sesión de whatsmeow del sidecar, gestiona el almacén de dispositivo
// real (sqlstore) y recibe los eventos crudos del canal.
//
// # Almacén de dispositivo
//
// El almacén es un sqlstore.Container abierto en la ruta configurada por el paquete
// configuracion, con el dialecto "sqlite" (modernc.org/sqlite, Go puro, CGO_ENABLED=0).
// El DSN lleva foreign_keys(1), journal_mode(WAL), synchronous(FULL) y busy_timeout(5000),
// las mismas pragmas que el outbox del paquete outbox usa por el mismo motivo.
//
// La sesión se clasifica como emparejada o no emparejada según si el dispositivo del almacén
// tiene un ID no nulo. Un almacén vacío devuelve un dispositivo con ID nulo, que es lo que
// permite a whatsmeow abrir el canal QR y emparejar.
//
// # Por qué el manejador de eventos solo registra el tipo
//
// Un evento de whatsmeow puede llevar el texto de un mensaje. El manejador escribe el **tipo** del
// evento y nada de su contenido, que es la misma frontera estructural de privacidad que adr-0019
// impone al registro del núcleo. La traducción del contenido ocurre en la tarea 8 y va al outbox y
// al socket, nunca a un log.
package canal

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"

	_ "modernc.org/sqlite"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Nombres fijos de suceso que este paquete emite. Son constantes por el motivo de adr-0019:
// ningún valor construido en tiempo de ejecución puede acabar en el campo `evento`.
const (
	EventoSesionConstruida      = "canal.sesion_construida"
	EventoCrudoRecibido         = "canal.evento_crudo_recibido"
	EventoSesionCerrada         = "canal.sesion_cerrada"
	EventoAlmacenAbierto        = "canal.almacen_abierto"
	EventoDispositivoEncontrado = "canal.dispositivo_encontrado"
	EventoDispositivoNuevo      = "canal.dispositivo_nuevo"
)

// ModuloWhatsmeow es el nombre de módulo raíz con el que la biblioteca aparece en el registro.
const ModuloWhatsmeow = "whatsmeow"

// ErrRegistroNoEspecificado se devuelve si se intenta construir una sesión sin registro.
var ErrRegistroNoEspecificado = errors.New("canal: la sesión necesita un registro")

// Sesion es el cliente de whatsmeow del sidecar junto al registro con el que informa.
type Sesion struct {
	cliente     *whatsmeow.Client
	registro    *registro.Registro
	dispositivo *store.Device
	ctx         context.Context
}

// AbrirAlmacenDeDispositivo abre el sqlstore.Container en la ruta dada con el dialecto "sqlite"
// y las pragmas de durabilidad requeridas. Devuelve el contenedor listo para usar; el llamador
// es responsable de cerrarlo cuando termine.
//
// El DSN incluye foreign_keys(1), journal_mode(WAL), synchronous(FULL) y busy_timeout(5000),
// las mismas pragmas que el outbox del paquete outbox usa por el mismo motivo. La diferencia
// con mattn/go-sqlite3 es que la sintaxis es _pragma=X en lugar de ?_X=valor.
func AbrirAlmacenDeDispositivo(ctx context.Context, ruta string, reg *registro.Registro) (*sqlstore.Container, error) {
	dsn := fmt.Sprintf(
		"file:%s?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)&_pragma=busy_timeout(5000)",
		ruta,
	)

	puente := registro.NuevoAdaptadorWaLog(reg, "sqlstore")

	contenedor, err := sqlstore.New(ctx, "sqlite", dsn, puente)
	if err != nil {
		return nil, fmt.Errorf("canal: no se pudo abrir el almacén de dispositivo: %w", err)
	}

	reg.Info(EventoAlmacenAbierto, registro.Campos{
		Detalle: "almacén sqlstore abierto y actualizado",
	})

	return contenedor, nil
}

// NuevaSesion construye el cliente de whatsmeow a partir de un almacén de dispositivo real.
// Si el almacén no tiene un dispositivo previo, se crea uno nuevo (con ID nulo, que habilita
// el emparejamiento). Si ya tiene uno, se reutiliza (sesión emparejada, reanudación automática).
func NuevaSesion(ctx context.Context, contenedor *sqlstore.Container, reg *registro.Registro) (*Sesion, error) {
	if reg == nil {
		return nil, ErrRegistroNoEspecificado
	}

	dispositivo, err := contenedor.GetFirstDevice(ctx)
	if err != nil {
		return nil, fmt.Errorf("canal: no se pudo obtener el dispositivo del almacén: %w", err)
	}

	emparejada := dispositivo.ID != nil
	if emparejada {
		reg.Info(EventoDispositivoEncontrado, registro.Campos{
			Detalle: "dispositivo existente encontrado; sesión reanudable sin emparejamiento",
		})
	} else {
		reg.Info(EventoDispositivoNuevo, registro.Campos{
			Detalle: "almacén vacío; se requiere emparejamiento para conectar",
		})
	}

	puente := registro.NuevoAdaptadorWaLog(reg, ModuloWhatsmeow)
	cliente := whatsmeow.NewClient(dispositivo, puente)

	reg.Info(EventoSesionConstruida, registro.Campos{
		Detalle: "cliente whatsmeow construido sobre almacén sqlstore; sin conexión",
	})
	return &Sesion{
		cliente:     cliente,
		registro:    reg,
		dispositivo: dispositivo,
		ctx:         ctx,
	}, nil
}

// Cliente devuelve el cliente de whatsmeow subyacente.
//
// Se expone para que las tareas posteriores de la etapa —reconexión, traducción— construyan
// sobre él sin que este paquete tenga que anticipar su superficie.
func (s *Sesion) Cliente() *whatsmeow.Client {
	return s.cliente
}

// EstaEmparejada devuelve verdadero si el almacén contiene un dispositivo con ID no nulo,
// lo que indica que hay credenciales emparejadas y la sesión puede reanudarse sin QR.
func (s *Sesion) EstaEmparejada() bool {
	return s.dispositivo.ID != nil
}

// RegistrarManejador engancha el manejador de eventos crudos y devuelve su identificador.
//
// El manejador registra el **tipo** de cada evento recibido y nada más. La traducción al formato
// canónico del puerto, con su identificador de deduplicación, es la tarea 8; el paso previo —
// persistir en el outbox durable antes de cualquier otra cosa— es la tarea 3, y este manejador
// será el punto donde se enganche.
func (s *Sesion) RegistrarManejador() uint32 {
	return s.cliente.AddEventHandler(func(evento any) {
		s.registro.Info(EventoCrudoRecibido, registro.Campos{
			Detalle: fmt.Sprintf("%T", evento),
		})
	})
}

// Conectar abre el websocket saliente hacia WhatsApp.
//
// Ningún test de esta tarea la llama: sin credenciales emparejadas whatsmeow no puede completar el
// inicio de sesión, y probar contra el canal real es la tarea 15 del plan. El punto de entrada
// existe para que la tarea 4 tenga dónde engancharse.
func (s *Sesion) Conectar(ctx context.Context) error {
	return s.cliente.ConnectContext(ctx)
}

// Cerrar desconecta el cliente de forma ordenada.
func (s *Sesion) Cerrar() {
	s.cliente.Disconnect()
	s.registro.Info(EventoSesionCerrada, registro.Campos{})
}

// CerrarDB cierra la conexión a la base de datos del sqlstore. Es una función auxiliar para
// que main.go cierre el almacén durante el apagado ordenado.
func CerrarDB(db *sql.DB) error {
	if db == nil {
		return nil
	}
	return db.Close()
}
