package metricas_test

import (
	"bytes"
	"encoding/json"
	"fmt"
	"log/slog"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/metricas"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// relojFalso es la costura de prueba: nunca duerme, nunca lee el reloj de pared, avanza solo
// cuando la prueba lo pide. Todo el paquete se prueba bajo `go test ./... -count=1`, el mismo
// perfil de compilación que corre CI.
type relojFalso struct {
	ahoraMs int64
}

func (r *relojFalso) ahora() int64 { return r.ahoraMs }
func (r *relojFalso) avanzar(ms int64) {
	r.ahoraMs += ms
}

// ultimoDetalle decodifica la última línea JSON emitida por el registro y devuelve su campo
// detalle: la superficie de aserción de todo este archivo.
func ultimoDetalle(t *testing.T, salida *bytes.Buffer) string {
	t.Helper()
	lineas := strings.Split(strings.TrimRight(salida.String(), "\n"), "\n")
	if len(lineas) == 0 || lineas[len(lineas)-1] == "" {
		t.Fatalf("no se emitió ninguna línea de registro")
	}
	var objeto map[string]any
	if err := json.Unmarshal([]byte(lineas[len(lineas)-1]), &objeto); err != nil {
		t.Fatalf("la última línea %q no es JSON válido: %v", lineas[len(lineas)-1], err)
	}
	detalle, _ := objeto[registro.CampoDetalle].(string)
	return detalle
}

func nuevoProductorDePrueba(reloj *relojFalso) (*metricas.Productor, *bytes.Buffer) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "prueba-01")
	return metricas.NuevoProductor(reg, reloj.ahora), &salida
}

// buscarClave extrae el valor de una clave=valor del payload plano, sin asumir un orden ni un
// formato distinto del que Instantanea() produce.
func buscarClave(t *testing.T, detalle, clave string) string {
	t.Helper()
	for _, parte := range strings.Split(detalle, " ") {
		if strings.HasPrefix(parte, clave+"=") {
			return strings.TrimPrefix(parte, clave+"=")
		}
	}
	t.Fatalf("no se encontró la clave %q en el detalle %q", clave, detalle)
	return ""
}

func tieneClave(detalle, clave string) bool {
	for _, parte := range strings.Split(detalle, " ") {
		if strings.HasPrefix(parte, clave+"=") {
			return true
		}
	}
	return false
}

// Escenario 1: una sola línea lleva las tres series a la vez.
// MUTACIÓN: borrar cualquiera de las tres series de Instantanea() y esta prueba debe fallar.
func TestInstantaneaLlevaLasTresSeriesALaVez(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 1_000_000}
	p, _ := nuevoProductorDePrueba(reloj)

	p.ObservarEnvio("conv-1", "corr-1")
	p.ObservarAcuse("corr-1", "entregado")
	p.ObservarEstadoSesion("activa")
	p.ObservarEntrante()
	reloj.avanzar(3_600_000)

	detalle := p.Instantanea()
	for _, clave := range []string{"reconexiones_por_hora", "silencio_entrante_ms", "contactos_omitidos", "ack_ratio.conv-1"} {
		if !tieneClave(detalle, clave) {
			t.Errorf("falta la clave %q en %q", clave, detalle)
		}
	}
}

// Escenario 2: el ack ratio va segmentado por contacto, nunca como agregado único.
// MUTACIÓN: colapsar el mapa por contacto en una sola cifra agregada y esta prueba debe fallar.
func TestAckRatioSegmentadoPorContactoNuncaAgregado(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 1_000_000}
	p, _ := nuevoProductorDePrueba(reloj)

	p.ObservarEnvio("conv-a", "corr-a1")
	p.ObservarAcuse("corr-a1", "entregado")

	p.ObservarEnvio("conv-b", "corr-b1")
	p.ObservarEnvio("conv-b", "corr-b2")
	p.ObservarAcuse("corr-b1", "entregado")
	// corr-b2 nunca se acusa: conv-b queda en 1/2 = 0.50.

	detalle := p.Instantanea()
	if v := buscarClave(t, detalle, "ack_ratio.conv-a"); v != "1.00" {
		t.Errorf("ack_ratio.conv-a = %s, se esperaba 1.00", v)
	}
	if v := buscarClave(t, detalle, "ack_ratio.conv-b"); v != "0.50" {
		t.Errorf("ack_ratio.conv-b = %s, se esperaba 0.50", v)
	}
	if tieneClave(detalle, "ack_ratio") {
		t.Errorf("no debe existir una clave ack_ratio agregada sin sufijo de contacto: %q", detalle)
	}
}

