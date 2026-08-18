package canal

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

func retrocesoDePrueba() configuracion.Retroceso {
	return configuracion.Retroceso{
		IntervaloInicial: 1000,
		Factor:           2,
		IntervaloMaximo:  4000,
		BaneoInicial:     1000,
		BaneoMaximo:      5000,
	}
}

func TestIntervaloDeRetrocesoCreceYSeClavaEnElTecho(t *testing.T) {
	t.Parallel()

	inicial := time.Second
	maximo := 4 * time.Second
	esperados := []time.Duration{
		time.Second,
		2 * time.Second,
		4 * time.Second,
		4 * time.Second,
		4 * time.Second,
		4 * time.Second,
		4 * time.Second,
		4 * time.Second,
	}
	for intento, esperado := range esperados {
		obtenido := intervaloDeRetroceso(intento+1, inicial, 2, maximo)
		if obtenido != esperado {
			t.Fatalf("intento %d = %s, se esperaba %s", intento+1, obtenido, esperado)
		}
	}
}

func TestIntervaloDeRetrocesoSeClavaEnUnTechoNoMultiplo(t *testing.T) {
	t.Parallel()

	// El techo NO es múltiplo del inicial por el factor: 1000 → 2000 → 4000 se pasaría de
	// 3000. Sin el recorte por exceso el intento 3 devolvería 4000 y este caso se pone rojo.
	inicial := 1000 * time.Millisecond
	maximo := 3000 * time.Millisecond
	esperados := []time.Duration{
		1000 * time.Millisecond,
		2000 * time.Millisecond,
		3000 * time.Millisecond,
		3000 * time.Millisecond,
		3000 * time.Millisecond,
	}
	for intento, esperado := range esperados {
		obtenido := intervaloDeRetroceso(intento+1, inicial, 2, maximo)
		if obtenido != esperado {
			t.Fatalf("intento %d = %s, se esperaba %s", intento+1, obtenido, esperado)
		}
	}
}

func TestSupervisorReintentaTransitorioConTechoYRegistraIntentos(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	var esperas []time.Duration
	var estados []ipc.EstadoSesion
	intentosConexion := 0

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		intentosConexion++
		if intentosConexion < 4 {
			return errors.New("fallo transitorio inyectado")
		}
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		return nil
	}

	supervisor.procesarEvento(context.Background(), &events.Disconnected{})

	esperadas := []time.Duration{time.Second, 2 * time.Second, 4 * time.Second, 4 * time.Second}
	if !reflect.DeepEqual(esperas, esperadas) {
		t.Fatalf("esperas = %v, se esperaba %v", esperas, esperadas)
	}
	if intentosConexion != 4 {
		t.Fatalf("intentos de conexión = %d", intentosConexion)
	}
	if len(estados) < 2 || estados[0].Estado != ipc.EstadoReconectando || estados[1].Estado != ipc.EstadoActiva {
		t.Fatalf("estados emitidos = %#v", estados)
	}
	log := salida.String()
	if strings.Count(log, EventoReintentoConexion) < 4 {
		t.Fatalf("log sin una línea por intento: %s", log)
	}
	if !strings.Contains(log, EventoEstadoSesion) {
		t.Fatalf("log sin transición de estado: %s", log)
	}
}

