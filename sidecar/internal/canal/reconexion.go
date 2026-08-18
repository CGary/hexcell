package canal

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Espera es la costura inyectable del supervisor para no dormir en tests.
type Espera func(context.Context, time.Duration) error

// SumideroDeEstado recibe cada estado de sesión listo para codificarse por IPC.
type SumideroDeEstado func(ipc.EstadoSesion)

// Nombres fijos de sucesos de reconexión y estado.
const (
	EventoEstadoSesion           = "canal.estado_sesion"
	EventoReintentoConexion      = "canal.reintento_conexion"
	EventoRetrocesoBaneo         = "canal.retroceso_baneo_temporal"
	EventoReconexionRestaurada   = "canal.reconexion_restaurada"
	EventoReconexionInterrumpida = "canal.reconexion_interrumpida"
	EventoPausaVigente           = "canal.pausa_vigente"
	EventoPoliticaEnCurso        = "canal.politica_en_curso"
	causaArranqueInicial         = "arranque_inicial"
)

// Supervisor aplica la política de reconexión propia del sidecar.
//
// CONCURRENCIA: whatsmeow despacha CADA evento en una goroutine nueva (`go cli.dispatchEvent`),
// y canal.go engancha procesarEvento directamente a ese manejador. Dos desconexiones solapadas
// entran aquí a la vez, así que todo el estado mutable vive detrás de mu y no puede leerse ni
// escribirse sin el candado.
type Supervisor struct {
	registro  *registro.Registro
	retroceso configuracion.Retroceso
	conectar  func(context.Context) error
	esperar   Espera
	sumidero  SumideroDeEstado
	ahoraMs   func() int64

	mu            sync.Mutex
	intentos      int
	intentosBaneo int
	ultimoEstado  string
	// pausada es TERMINAL y de un solo sentido: se pone en true al proyectar el estado
	// `pausada` por baneo temporal y NUNCA vuelve a false. No existe método exportado,
	// evento, mensaje IPC ni secuencia de eventos que la limpie; la única salida es
	// reiniciar el proceso, que es una decisión humana.
	//
	// Por qué es absorbente y no un temporizador: persistir con el cliente no oficial
	// durante un baneo temporal ESCALA el baneo a permanente, de modo que una célula que
	// reconecta durante la pausa quema el número del cliente. expira_en_ms es INFORMACIÓN
	// PARA EL OPERADOR, jamás un disparador: el supervisor no reanuda por su cuenta cuando
	// vence la expiración.
	pausada bool
	// reconexionEnCurso impide que dos eventos solapados abran dos bucles de reintento a la
	// vez y dupliquen la política de retroceso: existe exactamente UNA política viva.
	reconexionEnCurso bool
}

// NuevoSupervisor construye el servicio de aplicación que clasifica desconexiones, emite
// estado_sesion y controla los reintentos. Si el sumidero es nil, el estado se registra pero no
// se entrega a ningún socket, porque el servidor IPC todavía no existe en esta tarea.
func NuevoSupervisor(
	reg *registro.Registro,
	retroceso configuracion.Retroceso,
	conectar func(context.Context) error,
	sumidero SumideroDeEstado,
) *Supervisor {
	if sumidero == nil {
		sumidero = func(ipc.EstadoSesion) {}
	}
	return &Supervisor{
		registro:  reg,
		retroceso: retroceso,
		conectar:  conectar,
		esperar:   esperarConTemporizador,
		sumidero:  sumidero,
		ahoraMs:   func() int64 { return time.Now().UnixMilli() },
	}
}

func esperarConTemporizador(ctx context.Context, duracion time.Duration) error {
	if duracion <= 0 {
		return nil
	}
	temporizador := time.NewTimer(duracion)
	defer temporizador.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-temporizador.C:
		return nil
	}
}

func intervaloDeRetroceso(intento int, inicial time.Duration, factor int64, maximo time.Duration) time.Duration {
	if intento <= 1 {
		if inicial > maximo {
			return maximo
		}
		return inicial
	}

	actual := inicial
	for i := 1; i < intento; i++ {
		if actual >= maximo {
			return maximo
		}
		if factor <= 1 {
			return actual
		}
		siguiente := actual * time.Duration(factor)
		if siguiente <= actual || siguiente > maximo {
			return maximo
		}
		actual = siguiente
	}
	return actual
}

func (s *Supervisor) procesarEvento(ctx context.Context, evento any) {
	desc, ok := clasificarDesconexion(evento, s.ahoraMs())
	if !ok {
		return
	}
	s.procesarDesconexion(ctx, desc)
}