// Escenario 3: un acuse cuya correlación nunca se observó como envío (o ya fue desalojada) se
// ignora sin pánico y sin inventar un contacto fantasma.
// MUTACIÓN: quitar la comprobación de existencia y esta prueba debe fallar (o entrar en pánico).
func TestAcuseDeCorrelacionDesconocidaSeIgnoraSinContactoFantasma(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 1_000_000}
	p, _ := nuevoProductorDePrueba(reloj)

	p.ObservarAcuse("corr-nunca-vista", "entregado")

	detalle := p.Instantanea()
	if tieneClave(detalle, "ack_ratio.corr-nunca-vista") {
		t.Errorf("no debía crearse ningún contacto a partir de un acuse huérfano: %q", detalle)
	}
	// contactos_omitidos no debe moverse por un acuse ignorado: no es una eviction.
	if v := buscarClave(t, detalle, "contactos_omitidos"); v != "0" {
		t.Errorf("contactos_omitidos = %s, se esperaba 0", v)
	}
}

// Escenario 4: reconexiones_por_hora nace de las transiciones HACIA conectado, no de cada estado
// conectado repetido.
// MUTACIÓN: contar cada estado en vez de la transición y esta prueba debe fallar.
func TestReconexionesPorHoraCuentaTransicionesNoEstadosRepetidos(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 0}
	p, _ := nuevoProductorDePrueba(reloj)

	p.ObservarEstadoSesion("activa")
	p.ObservarEstadoSesion("activa")
	p.ObservarEstadoSesion("activa")
	reloj.avanzar(3_600_000) // una hora exacta

	if v := buscarClave(t, p.Instantanea(), "reconexiones_por_hora"); v != "1.00" {
		t.Errorf("reconexiones_por_hora = %s, se esperaba 1.00 (una sola transición)", v)
	}

	p.ObservarEstadoSesion("pausada")
	p.ObservarEstadoSesion("activa")
	// Ahora dos transiciones reales en dos horas transcurridas -> 1.00 por hora otra vez.
	reloj.avanzar(3_600_000)
	if v := buscarClave(t, p.Instantanea(), "reconexiones_por_hora"); v != "1.00" {
		t.Errorf("reconexiones_por_hora = %s, se esperaba 1.00 tras la segunda transición real", v)
	}
}

// Escenario 5: la ventana de silencio entrante crece con el reloj inyectado y vuelve a cero con
// ObservarEntrante().
// MUTACIÓN: no reiniciar la marca y esta prueba debe fallar.
func TestSilencioEntranteCreceYSeReiniciaConObservarEntrante(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 0}
	p, _ := nuevoProductorDePrueba(reloj)

	reloj.avanzar(5_000)
	if v := buscarClave(t, p.Instantanea(), "silencio_entrante_ms"); v != "5000" {
		t.Errorf("silencio_entrante_ms = %s, se esperaba 5000", v)
	}

	p.ObservarEntrante()
	if v := buscarClave(t, p.Instantanea(), "silencio_entrante_ms"); v != "0" {
		t.Errorf("silencio_entrante_ms = %s, se esperaba 0 justo tras ObservarEntrante", v)
	}

	reloj.avanzar(2_500)
	if v := buscarClave(t, p.Instantanea(), "silencio_entrante_ms"); v != "2500" {
		t.Errorf("silencio_entrante_ms = %s, se esperaba 2500", v)
	}
}