func TestSupervisorPausaBaneoTemporalSinReconectarNiReactivar(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	var esperas []time.Duration
	var lineasIPC []string
	ahora := ahoraFijoMs
	intentosConexion := 0

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		intentosConexion++
		return nil
	}, func(estado ipc.EstadoSesion) {
		linea, err := ipc.Codificar(ipc.NuevoSobre(estado))
		if err != nil {
			t.Fatalf("Codificar estado_sesion: %v", err)
		}
		lineasIPC = append(lineasIPC, string(linea))
	})
	supervisor.ahoraMs = func() int64 { return ahora }
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		ahora += duracion.Milliseconds()
		return nil
	}

	supervisor.procesarEvento(context.Background(), &events.TemporaryBan{
		Code:   events.TempBanBlockedByUsers,
		Expire: 7 * time.Second,
	})

	if len(lineasIPC) == 0 || !strings.Contains(lineasIPC[0], `"estado":"pausada"`) {
		t.Fatalf("no se codificó la entrada a pausada: %v", lineasIPC)
	}
	if !strings.Contains(lineasIPC[0], `"causa":"baneo_temporal"`) ||
		!strings.Contains(lineasIPC[0], `"codigo":102`) ||
		!strings.Contains(lineasIPC[0], `"expira_en_ms":1786083207000`) {
		t.Fatalf("estado_sesion no conserva la señal cruda: %s", lineasIPC[0])
	}
	esperadas := []time.Duration{time.Second, 2 * time.Second, 4 * time.Second}
	if !reflect.DeepEqual(esperas, esperadas) {
		t.Fatalf("esperas de baneo = %v, se esperaba %v", esperas, esperadas)
	}
	if intentosConexion != 0 {
		t.Fatalf("se intentó reconectar durante el baneo: %d", intentosConexion)
	}
	for _, linea := range lineasIPC[1:] {
		if strings.Contains(linea, `"estado":"reconectando"`) || strings.Contains(linea, `"estado":"activa"`) {
			t.Fatalf("hubo transición fuera de pausada sin reinicio: %s", linea)
		}
	}
	if !strings.Contains(salida.String(), EventoRetrocesoBaneo) {
		t.Fatalf("no se registró el retroceso largo por baneo: %s", salida.String())
	}
}

func TestSupervisorDistingueSesionInvalidaDeErrorTransitorio(t *testing.T) {
	t.Parallel()

	var salidaTransitoria bytes.Buffer
	regTransitorio := registro.Nuevo(&salidaTransitoria, slog.LevelInfo, "test")
	conexionesTransitorias := 0
	var estadosTransitorios []ipc.EstadoSesion
	transitorio := NuevoSupervisor(regTransitorio, retrocesoDePrueba(), func(context.Context) error {
		conexionesTransitorias++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estadosTransitorios = append(estadosTransitorios, estado)
	})
	transitorio.esperar = func(context.Context, time.Duration) error { return nil }
	transitorio.procesarEvento(context.Background(), &events.Disconnected{})
	if conexionesTransitorias == 0 || len(estadosTransitorios) == 0 || estadosTransitorios[0].Estado != ipc.EstadoReconectando {
		t.Fatalf("la rama transitoria no estuvo viva: conexiones=%d estados=%#v", conexionesTransitorias, estadosTransitorios)
	}

	var salidaInvalida bytes.Buffer
	regInvalido := registro.Nuevo(&salidaInvalida, slog.LevelInfo, "test")
	conexionesInvalidas := 0
	var estadosInvalidos []ipc.EstadoSesion
	invalido := NuevoSupervisor(regInvalido, retrocesoDePrueba(), func(context.Context) error {
		conexionesInvalidas++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estadosInvalidos = append(estadosInvalidos, estado)
	})
	invalido.esperar = func(context.Context, time.Duration) error {
		t.Fatalf("la sesión inválida no debe esperar para reconectar")
		return nil
	}
	invalido.procesarEvento(context.Background(), &events.LoggedOut{
		OnConnect: false,
		Reason:    events.ConnectFailureLoggedOut,
	})
	if len(estadosInvalidos) == 0 || estadosInvalidos[0].Estado != ipc.EstadoDesvinculada {
		t.Fatalf("no se emitió desvinculada: %#v", estadosInvalidos)
	}
	if conexionesInvalidas != 0 {
		t.Fatalf("la sesión inválida intentó reconectar: %d", conexionesInvalidas)
	}
	if !strings.Contains(salidaTransitoria.String(), EventoReintentoConexion) ||
		!strings.Contains(salidaInvalida.String(), EventoEstadoSesion) {
		t.Fatalf("faltan logs positivos: transitorio=%s invalido=%s", salidaTransitoria.String(), salidaInvalida.String())
	}
}

