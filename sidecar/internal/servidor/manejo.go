package servidor

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// conexionActiva encapsula el estado por conexión: el descriptor de red, un canal de
// finalización que notifica a las goroutines de lectura y escritura el cierre de la conexión,
// y un canal búfer para mensajes salientes ad-hoc (estado_sesion, acuse_envio, respuestas de
// emparejamiento y respaldo).
type conexionActiva struct {
	conn     net.Conn
	cerrada  chan struct{}
	saliente chan []byte
	once     sync.Once
}

func nuevaConexionActiva(conn net.Conn) *conexionActiva {
	return &conexionActiva{
		conn:     conn,
		cerrada:  make(chan struct{}),
		saliente: make(chan []byte, 100),
	}
}

// cerrar cierra de forma idempotente el canal de señalización y el descriptor de red.
func (c *conexionActiva) cerrar() {
	c.once.Do(func() {
		close(c.cerrada)
		_ = c.conn.Close()
	})
}

// enviar encola un mensaje saliente sin bloquear indefinidamente si la conexión se cierra.
func (c *conexionActiva) enviar(frame []byte) {
	select {
	case <-c.cerrada:
	case c.saliente <- frame:
	}
}

// leerLineaAcotada lee hasta el siguiente salto de línea acotando por [ipc.LongitudMaximaDeLinea].
func leerLineaAcotada(r *bufio.Reader) ([]byte, error) {
	linea, err := r.ReadBytes('\n')
	if err != nil {
		return nil, err
	}
	if len(linea) > ipc.LongitudMaximaDeLinea {
		return nil, ipc.ErrLineaDemasiadoLarga
	}
	return linea, nil
}

// atenderConexion realiza el apretón de manos inicial (saludo estricto v4), aplica el relevo de
// conexión única (most-recent-wins) y arranca las goroutines lectora y escritora.
func (s *Servidor) atenderConexion(ctx context.Context, conn net.Conn) {
	lector := bufio.NewReader(conn)

	// 1. Leer el saludo inicial del cliente
	linea, err := leerLineaAcotada(lector)
	if err != nil {
		registroProtocoloError(s.deps.Registro, err)
		_ = conn.Close()
		return
	}

	sobre, err := ipc.Decodificar(linea)
	if err != nil {
		registroProtocoloError(s.deps.Registro, err)
		_ = conn.Close()
		return
	}

	if sobre.Tipo != ipc.TipoSaludo {
		registroProtocoloError(s.deps.Registro, fmt.Errorf("%w: se esperaba %q, recibido %q", ipc.ErrCuerpoIncoherente, ipc.TipoSaludo, sobre.Tipo))
		_ = conn.Close()
		return
	}

	// 2. Aceptada la conexión: relevar la anterior si existía (most-recent-wins)
	nueva := nuevaConexionActiva(conn)

	s.mu.Lock()
	if s.cerrado {
		s.mu.Unlock()
		_ = conn.Close()
		return
	}
	anterior := s.actual
	s.actual = nueva
	s.mu.Unlock()

	if anterior != nil {
		anterior.cerrar()
		if s.deps.Registro != nil {
			s.deps.Registro.Info("servidor.conexion_reemplazada", registro.Campos{
				Detalle: "nueva conexión IPC aceptada; conexión previa cerrada por relevo",
			})
		}
	}

	// 3. Responder con el saludo propio del sidecar antes de entregar cualquier evento
	saludo := ipc.NuevoSobre(ipc.Saludo{
		Emisor:   ipc.EmisorSidecar,
		IdCelula: s.deps.IdCelula,
	})
	b, err := ipc.Codificar(saludo)
	if err != nil {
		registroProtocoloError(s.deps.Registro, err)
		nueva.cerrar()
		return
	}

	if _, err := nueva.conn.Write(b); err != nil {
		nueva.cerrar()
		return
	}

	if s.deps.Registro != nil {
		s.deps.Registro.Info("servidor.saludo_completado", registro.Campos{
			Detalle: "apretón de manos de saludo versión 4 completado con éxito",
		})
	}

	// 4. Iniciar bucles concurrentes de lectura y escritura
	go s.leerEntrante(ctx, nueva, lector)
	go s.escribirSaliente(ctx, nueva)
}