// Escenario 6: el desalojo de contactos elige, entre 257 contactos distintos, al de actividad más
// antigua, incrementa contactos_omitidos y deja exactamente el conjunto esperado.
// MUTACIÓN: elegir cualquier otra víctima (la más nueva, al azar, por orden de iteración del
// mapa) y esta prueba debe fallar.
func TestDesalojoDeContactoElimaAlDeActividadMasAntigua(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 0}
	p, _ := nuevoProductorDePrueba(reloj)

	for i := 0; i < metricas.MaximoContactos; i++ {
		reloj.avanzar(1)
		p.ObservarEnvio(fmt.Sprintf("conv-%03d", i), fmt.Sprintf("corr-%03d", i))
	}
	// conv-000 es ahora la de actividad más antigua (marca de tiempo 1); el 257º contacto debe
	// desalojarla.
	reloj.avanzar(1)
	p.ObservarEnvio("conv-nueva", "corr-nueva")

	detalle := p.Instantanea()
	if tieneClave(detalle, "ack_ratio.conv-000") {
		t.Errorf("conv-000 debía haber sido desalojada por ser la de actividad más antigua: %q", detalle)
	}
	if !tieneClave(detalle, "ack_ratio.conv-001") {
		t.Errorf("conv-001 debía sobrevivir: %q", detalle)
	}
	if !tieneClave(detalle, "ack_ratio.conv-nueva") {
		t.Errorf("conv-nueva debía existir tras insertarse: %q", detalle)
	}
	if v := buscarClave(t, detalle, "contactos_omitidos"); v != "1" {
		t.Errorf("contactos_omitidos = %s, se esperaba 1", v)
	}
}

// Escenario 7: con dos contactos de idéntica marca de actividad, el desempate es el id ascendente,
// de forma determinista y repetible.
// MUTACIÓN: invertir el desempate a descendente y esta prueba debe fallar.
func TestDesalojoDeContactoDesempataPorIdAscendente(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 500}
	p, _ := nuevoProductorDePrueba(reloj)

	// Todos comparten exactamente la misma marca de actividad: el reloj no avanza entre envíos.
	for i := 0; i < metricas.MaximoContactos; i++ {
		p.ObservarEnvio(fmt.Sprintf("conv-%03d", i), fmt.Sprintf("corr-%03d", i))
	}
	p.ObservarEnvio("conv-nueva", "corr-nueva")

	detalle := p.Instantanea()
	// conv-000 es el id ascendente más bajo entre los empatados: debe ser la víctima.
	if tieneClave(detalle, "ack_ratio.conv-000") {
		t.Errorf("conv-000 (id ascendente más bajo) debía ser la víctima del desempate: %q", detalle)
	}
	if !tieneClave(detalle, "ack_ratio.conv-001") {
		t.Errorf("conv-001 debía sobrevivir al desempate: %q", detalle)
	}
}