func TestSupervisorNoExponeMetodoDeReanudacion(t *testing.T) {
	t.Parallel()

	tipo := reflect.TypeOf(&Supervisor{})
	for i := 0; i < tipo.NumMethod(); i++ {
		nombre := strings.ToLower(tipo.Method(i).Name)
		if strings.Contains(nombre, "resume") || strings.Contains(nombre, "reanudar") ||
			strings.Contains(nombre, "reactivar") || strings.Contains(nombre, "unpause") ||
			strings.Contains(nombre, "despausar") || strings.Contains(nombre, "pausa") {
			t.Fatalf("Supervisor expone método de reanudación: %s", tipo.Method(i).Name)
		}
	}
}

func TestSupervisorBaneoSinExpiracionNoReconecta(t *testing.T) {
	t.Parallel()

	conexiones := 0
	var estados []ipc.EstadoSesion
	var esperas []time.Duration
	supervisor := NuevoSupervisor(nil, retrocesoDePrueba(), func(context.Context) error {
		conexiones++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		return nil
	}

	supervisor.procesarEvento(context.Background(), &events.TemporaryBan{
		Code:   events.TempBanSentToTooManyPeople,
		Expire: 0,
	})

	if len(estados) != 1 || estados[0].Estado != ipc.EstadoPausada || estados[0].ExpiraEnMs != 0 {
		t.Fatalf("no se emitió pausa con expiración desconocida: %#v", estados)
	}
	if len(esperas) != 1 || esperas[0] != time.Second {
		t.Fatalf("espera larga desconocida = %v", esperas)
	}
	if conexiones != 0 {
		t.Fatalf("se reconectó durante baneo de expiración desconocida: %d", conexiones)
	}
}

// TestSupervisorIgnoraParaSiempreLosEventosPosterioresAlBaneoTemporal cubre el camino que solo
// se abre bajo CONCURRENCIA en producción: whatsmeow despacha cada evento en su propia
// goroutine, así que una desconexión cualquiera puede llegar mientras la célula ya está
// pausada. La pausa es ABSORBENTE: ningún evento posterior la levanta y la única salida es
// reiniciar el proceso. Reconectar durante un baneo temporal lo escala a permanente.
func TestSupervisorIgnoraParaSiempreLosEventosPosterioresAlBaneoTemporal(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	ahora := ahoraFijoMs
	conexiones := 0
	var estados []ipc.EstadoSesion
	var esperas []time.Duration

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		conexiones++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.ahoraMs = func() int64 { return ahora }
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		ahora += duracion.Milliseconds()
		return nil
	}

	supervisor.procesarEvento(context.Background(), &events.TemporaryBan{
		Code:   events.TempBanBlockedByUsers,
		Expire: 3 * time.Second,
	})

	// PRESENCIA: la pausa se entró de verdad antes de afirmar cualquier ausencia.
	if len(estados) != 1 || estados[0].Estado != ipc.EstadoPausada {
		t.Fatalf("la pausa no se entró: %#v", estados)
	}
	if estados[0].Causa != ipc.CausaBaneoTemporal || estados[0].Codigo != int64(events.TempBanBlockedByUsers) {
		t.Fatalf("la señal cruda no viajó con la proyección: %#v", estados[0])
	}
	if estados[0].ExpiraEnMs != ahoraFijoMs+(3*time.Second).Milliseconds() {
		t.Fatalf("expira_en_ms = %d, se esperaba la conversión absoluta", estados[0].ExpiraEnMs)
	}
	if len(esperas) == 0 {
		t.Fatalf("el retroceso largo por baneo no llegó a correr")
	}
	if !strings.Contains(salida.String(), EventoRetrocesoBaneo) {
		t.Fatalf("no se registró el retroceso largo: %s", salida.String())
	}
	if conexiones != 0 {
		t.Fatalf("se intentó conectar durante la pausa: %d", conexiones)
	}

	estadosTrasPausa := len(estados)
	esperasTrasPausa := len(esperas)

	// AUSENCIA: con la pausa vigente, ningún evento posterior reconecta ni cambia de estado.
	// El reloj ya pasó el vencimiento declarado y aun así la célula sigue inerte.
	posteriores := []any{
		&events.Disconnected{},
		&events.ConnectFailure{Reason: events.ConnectFailureServiceUnavailable},
		&events.StreamReplaced{},
		&events.StreamError{Code: "515"},
		&events.ClientOutdated{},
		&events.LoggedOut{OnConnect: true, Reason: events.ConnectFailureLoggedOut},
		&events.TemporaryBan{Code: events.TempBanSentToTooManyPeople, Expire: time.Second},
	}
	for _, evento := range posteriores {
		supervisor.procesarEvento(context.Background(), evento)
	}

	if conexiones != 0 {
		t.Fatalf("la célula reconectó después del baneo: %d intentos", conexiones)
	}
	if len(esperas) != esperasTrasPausa {
		t.Fatalf("se abrió un retroceso nuevo tras la pausa: %v", esperas)
	}
	if len(estados) != estadosTrasPausa {
		t.Fatalf("hubo transición de estado saliendo de pausada: %#v", estados)
	}
	if !strings.Contains(salida.String(), EventoPausaVigente) {
		t.Fatalf("no se registró que los eventos se ignoran por pausa: %s", salida.String())
	}
	if strings.Contains(salida.String(), EventoReconexionRestaurada) {
		t.Fatalf("se registró una reconexión restaurada durante la pausa: %s", salida.String())
	}
}