// leerEntrante decodifica líneas recibidas del cliente IPC y las enruta a los manejadores
// correspondientes. Ante cualquier error de protocolo, cierra la conexión (fallo cerrado).
func (s *Servidor) leerEntrante(ctx context.Context, c *conexionActiva, lector *bufio.Reader) {
	defer c.cerrar()

	for {
		linea, err := leerLineaAcotada(lector)
		if err != nil {
			if !errors.Is(err, io.EOF) && !errors.Is(err, net.ErrClosed) {
				select {
				case <-c.cerrada:
					// Cierre esperado por relevo o parada
					return
				default:
					registroProtocoloError(s.deps.Registro, err)
				}
			}
			return
		}

		sobre, err := ipc.Decodificar(linea)
		if err != nil {
			registroProtocoloError(s.deps.Registro, err)
			return
		}

		switch sobre.Tipo {
		case ipc.TipoConfirmacion:
			if conf, ok := sobre.Cuerpo.(ipc.Confirmacion); ok && s.deps.Buzon != nil {
				_, _ = s.deps.Buzon.Confirmar(ctx, conf.IdDeduplicacion)
			}
		case ipc.TipoOrdenRespaldoSqlstore:
			if orden, ok := sobre.Cuerpo.(ipc.OrdenRespaldoSqlstore); ok {
				acuse := canal.ManejarOrdenRespaldoSqlstore(ctx, s.deps.DBRespaldo, orden, s.deps.Registro)
				if b, err := ipc.Codificar(ipc.NuevoSobre(acuse)); err == nil {
					c.enviar(b)
				}
			}
		case ipc.TipoOrdenEmparejar:
			if orden, ok := sobre.Cuerpo.(ipc.OrdenEmparejar); ok {
				go s.procesarOrdenEmparejar(ctx, c, orden)
			}
		case ipc.TipoMensajeSaliente:
			if saliente, ok := sobre.Cuerpo.(ipc.MensajeSaliente); ok && s.deps.Portero != nil {
				_ = s.deps.Portero.Admitir(ctx, saliente.IdMensaje, saliente.IdConversacion, saliente.Contenido, saliente.MarcaTemporalOrigenMs)
			}
		default:
			registroProtocoloError(s.deps.Registro, fmt.Errorf("%w: tipo no esperado en conexión establecida: %q", ipc.ErrTipoDesconocido, sobre.Tipo))
			return
		}
	}
}