// Escenario 8: el desalojo de correlaciones por encima de 1024 también incrementa
// contactos_omitidos, para que el truncamiento sea observable y nunca silencioso, y la víctima
// desalojada es demostrablemente la más antigua (corr-0000), no una cualquiera.
// MUTACIÓN: desalojar sin incrementar el contador, o desalojar la correlación equivocada
// (más nueva primero), y esta prueba debe fallar.
//
// corr-0000 nace en su propio contacto (conv-vieja) precisamente para que el resultado sea
// numéricamente distinguible: con un único contacto para las 1025 correlaciones, 1/1025 también
// redondea a 0.00 bajo %.2f, así que desalojar la víctima correcta o la incorrecta producían el
// mismo texto. Aislar corr-0000 en su propio contacto convierte el desenlace en 0.00 (desalojada)
// frente a 1.00 (sobrevivió), que sí se distinguen.
func TestDesalojoDeCorrelacionIncrementaContactosOmitidosYDesalojaLaMasAntigua(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 0}
	p, _ := nuevoProductorDePrueba(reloj)

	// corr-0000 es la primera y, por tanto, la de creación más antigua.
	p.ObservarEnvio("conv-vieja", "corr-0000")
	// El resto de los envíos, hasta completar MaximoCorrelaciones+1 en total, se apilan en un
	// segundo contacto sin que ninguno se acuse todavía: las correlaciones se acumulan sin
	// resolverse y fuerzan el desalojo por su propio límite, no por el de contactos.
	for i := 1; i <= metricas.MaximoCorrelaciones; i++ {
		reloj.avanzar(1)
		p.ObservarEnvio("conv-unica", fmt.Sprintf("corr-%04d", i))
	}

	detalle := p.Instantanea()
	if v := buscarClave(t, detalle, "contactos_omitidos"); v != "1" {
		t.Errorf("contactos_omitidos = %s, se esperaba 1 tras desalojar una correlación", v)
	}

	// La correlación más antigua (corr-0000) ya no debe poder resolver un acuse: si de verdad
	// fue desalojada, conv-vieja se queda en 0.00; si sobrevivió (por ejemplo, por un desalojo
	// que elige la más nueva primero), el acuse la confirmaría y conv-vieja pasaría a 1.00.
	p.ObservarAcuse("corr-0000", "entregado")
	if v := buscarClave(t, p.Instantanea(), "ack_ratio.conv-vieja"); v != "0.00" {
		t.Errorf("ack_ratio.conv-vieja = %s, se esperaba 0.00: corr-0000 debía estar desalojada", v)
	}
}

// Escenario 12: con correlaciones de idéntica marca de creación, el desempate del desalojo es el
// id ascendente, igual que en el desalojo de contactos (escenario 7).
// MUTACIÓN: invertir el desempate a descendente y esta prueba debe fallar.
//
// corr-0000 nace en su propio contacto (conv-vieja) por la misma razón que en el escenario 8:
// aislarla vuelve el desenlace distinguible (0.00 desalojada frente a 1.00 sobreviviente) en vez
// de diluirse en un ack ratio compartido con las demás correlaciones empatadas.
func TestDesalojoDeCorrelacionDesempataPorIdAscendente(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 700}
	p, _ := nuevoProductorDePrueba(reloj)

	// corr-0000 es el id ascendente más bajo de todo el lote.
	p.ObservarEnvio("conv-vieja", "corr-0000")
	// El resto, hasta completar MaximoCorrelaciones en total, comparte exactamente la misma
	// marca de creación: el reloj no avanza entre envíos.
	for i := 1; i < metricas.MaximoCorrelaciones; i++ {
		p.ObservarEnvio("conv-unica", fmt.Sprintf("corr-%04d", i))
	}
	// Fuerza el desalojo: todas las correlaciones existentes están empatadas en marca de
	// actividad, así que el desempate por id ascendente decide, y corr-0000 es el id más bajo.
	p.ObservarEnvio("conv-unica", "corr-nueva")

	p.ObservarAcuse("corr-0000", "entregado")
	if v := buscarClave(t, p.Instantanea(), "ack_ratio.conv-vieja"); v != "0.00" {
		t.Errorf("ack_ratio.conv-vieja = %s, se esperaba 0.00: corr-0000 (id ascendente más bajo) debía ser la víctima del desempate", v)
	}
}