// TestSupervisorRecortaElRetrocesoLargoAlVencimientoDelBaneo cubre el recorte por `restante`:
// con un vencimiento que no es múltiplo del retroceso largo, la última espera se acorta para
// no pasarse de la expiración declarada.
func TestSupervisorRecortaElRetrocesoLargoAlVencimientoDelBaneo(t *testing.T) {
	t.Parallel()

	ahora := ahoraFijoMs
	var esperas []time.Duration
	var estados []ipc.EstadoSesion
	conexiones := 0

	supervisor := NuevoSupervisor(nil, retrocesoDePrueba(), func(context.Context) error {
		conexiones++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.ahoraMs = func() int64 { return ahora }
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		ahora += duracion.Milliseconds()
		return nil
	}

	supervisor.procesarEvento(context.Background(), &events.TemporaryBan{
		Code:   events.TempBanBlockedByUsers,
		Expire: 2500 * time.Millisecond,
	})

	// PRESENCIA: la pausa entró con su vencimiento absoluto antes de mirar las esperas.
	if len(estados) != 1 || estados[0].Estado != ipc.EstadoPausada {
		t.Fatalf("la pausa no se entró: %#v", estados)
	}
	if estados[0].ExpiraEnMs != ahoraFijoMs+2500 {
		t.Fatalf("expira_en_ms = %d", estados[0].ExpiraEnMs)
	}
	// 1000 crece a 2000, pero solo quedan 1500 hasta el vencimiento: la segunda espera se
	// recorta. Sin el recorte la secuencia sería 1000, 2000.
	esperadas := []time.Duration{1000 * time.Millisecond, 1500 * time.Millisecond}
	if !reflect.DeepEqual(esperas, esperadas) {
		t.Fatalf("esperas de baneo = %v, se esperaba %v", esperas, esperadas)
	}
	if conexiones != 0 {
		t.Fatalf("se intentó conectar durante el baneo: %d", conexiones)
	}
}

// TestSupervisorProcesaElBaneoConUnIntentoDeConexionEnVuelo fija la propiedad que hace vivible
// al supervisor en producción: `conectar` corre FUERA del candado. whatsmeow no acota la subida
// a websocket, así que un par que completa TCP y TLS y luego calla deja el intento colgado sin
// plazo; si ese intento retuviera mu, el evento de baneo temporal —que llega en su propia
// goroutine— quedaría bloqueado para siempre y con él toda goroutine de eventos. Aquí el baneo
// se procesa ENTERO con el intento en vuelo, y al volver el intento la pausa ya vigente gana.
func TestSupervisorProcesaElBaneoConUnIntentoDeConexionEnVuelo(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")

	var mu sync.Mutex
	ahora := ahoraFijoMs
	conexiones := 0
	var estados []ipc.EstadoSesion

	enConexion := make(chan struct{})
	soltarConexion := make(chan struct{})

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		mu.Lock()
		conexiones++
		primera := conexiones == 1
		mu.Unlock()
		if primera {
			close(enConexion)
			<-soltarConexion
		}
		return nil
	}, func(estado ipc.EstadoSesion) {
		mu.Lock()
		defer mu.Unlock()
		estados = append(estados, estado)
	})
	supervisor.ahoraMs = func() int64 {
		mu.Lock()
		defer mu.Unlock()
		return ahora
	}
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		mu.Lock()
		defer mu.Unlock()
		ahora += duracion.Milliseconds()
		return nil
	}

	var reconexion sync.WaitGroup
	reconexion.Add(1)
	go func() {
		defer reconexion.Done()
		supervisor.procesarEvento(context.Background(), &events.Disconnected{})
	}()
	<-enConexion

	baneoListo := make(chan struct{})
	go func() {
		defer close(baneoListo)
		supervisor.procesarEvento(context.Background(), &events.TemporaryBan{
			Code:   events.TempBanBlockedByUsers,
			Expire: 3 * time.Second,
		})
	}()

	// Con `conectar` bajo el candado este baneo no avanzaría nunca. El plazo hace que el caso
	// falle RÁPIDO en vez de colgar la suite hasta el tiempo límite de `go test`.
	select {
	case <-baneoListo:
	case <-time.After(10 * time.Second):
		t.Fatal("el baneo quedó bloqueado detrás del intento de conexión en vuelo")
	}

	// PRESENCIA: el intento estaba de verdad en vuelo y la pausa entró de verdad, con su
	// vencimiento absoluto. El bucle esperó IntervaloInicial antes de conectar, así que el
	// reloj inyectado ya había avanzado 1000 ms cuando llegó el baneo.
	mu.Lock()
	instantanea := append([]ipc.EstadoSesion(nil), estados...)
	intentos := conexiones
	mu.Unlock()
	if intentos != 1 {
		t.Fatalf("el intento de conexión no estaba en vuelo: %d", intentos)
	}
	if len(instantanea) != 2 || instantanea[0].Estado != ipc.EstadoReconectando ||
		instantanea[1].Estado != ipc.EstadoPausada {
		t.Fatalf("estados hasta la pausa = %#v", instantanea)
	}
	if instantanea[1].Causa != ipc.CausaBaneoTemporal ||
		instantanea[1].ExpiraEnMs != ahoraFijoMs+1000+(3*time.Second).Milliseconds() {
		t.Fatalf("la pausa no registró el vencimiento declarado: %#v", instantanea[1])
	}

	close(soltarConexion)
	reconexion.Wait()

	// AUSENCIA: el intento que volvió tarde no escribió estado encima de la pausa, no arrancó
	// ningún intento NUEVO y quedó registrado que se descartó por pausa vigente.
	mu.Lock()
	defer mu.Unlock()
	if conexiones != 1 {
		t.Fatalf("arrancó un intento nuevo con la pausa vigente: %d", conexiones)
	}
	if len(estados) != len(instantanea) {
		t.Fatalf("se emitió estado encima de la pausa: %#v", estados)
	}
	if !strings.Contains(salida.String(), EventoPausaVigente) {
		t.Fatalf("no se registró el descarte por pausa vigente: %s", salida.String())
	}
	if strings.Contains(salida.String(), EventoReconexionRestaurada) {
		t.Fatalf("se registró una reconexión restaurada tras el baneo: %s", salida.String())
	}
}

