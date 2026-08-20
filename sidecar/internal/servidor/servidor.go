// Package servidor implementa el servidor del socket IPC de dominio Unix del sidecar en Go,
// según lo especificado en docs/protocolo-ipc-nucleo-sidecar.md, sección 2 (transporte y
// procedimiento de socket huérfano), sección 3 (saludo y control estricto de versión) y
// sección 4 (redistribución y confirmación at-least-once).
//
// El sidecar actúa como SERVIDOR (bind+listen) sobre un socket AF_UNIX SOCK_STREAM con modo
// 0600 en el volumen compartido de la célula, y el núcleo Rust se conecta como cliente.
package servidor

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"sync"
	"syscall"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// ErrOtroSidecarActivo se devuelve cuando la prueba de conexión al socket tiene éxito,
// indicando que otro proceso sidecar vivo ya está escuchando en la ruta configurada.
var ErrOtroSidecarActivo = errors.New("servidor: otro proceso sidecar ya está escuchando en el socket IPC")

// Dependencias agrupa los servicios y configuración requeridos por el servidor IPC.
type Dependencias struct {
	RutaSocket          string
	IdCelula            string
	Registro            *registro.Registro
	Buzon               *outbox.Outbox
	Portero             *outbox.PorteroDeSalida
	DBRespaldo          *sql.DB
	DBRespaldoIdentidad *sql.DB
	Sesion              *canal.Sesion
	TelefonoCelula      string
}

// Servidor gestiona la apertura, escucha, despacho de conexiones y ciclo de vida del socket IPC.
type Servidor struct {
	deps     Dependencias
	listener *net.UnixListener
	mu       sync.RWMutex
	actual   *conexionActiva
	trabajo  chan struct{}
	cerrado  bool
}

// NuevoServidor construye una nueva instancia del servidor con las dependencias inyectadas.
func NuevoServidor(deps Dependencias) *Servidor {
	return &Servidor{
		deps:    deps,
		trabajo: make(chan struct{}, 1),
	}
}

// Escuchar ejecuta el procedimiento de inicio con detección de socket huérfano según la
// sección 2 de docs/protocolo-ipc-nucleo-sidecar.md:
//
//  1. Intenta conectarse como cliente (sondeo) a la ruta configurada.
//  2. Si la conexión tiene éxito -> otro sidecar vivo está escuchando: registra y termina
//     devolviendo ErrOtroSidecarActivo sin eliminar nada.
//  3. Si la conexión es rechazada (ECONNREFUSED) o el archivo no existe -> el socket es huérfano
//     o nuevo: desenlaza la ruta si existe, abre net.ListenUnix y fija permisos 0600.
//  4. Cualquier otro error de sondeo -> aborta el arranque devolviendo el error sin borrar nada.
func (s *Servidor) Escuchar(ctx context.Context) error {
	ruta := s.deps.RutaSocket
	if ruta == "" {
		ruta = configuracion.RutaSocketPorOmision
	}

	dir := filepath.Dir(ruta)
	if err := os.MkdirAll(dir, 0755); err != nil {
		if s.deps.Registro != nil {
			s.deps.Registro.Error("servidor.error_directorio_socket", registro.Campos{Detalle: err.Error()})
		}
		return fmt.Errorf("servidor: error al preparar directorio del socket: %w", err)
	}

	// Paso 1: Sondeo de conexión previa
	probeConn, err := net.Dial("unix", ruta)
	if err == nil {
		// Rama 2: Conexión exitosa -> otro sidecar activo
		_ = probeConn.Close()
		if s.deps.Registro != nil {
			s.deps.Registro.Error("servidor.otro_sidecar_activo", registro.Campos{
				Detalle: fmt.Sprintf("otro proceso sidecar está escuchando en %s; terminando arranque", ruta),
			})
		}
		return ErrOtroSidecarActivo
	}

	// Rama 3: Socket huérfano o archivo inexistente
	if esConexionRechazadaONoExiste(err, ruta) {
		if errRem := os.Remove(ruta); errRem != nil && !os.IsNotExist(errRem) {
			if s.deps.Registro != nil {
				s.deps.Registro.Error("servidor.error_eliminar_socket_huerfano", registro.Campos{Detalle: errRem.Error()})
			}
			return fmt.Errorf("servidor: no se pudo eliminar el socket huérfano: %w", errRem)
		}

		addr := &net.UnixAddr{Name: ruta, Net: "unix"}
		l, errListen := net.ListenUnix("unix", addr)
		if errListen != nil {
			if s.deps.Registro != nil {
				s.deps.Registro.Error("servidor.error_escucha", registro.Campos{Detalle: errListen.Error()})
			}
			return fmt.Errorf("servidor: fallo al abrir socket Unix: %w", errListen)
		}

		if errChmod := os.Chmod(ruta, 0600); errChmod != nil {
			_ = l.Close()
			if s.deps.Registro != nil {
				s.deps.Registro.Error("servidor.error_permisos_socket", registro.Campos{Detalle: errChmod.Error()})
			}
			return fmt.Errorf("servidor: fallo al fijar permisos 0600 en socket: %w", errChmod)
		}

		s.mu.Lock()
		s.listener = l
		s.cerrado = false
		s.mu.Unlock()

		if s.deps.Registro != nil {
			s.deps.Registro.Info("servidor.socket_abierto", registro.Campos{
				Detalle: fmt.Sprintf("escuchando en %s con modo 0600", ruta),
			})
		}
		return nil
	}

	// Rama 4: Cualquier otro error
	if s.deps.Registro != nil {
		s.deps.Registro.Error("servidor.error_sondeo_socket", registro.Campos{
			Detalle: fmt.Sprintf("error al sondear socket %s: %v", ruta, err),
		})
	}
	return fmt.Errorf("servidor: error al sondear socket previo: %w", err)
}