// Escenario 9: la línea emitida es una única entrada de registro cuyo evento es la constante fija
// y cuyo payload es una cadena clave=valor en el campo detalle, nunca JSON ni un tipo IPC nuevo.
// MUTACIÓN: emitir JSON en detalle y esta prueba debe fallar.
func TestEmitirEscribeUnaLineaConEventoFijoYDetallePlano(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 1_000}
	p, salida := nuevoProductorDePrueba(reloj)
	p.ObservarEnvio("conv-1", "corr-1")

	p.Emitir()

	lineas := strings.Split(strings.TrimRight(salida.String(), "\n"), "\n")
	if len(lineas) != 1 {
		t.Fatalf("se esperaba exactamente 1 línea emitida, salieron %d", len(lineas))
	}
	var objeto map[string]any
	if err := json.Unmarshal([]byte(lineas[0]), &objeto); err != nil {
		t.Fatalf("la línea de registro no es JSON válido (el propio registro siempre lo es): %v", err)
	}
	if objeto[registro.CampoEvento] != metricas.EventoMetricasSidecar {
		t.Errorf("evento = %v, se esperaba %q", objeto[registro.CampoEvento], metricas.EventoMetricasSidecar)
	}
	detalle, esCadena := objeto[registro.CampoDetalle].(string)
	if !esCadena {
		t.Fatalf("detalle no es una cadena: %v", objeto[registro.CampoDetalle])
	}
	// El payload en sí, dentro de detalle, no debe parecer JSON: nunca empieza por '{' ni '['.
	if strings.HasPrefix(strings.TrimSpace(detalle), "{") || strings.HasPrefix(strings.TrimSpace(detalle), "[") {
		t.Errorf("detalle parece JSON en vez de key=value: %q", detalle)
	}
	if !strings.Contains(detalle, "=") {
		t.Errorf("detalle no contiene ningún par clave=valor: %q", detalle)
	}
}

// Escenario 10: la clave por contacto es el id_conversacion interno, nunca un JID crudo de
// transporte: un valor con forma de JID nunca se acepta como clave de contacto desde la propia
// superficie de la API del productor.
// MUTACIÓN: indexar el mapa por un JID y esta prueba debe fallar.
func TestClaveDeContactoNuncaAceptaUnJIDCrudo(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 0}
	p, _ := nuevoProductorDePrueba(reloj)

	const jid = "5215500000000@s.whatsapp.net"
	p.ObservarEnvio(jid, "corr-jid")

	detalle := p.Instantanea()
	if strings.Contains(detalle, jid) || strings.Contains(detalle, "@s.whatsapp.net") {
		t.Fatalf("un JID crudo llegó a la línea emitida, viola la frontera de privacidad de adr-0019: %q", detalle)
	}
	if tieneClave(detalle, "ack_ratio."+jid) {
		t.Fatalf("se creó un contacto indexado por JID: %q", detalle)
	}
}

// Escenario 11: Instantanea() renderiza los contactos en orden ascendente de id, así que dos
// llamadas sobre el mismo estado producen exactamente el mismo texto.
// MUTACIÓN: iterar el mapa de Go directamente sin ordenar y esta prueba debe fallar bajo
// -count=1 por la aleatoriedad de orden que Go introduce a propósito en sus mapas.
func TestInstantaneaEsDeterministaEntreLlamadasRepetidas(t *testing.T) {
	t.Parallel()
	reloj := &relojFalso{ahoraMs: 0}
	p, _ := nuevoProductorDePrueba(reloj)

	for i := 0; i < 40; i++ {
		p.ObservarEnvio(fmt.Sprintf("conv-%03d", i), fmt.Sprintf("corr-%03d", i))
	}

	primera := p.Instantanea()
	for intento := 0; intento < 20; intento++ {
		if segunda := p.Instantanea(); segunda != primera {
			t.Fatalf("Instantanea() no es determinista entre llamadas:\n%q\n%q", primera, segunda)
		}
	}
}

// Bucle en sí es un temporizador real (time.Ticker) y no se ejerce aquí con reloj de pared ni
// sleeps: el contrato de esta tarea prohíbe expresamente esa dependencia en las pruebas
// ("nunca reloj de pared, nunca sleeps"). Su única lógica propia -emitir en cada tick y volver al
// cancelarse ctx- es la misma que ya cubre bucleDeDrenajeSalida en sidecar/main.go sin prueba
// dedicada; Emitir() e Instantanea(), que sí concentran la lógica de negocio, quedan cubiertas
// arriba de forma íntegra y determinista.