// TestSupervisorNoAbreDosBuclesDeReconexionSimultaneos comprueba la propiedad que la
// concurrencia de whatsmeow pone en riesgo: la biblioteca despacha cada evento con
// `go cli.dispatchEvent`, así que dos desconexiones solapadas entran al supervisor a la vez.
// Debe existir exactamente UNA política de reconexión viva. Se ejecuta bajo -race.
func TestSupervisorNoAbreDosBuclesDeReconexionSimultaneos(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")

	var mu sync.Mutex
	conexiones := 0
	var estados []ipc.EstadoSesion

	enEspera := make(chan struct{})
	continuar := make(chan struct{})
	detenida := false

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		mu.Lock()
		defer mu.Unlock()
		conexiones++
		return nil
	}, func(estado ipc.EstadoSesion) {
		mu.Lock()
		defer mu.Unlock()
		estados = append(estados, estado)
	})
	// Solo la PRIMERA espera queda detenida hasta que el test la suelte. Las siguientes
	// devuelven al instante a propósito: si la guarda de política única desaparece, el evento
	// solapado corre su bucle entero y el caso se pone rojo enseguida, en vez de colgarse.
	supervisor.esperar = func(context.Context, time.Duration) error {
		mu.Lock()
		primera := !detenida
		detenida = true
		mu.Unlock()
		if primera {
			close(enEspera)
			<-continuar
		}
		return nil
	}

	var primera sync.WaitGroup
	primera.Add(1)
	go func() {
		defer primera.Done()
		supervisor.procesarEvento(context.Background(), &events.Disconnected{})
	}()
	<-enEspera

	var segunda sync.WaitGroup
	segunda.Add(1)
	go func() {
		defer segunda.Done()
		supervisor.procesarEvento(context.Background(), &events.ConnectFailure{
			Reason: events.ConnectFailureServiceUnavailable,
		})
	}()
	segunda.Wait()

	// El segundo evento solapado no abrió su propio bucle: no conectó nada mientras el
	// primero seguía vivo.
	mu.Lock()
	conexionesSolapadas := conexiones
	mu.Unlock()
	if conexionesSolapadas != 0 {
		t.Fatalf("el evento solapado abrió una segunda política: %d conexiones", conexionesSolapadas)
	}
	if !strings.Contains(salida.String(), EventoPoliticaEnCurso) {
		t.Fatalf("no se registró el rechazo del evento solapado: %s", salida.String())
	}

	close(continuar)
	primera.Wait()

	// PRESENCIA: la política que sí estaba viva llegó a conectar de verdad.
	mu.Lock()
	defer mu.Unlock()
	if conexiones != 1 {
		t.Fatalf("conexiones tras soltar la política = %d, se esperaba 1", conexiones)
	}
	if len(estados) == 0 || estados[len(estados)-1].Estado != ipc.EstadoActiva {
		t.Fatalf("la política viva no llegó a activa: %#v", estados)
	}
}