// Arrancar es el único punto de entrada que inicia la conexión automática de un dispositivo
// ya emparejado al arrancar el sidecar. Si emparejada es false, es una operación nula (un almacén
// sin dispositivo nunca se conecta solo y el emparejamiento sigue siendo la única entrada).
// El primer intento también espera IntervaloInicial antes de marcar la conexión, respetando
// la disciplina de retroceso configurada.
func (s *Supervisor) Arrancar(ctx context.Context, emparejada bool) {
	if !emparejada {
		return
	}
	s.reintentarConexion(ctx, causaArranqueInicial)
}

func (s *Supervisor) procesarDesconexion(ctx context.Context, desc desconexion) {
	estado := ipc.EstadoSesion{
		Estado:     proyectarEstado(desc.Causa),
		Causa:      desc.Causa,
		Codigo:     desc.Codigo,
		ExpiraEnMs: desc.ExpiraEnMs,
	}

	// La pausa por baneo se marca ANTES de emitir nada: cualquier bucle de reintento en
	// vuelo la ve en su siguiente comprobación y se detiene sin intentar conectar.
	if estado.Estado == ipc.EstadoPausada {
		if !s.entrarEnPausaTerminal() {
			s.registrarPausaVigente(desc.Causa)
			return
		}
		s.emitirEstado(estado)
		s.esperarBaneoTemporal(ctx, desc)
		return
	}

	// Guarda terminal: con la célula pausada, TODO evento posterior se ignora para siempre.
	if s.enPausa() {
		s.registrarPausaVigente(desc.Causa)
		return
	}

	s.emitirEstado(estado)

	if estado.Estado == ipc.EstadoDesvinculada {
		s.reiniciarContadores()
		return
	}
	s.reintentarConexion(ctx, desc.Causa)
}

// entrarEnPausaTerminal marca la pausa absorbente y devuelve true solo la primera vez.
// No existe la operación inversa, ni exportada ni interna: la pausa no se limpia nunca.
func (s *Supervisor) entrarEnPausaTerminal() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.pausada {
		return false
	}
	s.pausada = true
	return true
}

func (s *Supervisor) enPausa() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.pausada
}

func (s *Supervisor) reiniciarContadores() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.intentos = 0
	s.intentosBaneo = 0
}

// tomarPolitica reserva la única política de reconexión viva. Devuelve false si ya hay un
// bucle de reintento corriendo, en cuyo caso el evento solapado se descarta con registro.
func (s *Supervisor) tomarPolitica() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.reconexionEnCurso {
		return false
	}
	s.reconexionEnCurso = true
	return true
}

func (s *Supervisor) soltarPolitica() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.reconexionEnCurso = false
}

func (s *Supervisor) registrarPausaVigente(causa string) {
	if s.registro == nil {
		return
	}
	s.registro.Info(EventoPausaVigente, registro.Campos{
		Detalle: fmt.Sprintf("celula pausada por baneo temporal: evento ignorado causa=%s", causa),
	})
}

func (s *Supervisor) registrarPoliticaEnCurso(causa string) {
	if s.registro == nil {
		return
	}
	s.registro.Info(EventoPoliticaEnCurso, registro.Campos{
		Detalle: fmt.Sprintf("ya hay una politica de reconexion viva: evento ignorado causa=%s", causa),
	})
}

// emitirEstado serializa la entrega al sumidero bajo el candado: el protocolo IPC es de una
// línea por mensaje y dos goroutines de eventos no deben entrelazar sus líneas.
// Ningún llamador puede tener mu tomado al entrar aquí.
func (s *Supervisor) emitirEstado(estado ipc.EstadoSesion) {
	s.mu.Lock()
	s.ultimoEstado = estado.Estado
	s.sumidero(estado)
	s.mu.Unlock()
	if s.registro != nil {
		s.registro.Info(EventoEstadoSesion, registro.Campos{
			Detalle: fmt.Sprintf("estado=%s causa=%s codigo=%d expira_en_ms=%d",
				estado.Estado, estado.Causa, estado.Codigo, estado.ExpiraEnMs),
		})
	}
}

