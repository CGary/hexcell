// Package metricas es el productor de métricas nativas del canal propio: agrega tres series
// acotadas —ratio de acuse por contacto, reconexiones por hora y ventana de silencio entrante— y
// las emite en una sola línea periódica de registro estructurado (`docs/adr/adr-0033`).
//
// # Por qué es una hoja
//
// Este paquete importa solo la biblioteca estándar más internal/registro. NO importa
// internal/canal, internal/outbox ni whatsmeow: los observadores reciben escalares
// (id_conversacion, id_correlacion, estado como string) y es sidecar/main.go, la raíz de
// composición, quien adapta los tipos concretos de esas costuras a esta API. Esa disciplina evita
// un ciclo de importación y mantiene el binario de pruebas de este paquete libre de whatsmeow.
//
// # Por qué dos mapas acotados y no uno
//
// canal.Acuse (HEX-072-a) e ipc.AcuseEnvio solo llevan id_correlacion, estado y marca de tiempo:
// ninguno lleva un identificador de contacto. La segmentación por contacto exige entonces una
// unión id_correlacion -> id_conversacion, que este productor mantiene en `correlaciones`
// mientras el envío sigue sin confirmar. `contactos` es el estado de largo plazo (una entrada por
// conversación activa); `correlaciones` es de corta vida (una entrada por envío en vuelo, borrada
// en cuanto ObservarAcuse la resuelve). Por eso llevan cotas independientes: 256 contactos y 1024
// correlaciones, cada una con su propio desalojo determinista y su propio aporte al contador
// compartido `contactos_omitidos`.
package metricas