// procesarOrdenEmparejar atiende las órdenes de emparejamiento (QR o código de vinculación)
// en una goroutine dedicada para no bloquear la lectura de otros mensajes IPC entrantes.
func (s *Servidor) procesarOrdenEmparejar(ctx context.Context, c *conexionActiva, orden ipc.OrdenEmparejar) {
	if s.deps.Sesion == nil {
		acuse := ipc.NuevoSobre(ipc.AcuseEmparejamiento{
			Resultado: ipc.ResultadoEmparejamientoFallido,
			Motivo:    "sesión de canal no disponible",
		})
		if b, err := ipc.Codificar(acuse); err == nil {
			c.enviar(b)
		}
		return
	}

	switch orden.Metodo {
	case ipc.MetodoQr:
		canalQr, err := s.deps.Sesion.IniciarEmparejamientoQr()
		if err != nil {
			acuse := ipc.NuevoSobre(ipc.AcuseEmparejamiento{
				Resultado: ipc.ResultadoEmparejamientoFallido,
				Motivo:    err.Error(),
			})
			if b, errCod := ipc.Codificar(acuse); errCod == nil {
				c.enviar(b)
			}
			return
		}

		for res := range canalQr {
			if res.Completado {
				acuse := ipc.NuevoSobre(ipc.AcuseEmparejamiento{
					Resultado: ipc.ResultadoEmparejamientoCompletado,
					Motivo:    "",
				})
				if b, errCod := ipc.Codificar(acuse); errCod == nil {
					c.enviar(b)
				}
			} else if res.Expirado {
				acuse := ipc.NuevoSobre(ipc.AcuseEmparejamiento{
					Resultado: ipc.ResultadoEmparejamientoExpirado,
					Motivo:    "",
				})
				if b, errCod := ipc.Codificar(acuse); errCod == nil {
					c.enviar(b)
				}
			} else if res.Codigo != "" {
				cod := ipc.NuevoSobre(ipc.CodigoEmparejamiento{
					Metodo:     ipc.MetodoQr,
					Valor:      res.Codigo,
					ExpiraEnMs: res.ExpiraEnMs,
				})
				if b, errCod := ipc.Codificar(cod); errCod == nil {
					c.enviar(b)
				}
			}
		}

	case ipc.MetodoCodigoDeVinculacion:
		codigo, err := s.deps.Sesion.SolicitarCodigoDeVinculacion(ctx, s.deps.TelefonoCelula)
		if err != nil {
			acuse := ipc.NuevoSobre(ipc.AcuseEmparejamiento{
				Resultado: ipc.ResultadoEmparejamientoFallido,
				Motivo:    err.Error(),
			})
			if b, errCod := ipc.Codificar(acuse); errCod == nil {
				c.enviar(b)
			}
			return
		}

		cod := ipc.NuevoSobre(ipc.CodigoEmparejamiento{
			Metodo:     ipc.MetodoCodigoDeVinculacion,
			Valor:      codigo,
			ExpiraEnMs: 0,
		})
		if b, errCod := ipc.Codificar(cod); errCod == nil {
			c.enviar(b)
		}

	default:
		acuse := ipc.NuevoSobre(ipc.AcuseEmparejamiento{
			Resultado: ipc.ResultadoEmparejamientoFallido,
			Motivo:    fmt.Sprintf("método de emparejamiento desconocido: %s", orden.Metodo),
		})
		if b, errCod := ipc.Codificar(acuse); errCod == nil {
			c.enviar(b)
		}
	}
}

// escribirSaliente drena entradas no confirmadas del outbox (redelivery) tras el saludo inicial
// y maneja mensajes salientes encolados o nuevas señales de trabajo.
func (s *Servidor) escribirSaliente(ctx context.Context, c *conexionActiva) {
	defer c.cerrar()

	// 1. Redistribución inicial de entradas no confirmadas tras completar el saludo
	s.drenarPendientes(ctx, c)

	// 2. Bucle de drenaje de mensajes salientes y señales de nuevo trabajo
	for {
		select {
		case <-ctx.Done():
			return
		case <-c.cerrada:
			return
		case frame := <-c.saliente:
			if _, err := c.conn.Write(frame); err != nil {
				return
			}
		case <-s.trabajo:
			s.drenarPendientes(ctx, c)
		}
	}
}

// drenarPendientes envía todas las entradas no confirmadas del outbox por el socket.
func (s *Servidor) drenarPendientes(ctx context.Context, c *conexionActiva) {
	if s.deps.Buzon == nil {
		return
	}
	pendientes, err := s.deps.Buzon.Pendientes(ctx)
	if err != nil {
		return
	}
	for _, entrada := range pendientes {
		if _, err := c.conn.Write(entrada.Carga); err != nil {
			return
		}
	}
}

// registroProtocoloError registra un error de protocolo reportando únicamente la categoría
// del error y el campo afectado, sin exponer líneas crudas ni contenido sensible (adr-0019).
func registroProtocoloError(reg *registro.Registro, err error) {
	if reg == nil {
		return
	}
	reg.Error("servidor.error_protocolo", registro.Campos{
		Detalle: err.Error(),
	})
}