// esConexionRechazadaONoExiste determina si el error de conexión corresponde a una conexión
// rechazada (socket huérfano) o archivo inexistente.
func esConexionRechazadaONoExiste(err error, ruta string) bool {
	if errors.Is(err, syscall.ECONNREFUSED) {
		return true
	}
	if os.IsNotExist(err) || errors.Is(err, syscall.ENOENT) {
		return true
	}
	if _, errStat := os.Stat(ruta); os.IsNotExist(errStat) {
		return true
	}
	return false
}

// Aceptar ejecuta el bucle de aceptación de conexiones entrantes. Cada conexión aceptada se
// atiende en su propia goroutine vía [atenderConexion].
func (s *Servidor) Aceptar(ctx context.Context) {
	for {
		s.mu.RLock()
		l := s.listener
		cerrado := s.cerrado
		s.mu.RUnlock()

		if cerrado || l == nil {
			return
		}

		conn, err := l.Accept()
		if err != nil {
			s.mu.RLock()
			cerrado = s.cerrado
			s.mu.RUnlock()
			if cerrado || errors.Is(err, net.ErrClosed) {
				return
			}
			if s.deps.Registro != nil {
				s.deps.Registro.Aviso("servidor.error_aceptar", registro.Campos{Detalle: err.Error()})
			}
			continue
		}

		go s.atenderConexion(ctx, conn)
	}
}

// Cerrar detiene el listener, desenlaza el archivo del socket y cierra la conexión activa si existe.
func (s *Servidor) Cerrar() error {
	s.mu.Lock()
	if s.cerrado {
		s.mu.Unlock()
		return nil
	}
	s.cerrado = true
	l := s.listener
	s.listener = nil
	act := s.actual
	s.actual = nil
	s.mu.Unlock()

	if act != nil {
		act.cerrar()
	}

	if l != nil {
		return l.Close()
	}
	return nil
}

// EnviarEstadoSesion envía un mensaje de estado de sesión a la conexión IPC actualmente activa.
// Si no hay conexión activa, no realiza ninguna acción (estado_sesion no persiste en outbox).
func (s *Servidor) EnviarEstadoSesion(estado ipc.EstadoSesion) {
	sobre := ipc.NuevoSobre(estado)
	b, err := ipc.Codificar(sobre)
	if err != nil {
		if s.deps.Registro != nil {
			s.deps.Registro.Error("servidor.error_codificar_estado_sesion", registro.Campos{Detalle: err.Error()})
		}
		return
	}

	s.mu.RLock()
	act := s.actual
	s.mu.RUnlock()

	if act != nil {
		act.enviar(b)
	}
}

// EnviarAcuseEnvio envía un acuse de envío a la conexión IPC actualmente activa.
// Si no hay conexión activa, no realiza ninguna acción.
func (s *Servidor) EnviarAcuseEnvio(acuse ipc.AcuseEnvio) {
	sobre := ipc.NuevoSobre(acuse)
	b, err := ipc.Codificar(sobre)
	if err != nil {
		if s.deps.Registro != nil {
			s.deps.Registro.Error("servidor.error_codificar_acuse_envio", registro.Campos{Detalle: err.Error()})
		}
		return
	}

	s.mu.RLock()
	act := s.actual
	s.mu.RUnlock()

	if act != nil {
		act.enviar(b)
	}
}

// NotificarTrabajo envía una señal no bloqueante al canal de trabajo para que el escritor
// saliente drene eventos pendientes del outbox hacia la conexión activa.
func (s *Servidor) NotificarTrabajo() {
	select {
	case s.trabajo <- struct{}{}:
	default:
	}
}