import (
	"context"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

const (
	// EventoMetricasSidecar es el nombre fijo de suceso de la instantánea periódica, homólogo al
	// `metricas_instantanea` de adr-0024 pero propio del sidecar.
	EventoMetricasSidecar = "sidecar.metricas_instantanea"

	// MaximoContactos acota el estado de largo plazo por conversación.
	MaximoContactos = 256
	// MaximoCorrelaciones acota el estado transitorio de envíos aún no confirmados.
	MaximoCorrelaciones = 1024

	// IntervaloDeInstantanea replica el INTERVALO_DE_INSTANTANEA de adr-0024 en el lado Go.
	IntervaloDeInstantanea = 60 * time.Second

	// estadoSesionConectada es el valor de ipc.EstadoActiva copiado como literal, nunca importado:
	// importar internal/ipc arrastraría el conjunto cerrado del protocolo a un paquete que solo
	// necesita comparar una cadena. sidecar/main.go pasa exactamente ese valor concreto.
	estadoSesionConectada = "activa"

	unaHoraEnMs = int64(time.Hour / time.Millisecond)
)

// contacto es el estado acumulado de una conversación: cuántos envíos se observaron, cuántos
// fueron acusados (entregado o leído, ambos cuentan como confirmación) y cuándo fue su última
// actividad, la marca que decide a quién desalojar primero.
type contacto struct {
	enviados          int64
	acusados          int64
	ultimaActividadMs int64
}

// correlacionPendiente une un id_correlacion en vuelo con el contacto que lo originó, y con el
// instante en que se registró: esa marca, no una de actividad recurrente, es lo que ordena su
// desalojo, porque una correlación no tiene más actividad que su propia creación.
type correlacionPendiente struct {
	idConversacion string
	creadaMs       int64
}

// Productor acumula las tres series y expone la instantánea determinista de texto plano que
// sidecar/main.go emite por su Bucle. Todo el estado mutable vive detrás de mu: ObservarEnvio,
// ObservarAcuse, ObservarEstadoSesion y ObservarEntrante se llaman desde manejadores de eventos de
// whatsmeow, que whatsmeow despacha cada uno en su propia goroutine.
type Productor struct {
	reg     *registro.Registro
	ahoraMs func() int64

	mu                sync.Mutex
	contactos         map[string]*contacto
	correlaciones     map[string]*correlacionPendiente
	contactosOmitidos int64
	reconexiones      int64
	sesionConectada   bool
	inicioMs          int64
	ultimoEntranteMs  int64
}

// NuevoProductor construye el productor con el reloj inyectado como costura de prueba: ninguna
// lógica de este archivo llama a time.Now() directamente. Un ahoraMs nulo (el caso de producción,
// cableado desde sidecar/main.go) recae en el reloj real, el mismo patrón que canal.NuevoSupervisor
// ya usa para su propio reloj inyectable.
func NuevoProductor(reg *registro.Registro, ahoraMs func() int64) *Productor {
	if ahoraMs == nil {
		ahoraMs = func() int64 { return time.Now().UnixMilli() }
	}
	ahora := ahoraMs()
	return &Productor{
		reg:              reg,
		ahoraMs:          ahoraMs,
		contactos:        make(map[string]*contacto),
		correlaciones:    make(map[string]*correlacionPendiente),
		inicioMs:         ahora,
		ultimoEntranteMs: ahora,
	}
}

// pareceJID detecta la forma de una dirección cruda de WhatsApp (siempre lleva "@", como en
// "521.../s.whatsapp.net"), nunca la de un id_conversacion interno. Es una guarda de defensa en
// profundidad: además de que ninguna costura de sidecar/main.go debe pasar jamás un JID, el propio
// productor lo rechaza como clave de contacto si de todos modos llegara uno, para que la frontera
// de privacidad de adr-0019 sea un hecho del tipo de dato y no solo una convención del llamador.
func pareceJID(id string) bool {
	return strings.Contains(id, "@")
}

// ObservarEnvio registra un envío saliente: crea o refresca el contacto y abre la correlación que
// unirá el futuro acuse con su contacto. Un id_conversacion con forma de JID se descarta sin
// registrar nada, la misma guarda de privacidad que forbid.behaviors exige.
func (p *Productor) ObservarEnvio(idConversacion, idCorrelacion string) {
	if idConversacion == "" || idCorrelacion == "" || pareceJID(idConversacion) {
		return
	}
	ahora := p.ahoraMs()

	p.mu.Lock()
	defer p.mu.Unlock()

	if _, existe := p.correlaciones[idCorrelacion]; !existe {
		if len(p.correlaciones) >= MaximoCorrelaciones {
			p.desalojarCorrelacion()
		}
		p.correlaciones[idCorrelacion] = &correlacionPendiente{idConversacion: idConversacion, creadaMs: ahora}
	}

	c, existe := p.contactos[idConversacion]
	if !existe {
		if len(p.contactos) >= MaximoContactos {
			p.desalojarContacto()
		}
		c = &contacto{}
		p.contactos[idConversacion] = c
	}
	c.enviados++
	c.ultimaActividadMs = ahora
}

// ObservarAcuse resuelve la unión id_correlacion -> id_conversacion y confirma el envío: entregado
// y leído cuentan igual como acuse. Una correlación desconocida o ya desalojada es un no-op
// silencioso a propósito, la misma disciplina que canal.manejarEventoDeAcuse aplica a un evento que
// no clasifica: nunca inventa un contacto fantasma ni entra en pánico. Resuelta la unión, la
// correlación se borra: ya cumplió su único propósito y libera cupo de MaximoCorrelaciones para
// los envíos todavía en vuelo.
func (p *Productor) ObservarAcuse(idCorrelacion, estado string) {
	if idCorrelacion == "" || estado == "" {
		return
	}
	ahora := p.ahoraMs()

	p.mu.Lock()
	defer p.mu.Unlock()

	corr, existe := p.correlaciones[idCorrelacion]
	if !existe {
		return
	}
	c, existe := p.contactos[corr.idConversacion]
	if existe {
		c.acusados++
		c.ultimaActividadMs = ahora
	}
	delete(p.correlaciones, idCorrelacion)
}

// ObservarEstadoSesion cuenta las transiciones HACIA el estado conectado, nunca cada estado
// conectado repetido: dos "activa" consecutivas sin una desconexión de por medio son la misma
// conexión, no dos reconexiones.
func (p *Productor) ObservarEstadoSesion(estado string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	conectada := estado == estadoSesionConectada
	if conectada && !p.sesionConectada {
		p.reconexiones++
	}
	p.sesionConectada = conectada
}

// ObservarEntrante estampa el instante del último evento entrante recibido, reiniciando a cero la
// ventana de silencio que Instantanea() reporta.
func (p *Productor) ObservarEntrante() {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.ultimoEntranteMs = p.ahoraMs()
}

// desalojarContacto elige y borra el contacto de actividad más antigua, con el id ascendente como
// desempate, e incrementa contactosOmitidos. El recorrido es un escaneo de mínimo puro: el
// resultado depende solo de los valores comparados, nunca del orden de iteración del mapa de Go,
// que -count=1 no garantiza estable entre corridas. Debe llamarse con mu ya tomado.
func (p *Productor) desalojarContacto() {
	victima := ""
	var marcaVictima int64
	for id, c := range p.contactos {
		if victima == "" || c.ultimaActividadMs < marcaVictima || (c.ultimaActividadMs == marcaVictima && id < victima) {
			victima = id
			marcaVictima = c.ultimaActividadMs
		}
	}
	if victima == "" {
		return
	}
	delete(p.contactos, victima)
	p.contactosOmitidos++
}

// desalojarCorrelacion aplica el mismo desalojo determinista sobre las correlaciones pendientes,
// ordenado por su instante de creación (su única "actividad" posible) con el id ascendente como
// desempate. Debe llamarse con mu ya tomado.
func (p *Productor) desalojarCorrelacion() {
	victima := ""
	var marcaVictima int64
	for id, corr := range p.correlaciones {
		if victima == "" || corr.creadaMs < marcaVictima || (corr.creadaMs == marcaVictima && id < victima) {
			victima = id
			marcaVictima = corr.creadaMs
		}
	}
	if victima == "" {
		return
	}
	delete(p.correlaciones, victima)
	p.contactosOmitidos++
}

// Instantanea construye el payload determinista de la línea periódica: siempre las tres series
// agregadas (reconexiones_por_hora, silencio_entrante_ms, contactos_omitidos) más una entrada
// ack_ratio.<id_conversacion> por cada contacto conocido, en orden ascendente de id para que dos
// llamadas sobre el mismo estado produzcan el mismo texto byte a byte, sin depender del orden de
// iteración del mapa de Go.
func (p *Productor) Instantanea() string {
	ahora := p.ahoraMs()

	p.mu.Lock()
	defer p.mu.Unlock()

	var tasaReconexion float64
	if transcurridoMs := ahora - p.inicioMs; transcurridoMs > 0 {
		horas := float64(transcurridoMs) / float64(unaHoraEnMs)
		tasaReconexion = float64(p.reconexiones) / horas
	}

	silencioMs := ahora - p.ultimoEntranteMs
	if silencioMs < 0 {
		silencioMs = 0
	}

	ids := make([]string, 0, len(p.contactos))
	for id := range p.contactos {
		ids = append(ids, id)
	}
	sort.Strings(ids)

	partes := []string{
		fmt.Sprintf("reconexiones_por_hora=%.2f", tasaReconexion),
		fmt.Sprintf("silencio_entrante_ms=%d", silencioMs),
		fmt.Sprintf("contactos_omitidos=%d", p.contactosOmitidos),
	}
	for _, id := range ids {
		c := p.contactos[id]
		var ratio float64
		if c.enviados > 0 {
			ratio = float64(c.acusados) / float64(c.enviados)
		}
		partes = append(partes, fmt.Sprintf("ack_ratio.%s=%.2f", id, ratio))
	}
	return strings.Join(partes, " ")
}

// Emitir escribe una única entrada de registro con Instantanea() en el campo detalle, exactamente
// la convención metricas_instantanea de adr-0024: nunca JSON, nunca un mensaje IPC nuevo.
func (p *Productor) Emitir() {
	if p.reg == nil {
		return
	}
	p.reg.Info(EventoMetricasSidecar, registro.Campos{Detalle: p.Instantanea()})
}

// Bucle emite la instantánea a intervalos regulares hasta que ctx se cancela, el mismo patrón que
// bucleDeDrenajeSalida en sidecar/main.go.
func (p *Productor) Bucle(ctx context.Context, intervalo time.Duration) {
	ticker := time.NewTicker(intervalo)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			p.Emitir()
		}
	}
}