func (s *Supervisor) reintentarConexion(ctx context.Context, causa string) {
	if !s.tomarPolitica() {
		s.registrarPoliticaEnCurso(causa)
		return
	}
	defer s.soltarPolitica()

	for {
		s.mu.Lock()
		if s.pausada {
			s.mu.Unlock()
			s.registrarPausaVigente(causa)
			return
		}
		s.intentos++
		intento := s.intentos
		s.mu.Unlock()

		intervalo := s.intervaloNormal(intento)
		if s.registro != nil {
			s.registro.Info(EventoReintentoConexion, registro.Campos{
				LatenciaMs: intervalo.Milliseconds(),
				Detalle:    fmt.Sprintf("intento=%d causa=%s", intento, causa),
			})
		}
		if err := s.esperar(ctx, intervalo); err != nil {
			s.registrarInterrupcion(err)
			return
		}

		// El intento de conexión ocurre FUERA del candado, a propósito. conectar llama a
		// ConnectContext sin plazo y whatsmeow construye su cliente HTTP de websocket sin
		// campo Timeout, de modo que la subida a websocket NO ESTÁ ACOTADA: un par que
		// completa TCP y TLS y luego calla retendría mu para siempre y dejaría bloqueadas en
		// entrarEnPausaTerminal, enPausa y emitirEstado a todas las goroutines de eventos de
		// whatsmeow, una fuga por evento. Tampoco se acota el intento con un plazo de
		// contexto: whatsmeow adopta el contexto del marcado como contexto de la SESIÓN, así
		// que un plazo derribaría la sesión viva al vencer.
		//
		// CARRERA RESIDUAL, declarada sin adornos: puede quedar exactamente UN intento de
		// conexión en vuelo cuando llega el evento de baneo. Es irreducible, porque el baneo
		// lo decide el servidor remoto y ningún orden local lo anticipa. El diseño anterior
		// TAMPOCO la eliminaba: solo la cambiaba por una retención no acotada del candado.
		// La exclusión mutua entre intentos concurrentes ya la da la reserva de política
		// única, y lo que sí queda garantizado es lo garantizable: una vez registrado el
		// baneo, ningún intento NUEVO arranca jamás.
		s.mu.Lock()
		pausada := s.pausada
		conectar := s.conectar
		s.mu.Unlock()
		if pausada {
			s.registrarPausaVigente(causa)
			return
		}
		if conectar == nil {
			return
		}

		err := conectar(ctx)

		// Recomprobación tras el intento: si el baneo llegó mientras se conectaba, gana el
		// baneo y no se escribe ningún estado encima de la pausa.
		s.mu.Lock()
		pausada = s.pausada
		if !pausada && err == nil {
			s.intentos = 0
		}
		s.mu.Unlock()
		if pausada {
			s.registrarPausaVigente(causa)
			return
		}

		if err != nil {
			if s.registro != nil {
				s.registro.Aviso(EventoReintentoConexion, registro.Campos{
					LatenciaMs: intervalo.Milliseconds(),
					Detalle:    fmt.Sprintf("intento=%d error=%s", intento, err.Error()),
				})
			}
			continue
		}
		if s.registro != nil {
			s.registro.Info(EventoReconexionRestaurada, registro.Campos{})
		}
		s.emitirEstado(ipc.EstadoSesion{Estado: ipc.EstadoActiva})
		return
	}
}

// esperarBaneoTemporal acompaña la pausa con retroceso largo hasta el vencimiento declarado.
// Cuando vuelve, NO reanuda nada: la marca pausada sigue puesta para siempre y el supervisor
// queda inerte. Volver al servicio exige reiniciar el proceso.
func (s *Supervisor) esperarBaneoTemporal(ctx context.Context, desc desconexion) {
	for {
		ahora := s.ahoraMs()
		if desc.ExpiraEnMs > 0 && ahora >= desc.ExpiraEnMs {
			return
		}

		s.mu.Lock()
		s.intentosBaneo++
		intento := s.intentosBaneo
		s.mu.Unlock()

		intervalo := s.intervaloBaneo(intento)
		if desc.ExpiraEnMs > 0 {
			// Recorte al vencimiento declarado: el retroceso largo nunca se pasa de la
			// expiración, para no dejar la última espera colgando más allá del baneo.
			restante := time.Duration(desc.ExpiraEnMs-ahora) * time.Millisecond
			if intervalo > restante {
				intervalo = restante
			}
		}
		if s.registro != nil {
			s.registro.Info(EventoRetrocesoBaneo, registro.Campos{
				LatenciaMs: intervalo.Milliseconds(),
				Detalle:    fmt.Sprintf("intento=%d causa=%s", intento, desc.Causa),
			})
		}
		if err := s.esperar(ctx, intervalo); err != nil {
			s.registrarInterrupcion(err)
			return
		}
		if desc.ExpiraEnMs == 0 {
			return
		}
	}
}

func (s *Supervisor) intervaloNormal(intento int) time.Duration {
	return intervaloDeRetroceso(
		intento,
		time.Duration(s.retroceso.IntervaloInicial)*time.Millisecond,
		s.retroceso.Factor,
		time.Duration(s.retroceso.IntervaloMaximo)*time.Millisecond,
	)
}

func (s *Supervisor) intervaloBaneo(intento int) time.Duration {
	return intervaloDeRetroceso(
		intento,
		time.Duration(s.retroceso.BaneoInicial)*time.Millisecond,
		s.retroceso.Factor,
		time.Duration(s.retroceso.BaneoMaximo)*time.Millisecond,
	)
}

func (s *Supervisor) registrarInterrupcion(err error) {
	if s.registro == nil {
		return
	}
	s.registro.Aviso(EventoReconexionInterrumpida, registro.Campos{Detalle: err.Error()})
}