// TestSupervisorNoArrancaUnIntentoNuevoConLaPausaYaVigente cubre la guarda que corre JUSTO ANTES
// de llamar a conectar: el baneo entra mientras el bucle de reintento espera su retroceso, así que
// al volver la espera la célula ya está pausada y el intento NUEVO no debe arrancar. Es la mitad
// que sostiene la garantía terminal; la otra —el intento ya en vuelo— la cubre el caso anterior.
func TestSupervisorNoArrancaUnIntentoNuevoConLaPausaYaVigente(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	ahora := ahoraFijoMs
	conexiones := 0
	var estados []ipc.EstadoSesion

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		conexiones++
		return nil
	}, func(estado ipc.EstadoSesion) { estados = append(estados, estado) })
	supervisor.ahoraMs = func() int64 { return ahora }
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		ahora += duracion.Milliseconds()
		if supervisor.enPausa() {
			return nil
		}
		// El baneo se procesa ENTERO desde dentro de la primera espera del bucle transitorio: su
		// retroceso largo reentra aquí y termina en el vencimiento declarado, sin colgar la suite.
		supervisor.procesarEvento(context.Background(), &events.TemporaryBan{
			Code:   events.TempBanBlockedByUsers,
			Expire: 3 * time.Second,
		})
		return nil
	}

	supervisor.procesarEvento(context.Background(), &events.Disconnected{})

	// PRESENCIA: la pausa se entró de verdad, con su señal cruda y su vencimiento absoluto.
	if len(estados) != 2 || estados[0].Estado != ipc.EstadoReconectando {
		t.Fatalf("el bucle transitorio no llegó a la pausa: %#v", estados)
	}
	if estados[1].Estado != ipc.EstadoPausada || estados[1].Causa != ipc.CausaBaneoTemporal ||
		estados[1].Codigo != int64(events.TempBanBlockedByUsers) ||
		estados[1].ExpiraEnMs != ahoraFijoMs+1000+(3*time.Second).Milliseconds() {
		t.Fatalf("la pausa no entró con su señal cruda: %#v", estados[1])
	}

	// AUSENCIA: con la pausa ya vigente no arrancó NINGÚN intento nuevo, y quedó registrado.
	if conexiones != 0 {
		t.Fatalf("arrancó un intento con la pausa ya vigente: %d", conexiones)
	}
	if !strings.Contains(salida.String(), EventoPausaVigente) {
		t.Fatalf("no se registró el descarte por pausa vigente: %s", salida.String())
	}
}

func TestSupervisorArrancarConDispositivoEmparejadoDisparaConexionYEmiteEstadoActiva(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	var esperas []time.Duration
	var estados []ipc.EstadoSesion
	intentosConexion := 0

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		intentosConexion++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		return nil
	}

	supervisor.Arrancar(context.Background(), true)

	esperadas := []time.Duration{time.Second}
	if !reflect.DeepEqual(esperas, esperadas) {
		t.Fatalf("esperas = %v, se esperaba %v", esperas, esperadas)
	}
	if intentosConexion != 1 {
		t.Fatalf("intentos de conexión = %d, se esperaba 1", intentosConexion)
	}
	if len(estados) != 1 || estados[0].Estado != ipc.EstadoActiva {
		t.Fatalf("estados emitidos = %#v, se esperaba [activa]", estados)
	}
	log := salida.String()
	if !strings.Contains(log, EventoReintentoConexion) || !strings.Contains(log, "causa=arranque_inicial") {
		t.Fatalf("log sin causa de arranque inicial: %s", log)
	}
	if !strings.Contains(log, EventoReconexionRestaurada) {
		t.Fatalf("log sin reconexión restaurada: %s", log)
	}
}

func TestSupervisorArrancarSinDispositivoEsNoOp(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	var esperas []time.Duration
	var estados []ipc.EstadoSesion
	intentosConexion := 0

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		intentosConexion++
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		return nil
	}

	supervisor.Arrancar(context.Background(), false)

	if intentosConexion != 0 {
		t.Fatalf("intentos de conexión = %d, se esperaba 0", intentosConexion)
	}
	if len(esperas) != 0 {
		t.Fatalf("esperas = %v, se esperaba vacías", esperas)
	}
	if len(estados) != 0 {
		t.Fatalf("estados emitidos = %#v, se esperaba vacíos", estados)
	}
	if salida.Len() != 0 {
		t.Fatalf("se escribió log en arranque sin dispositivo: %s", salida.String())
	}
}

func TestSupervisorArrancarConFallosReintentaSegunRetroceso(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	var esperas []time.Duration
	var estados []ipc.EstadoSesion
	intentosConexion := 0

	supervisor := NuevoSupervisor(reg, retrocesoDePrueba(), func(context.Context) error {
		intentosConexion++
		if intentosConexion < 3 {
			return errors.New("fallo transitorio de arranque")
		}
		return nil
	}, func(estado ipc.EstadoSesion) {
		estados = append(estados, estado)
	})
	supervisor.esperar = func(_ context.Context, duracion time.Duration) error {
		esperas = append(esperas, duracion)
		return nil
	}

	supervisor.Arrancar(context.Background(), true)

	esperadas := []time.Duration{time.Second, 2 * time.Second, 4 * time.Second}
	if !reflect.DeepEqual(esperas, esperadas) {
		t.Fatalf("esperas = %v, se esperaba %v", esperas, esperadas)
	}
	if intentosConexion != 3 {
		t.Fatalf("intentos de conexión = %d, se esperaba 3", intentosConexion)
	}
	if len(estados) != 1 || estados[0].Estado != ipc.EstadoActiva {
		t.Fatalf("estados emitidos = %#v, se esperaba [activa]", estados)
	}
	log := salida.String()
	if strings.Count(log, EventoReintentoConexion) < 3 {
		t.Fatalf("log sin entradas para cada intento: %s", log)
	}
}
