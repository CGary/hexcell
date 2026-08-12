package outbox_test

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	_ "modernc.org/sqlite"
)

func abrirDbPruebaSalida(t *testing.T) (*sql.DB, string) {
	ruta := filepath.Join(t.TempDir(), "test_salida.db")
	dsn := "file:" + ruta + "?_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("error abriendo bd: %v", err)
	}
	esquema := `
	CREATE TABLE IF NOT EXISTS cola_salida (
		id_mensaje TEXT UNIQUE,
		id_conversacion TEXT,
		contenido TEXT,
		marca_temporal_origen_ms INTEGER,
		intentos INTEGER DEFAULT 0,
		id_correlacion TEXT DEFAULT '',
		enviado_en_ms INTEGER NULL
	);
	CREATE TABLE IF NOT EXISTS volumen_diario (
		dia TEXT PRIMARY KEY,
		envios INTEGER DEFAULT 0
	);
	`
	if _, err := db.Exec(esquema); err != nil {
		t.Fatalf("error creando esquema: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	return db, dsn
}

// transmisorFalso es un doble de outbox.Transmisor para ejercitar el motor de drenaje sin
// ninguna dependencia de whatsmeow: cuenta cuántas veces se le llamó y, según fallar, siempre
// tiene éxito o siempre falla, que es lo único que el motor necesita distinguir.
type transmisorFalso struct {
	mu            sync.Mutex
	llamadas      int
	fallar        bool
	idCorrelacion string
}

func (t *transmisorFalso) Transmitir(_ context.Context, _, _ string) (string, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.llamadas++
	if t.fallar {
		return "", errors.New("fallo simulado de transporte")
	}
	return t.idCorrelacion, nil
}

func (t *transmisorFalso) contador() int {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.llamadas
}

type emisorPresenciaFalso struct {
	mu       sync.Mutex
	llamadas int
	fallar   bool
}

func (e *emisorPresenciaFalso) EmitirEscribiendo(_ context.Context, _ string) error {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.llamadas++
	if e.fallar {
		return errors.New("fallo simulado emisor presencia")
	}
	return nil
}

func (e *emisorPresenciaFalso) contador() int {
	e.mu.Lock()
	defer e.mu.Unlock()
	return e.llamadas
}

func TestEncolarEsIdempotente(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	ctx := context.Background()

	err := cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)
	if err != nil {
		t.Fatalf("error encolando: %v", err)
	}

	// Re-encolar ignora el conflicto silenciosamente (ON CONFLICT DO NOTHING)
	err = cola.Encolar(ctx, "msg-1", "conv-2", "adios", 200)
	if err != nil {
		t.Fatalf("error re-encolando: %v", err)
	}

	var conv string
	db.QueryRow("SELECT id_conversacion FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&conv)
	if conv != "conv-1" {
		t.Fatalf("se sobreescribió la fila existente: %s", conv)
	}
}

func TestDrenarExpiradosBasadoEnOrigen(t *testing.T) {
	t.Parallel()
	db, dsn := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	ctx := context.Background()

	// origen = 100, TTL = 1000. Expira si ahoraMs - 100 > 1000 => ahoraMs > 1100
	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)

	// A los 1100ms exactos, 1100 - 100 = 1000. No es estrictamente mayor que el TTL, así que no expira.
	cola.Drenar(ctx, 1100)
	var count int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&count)
	if count != 1 {
		t.Fatalf("esperaba 1, hay %d", count)
	}

	// A los 1101ms, 1101 - 100 = 1001 > 1000. Expira y se descarta físicamente (hard discard).
	valorPrevio := outbox.ContadorExpiradas.Load()
	cola.Drenar(ctx, 1101)
	db.QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&count)
	if count != 0 {
		t.Fatalf("esperaba 0, hay %d", count)
	}

	if outbox.ContadorExpiradas.Load() <= valorPrevio {
		t.Fatalf("no se incrementó el contador de expiradas")
	}

	// Comprobar que el veredicto de expiración es idéntico tras reabrir la base de datos,
	// demostrando que no deriva del tiempo de ejecución del proceso.
	db2, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("error reabriendo bd: %v", err)
	}
	defer db2.Close()
	cola2 := outbox.NuevaColaDeSalida(db2, 1000, 3, nil, nil, nil)

	cola2.Encolar(ctx, "msg-2", "conv-1", "hola", 200)
	cola2.Drenar(ctx, 1201)
	db2.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-2'").Scan(&count)
	if count != 0 {
		t.Fatalf("esperaba 0 tras reabrir, hay %d", count)
	}
}

func TestDescartarExpiradosRegistraUnaLineaPorMensaje(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, reg, nil, nil)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)
	cola.Encolar(ctx, "msg-2", "conv-1", "hola", 100)

	if err := cola.Drenar(ctx, 1101); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	var lineasDeExpiracion []string
	for _, linea := range strings.Split(strings.TrimSpace(buf.String()), "\n") {
		if strings.Contains(linea, outbox.EventoSalidaExpirada) {
			lineasDeExpiracion = append(lineasDeExpiracion, linea)
		}
	}

	// Una línea POR MENSAJE, no un conteo agregado: con dos mensajes expirados debe haber dos
	// líneas, cada una identificando su propio id_mensaje en el campo id_evento.
	if len(lineasDeExpiracion) != 2 {
		t.Fatalf("se esperaba una línea de registro por mensaje expirado (2), hubo %d: %v", len(lineasDeExpiracion), lineasDeExpiracion)
	}
	if !strings.Contains(lineasDeExpiracion[0]+lineasDeExpiracion[1], `"id_evento":"msg-1"`) {
		t.Fatalf("ninguna línea identifica msg-1: %v", lineasDeExpiracion)
	}
	if !strings.Contains(lineasDeExpiracion[0]+lineasDeExpiracion[1], `"id_evento":"msg-2"`) {
		t.Fatalf("ninguna línea identifica msg-2: %v", lineasDeExpiracion)
	}
}

func TestMarcarEnviadoEsIdempotente(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)
	err := cola.MarcarEnviado(ctx, "msg-1", "corr-1", 200)
	if err != nil {
		t.Fatalf("error marcando enviado: %v", err)
	}

	err = cola.MarcarEnviado(ctx, "msg-1", "corr-2", 300)
	if err != nil {
		t.Fatalf("error re-marcando enviado: %v", err)
	}

	var corr string
	db.QueryRow("SELECT id_correlacion FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&corr)
	if corr != "corr-1" {
		t.Fatalf("se sobreescribió id_correlacion: %s", corr)
	}
}

func TestDrenarTransmiteYMarcaEnviado(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-exito"}
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, falso, nil)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)

	if err := cola.Drenar(ctx, 500); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}
	if falso.contador() != 1 {
		t.Fatalf("se esperaba exactamente 1 llamada al transmisor, hubo %d", falso.contador())
	}

	var enviadoEn sql.NullInt64
	var idCorrelacion string
	db.QueryRow("SELECT enviado_en_ms, id_correlacion FROM cola_salida WHERE id_mensaje='msg-1'").
		Scan(&enviadoEn, &idCorrelacion)
	if !enviadoEn.Valid {
		t.Fatal("el mensaje transmitido con éxito debía quedar marcado como enviado")
	}
	if idCorrelacion != "corr-exito" {
		t.Fatalf("id_correlacion incorrecto: %q", idCorrelacion)
	}

	// Idempotencia de un reintento repetido: un segundo drenaje no debe volver a transmitir un
	// mensaje que ya quedó marcado como enviado.
	if err := cola.Drenar(ctx, 600); err != nil {
		t.Fatalf("error al re-drenar: %v", err)
	}
	if falso.contador() != 1 {
		t.Fatalf("un mensaje ya enviado no debía reintentarse, llamadas=%d", falso.contador())
	}
}

func TestDrenarReintentaHastaElLimiteYAbandonaSinColaDeReenvio(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{fallar: true}
	// TTL deliberadamente amplio: lo que termina esta fila es el agotamiento de intentos, no la
	// expiración, y las dos rutas de descarte duro no deben confundirse en la prueba.
	cola := outbox.NuevaColaDeSalida(db, 1_000_000, 2, nil, falso, nil)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)

	if err := cola.Drenar(ctx, 200); err != nil {
		t.Fatalf("error al drenar (intento 1): %v", err)
	}
	var cuenta int
	var intentos int64
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&cuenta)
	if cuenta != 1 {
		t.Fatalf("tras el primer fallo la fila debía seguir viva, cuenta=%d", cuenta)
	}
	db.QueryRow("SELECT intentos FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&intentos)
	if intentos != 1 {
		t.Fatalf("se esperaba intentos=1 tras el primer fallo, hay %d", intentos)
	}

	if err := cola.Drenar(ctx, 300); err != nil {
		t.Fatalf("error al drenar (intento 2): %v", err)
	}
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("al alcanzar el límite de intentos la fila debía desaparecer sin dejar rastro (sin cola de reenvío), cuenta=%d", cuenta)
	}
	if falso.contador() != 2 {
		t.Fatalf("se esperaban exactamente 2 llamadas al transmisor antes de abandonar, hubo %d", falso.contador())
	}

	// Un tercer drenaje no encuentra nada que reintentar: el mensaje ya fue abandonado.
	if err := cola.Drenar(ctx, 400); err != nil {
		t.Fatalf("error al drenar (intento 3): %v", err)
	}
	if falso.contador() != 2 {
		t.Fatalf("un mensaje ya abandonado no debía volver a intentarse, llamadas=%d", falso.contador())
	}
}

func TestDrenarNuncaTransmiteUnMensajeYaExpirado(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "no-deberia-usarse"}
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, falso, nil)
	ctx := context.Background()

	// origen=100, TTL=1000: a partir de ahoraMs=1101 el mensaje ya expiró (1101-100=1001>1000).
	// El TTL debe ganar la carrera contra cualquier intento de reintento.
	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)

	if err := cola.Drenar(ctx, 1101); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	if falso.contador() != 0 {
		t.Fatalf("un mensaje expirado nunca debe llegar al transmisor, llamadas=%d", falso.contador())
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("el mensaje expirado debía descartarse igualmente, cuenta=%d", cuenta)
	}
}

// clienteWhatsmeowFalso es un doble de outbox.ClienteWhatsmeow: registra con qué destino y
// texto se le llamó, sin abrir ninguna sesión real de whatsmeow.
type clienteWhatsmeowFalso struct {
	mu            sync.Mutex
	llamadas      int
	ultimoDestino types.JID
	ultimoTexto   string
	respuesta     whatsmeow.SendResponse
	err           error
}

func (c *clienteWhatsmeowFalso) SendMessage(_ context.Context, to types.JID, message *waE2E.Message, _ ...whatsmeow.SendRequestExtra) (whatsmeow.SendResponse, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.llamadas++
	c.ultimoDestino = to
	c.ultimoTexto = message.GetConversation()
	return c.respuesta, c.err
}

func (c *clienteWhatsmeowFalso) contador() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.llamadas
}

// resolutorDeDireccionesFalso es un doble de outbox.ResolutorDeDirecciones.
type resolutorDeDireccionesFalso struct {
	direccion types.JID
	err       error
}

func (r resolutorDeDireccionesFalso) DireccionDe(_ context.Context, _ string) (types.JID, error) {
	return r.direccion, r.err
}

func TestTransmisorWhatsmeowResuelveDireccionYEnviaSoloElTexto(t *testing.T) {
	t.Parallel()
	destino := types.JID{User: "5491100000000", Server: "s.whatsapp.net"}
	// Sender deliberadamente distinto del destino: la prueba comprueba que ese Sender JID de la
	// respuesta nunca sale de Transmitir, solo el ID de correlación.
	cliente := &clienteWhatsmeowFalso{
		respuesta: whatsmeow.SendResponse{ID: "wamid-1", Sender: types.JID{User: "otro-remitente", Server: "s.whatsapp.net"}},
	}
	resolutor := resolutorDeDireccionesFalso{direccion: destino}

	transmisor := outbox.NuevoTransmisorWhatsmeow(cliente, resolutor)
	idCorrelacion, err := transmisor.Transmitir(context.Background(), "conv-interna-1", "hola mundo")
	if err != nil {
		t.Fatalf("error inesperado: %v", err)
	}
	if idCorrelacion != "wamid-1" {
		t.Fatalf("id de correlación incorrecto: %q", idCorrelacion)
	}
	if cliente.ultimoDestino != destino {
		t.Fatalf("no se envió a la dirección resuelta: %v", cliente.ultimoDestino)
	}
	if cliente.ultimoTexto != "hola mundo" {
		t.Fatalf("contenido incorrecto: %q", cliente.ultimoTexto)
	}
}

func TestTransmisorWhatsmeowPropagaErrorDeResolucionSinLlamarAlCliente(t *testing.T) {
	t.Parallel()
	resolutor := resolutorDeDireccionesFalso{err: errors.New("sin dirección conocida")}
	cliente := &clienteWhatsmeowFalso{}
	transmisor := outbox.NuevoTransmisorWhatsmeow(cliente, resolutor)

	_, err := transmisor.Transmitir(context.Background(), "conv-desconocida", "hola")
	if err == nil {
		t.Fatal("se esperaba un error de resolución")
	}
	if cliente.contador() != 0 {
		t.Fatalf("no debía llamarse a SendMessage sin dirección resuelta, llamadas=%d", cliente.contador())
	}
}

func TestTransmisorWhatsmeowPropagaErrorDeEnvio(t *testing.T) {
	t.Parallel()
	resolutor := resolutorDeDireccionesFalso{direccion: types.JID{User: "5491100000000", Server: "s.whatsapp.net"}}
	cliente := &clienteWhatsmeowFalso{err: errors.New("fallo de red simulado")}
	transmisor := outbox.NuevoTransmisorWhatsmeow(cliente, resolutor)

	_, err := transmisor.Transmitir(context.Background(), "conv-1", "hola")
	if err == nil {
		t.Fatal("se esperaba que el error de SendMessage se propagara")
	}
}

func TestDrenarDescartaMensajeEncoladoPrevioALaBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "no-debe-enviarse"}
	control := &controlDeBajaEspia{permitido: false}
	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, reg, falso, control)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-previo-1", "conv-1", "hola previo", 100)

	descartadasPrevias := outbox.ContadorDescartadasPorBaja.Load()
	if err := cola.Drenar(ctx, 200); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	if falso.contador() != 0 {
		t.Fatalf("un mensaje para contacto dado de baja nunca debe transmitirse, llamadas=%d", falso.contador())
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-previo-1'").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("el mensaje descartado por baja debía eliminarse con dureza, cuenta=%d", cuenta)
	}

	if outbox.ContadorDescartadasPorBaja.Load() <= descartadasPrevias {
		t.Errorf("no se incrementó ContadorDescartadasPorBaja")
	}

	if !strings.Contains(buf.String(), outbox.EventoSalidaDescartadaPorBaja) {
		t.Errorf("no se registró EventoSalidaDescartadaPorBaja: %s", buf.String())
	}
}

// cortacircuitosConExencion simula la semántica real de identidad.Almacen.SalidaPermitida: antes
// de disparado permite todo; una vez disparado (campo mutado por la prueba, como lo mutaría un
// disparo real persistido entre dos llamadas a Drenar) solo permite el id_mensaje_traspaso
// reclamado.
type cortacircuitosConExencion struct {
	disparado           bool
	idTraspasoPermitido string
}

func (c *cortacircuitosConExencion) SalidaPermitida(_ context.Context, _, idMensaje string) (bool, error) {
	if !c.disparado {
		return true, nil
	}
	return idMensaje == c.idTraspasoPermitido, nil
}

func (c *cortacircuitosConExencion) ReclamarMensajeDeTraspaso(_ context.Context, _, _ string, _ int64) (bool, error) {
	return false, nil
}

// cortacircuitosConError simula un ControlDeCortacircuitos cuya consulta en el drenaje falla,
// para probar el fallo cerrado (aplazar, nunca descartar ni transmitir).
type cortacircuitosConError struct{}

func (cortacircuitosConError) SalidaPermitida(_ context.Context, _, _ string) (bool, error) {
	return false, errors.New("fallo simulado de lectura del cortacircuitos")
}

func (cortacircuitosConError) ReclamarMensajeDeTraspaso(_ context.Context, _, _ string, _ int64) (bool, error) {
	return false, nil
}

// TestDrenarDescartaFilaEncoladaAntesDeDispararseCortacircuitos prueba el fix
// cortacircuitos-descarte-en-drenaje: una respuesta admitida ANTES del disparo (y todavía
// encolada al dispararse) debe descartarse en el drenaje en vez de transmitirse después del
// traspaso, y el marcador de presencia anunciada debe liberarse en ese descarte.
func TestDrenarDescartaFilaEncoladaAntesDeDispararseCortacircuitos(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-traspaso"}
	emisor := &emisorPresenciaFalso{}
	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: time.UTC,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	corta := &cortacircuitosConExencion{idTraspasoPermitido: "msg-traspaso-1"}
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, reg, falso, nil).ConDisciplina(disc, emisor).ConCortacircuitos(corta)
	ctx := context.Background()

	// La respuesta se admite ANTES del disparo: nada del cortacircuitos existía todavía.
	cola.Encolar(ctx, "msg-reply-1", "conv-1", "respuesta previa al disparo", 1000)

	// t=1500: aplazada por latencia mínima (el disparo aún no ocurrió), queda el marcador de
	// presencia en memoria.
	if err := cola.Drenar(ctx, 1500); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}
	if falso.contador() != 0 {
		t.Fatalf("no debía transmitirse antes de cumplirse la latencia, llamadas=%d", falso.contador())
	}
	if cola.PresenciasPendientesParaPruebas() != 1 {
		t.Fatalf("el marcador de presencia debía quedar activo tras el aplazamiento, obtenido %d", cola.PresenciasPendientesParaPruebas())
	}

	// El cortacircuitos se dispara mientras tanto y se encola el único traspaso permitido.
	corta.disparado = true
	cola.Encolar(ctx, "msg-traspaso-1", "conv-1", "handoff", 1400)

	descartadasPrevias := outbox.ContadorDescartadasPorCortacircuitos.Load()

	// t=4500: se cumple la latencia; msg-reply-1 debe descartarse por el disparo (bot en
	// silencio), msg-traspaso-1 debe transmitirse (única excepción permitida).
	if err := cola.Drenar(ctx, 4500); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	if falso.contador() != 1 {
		t.Fatalf("solo el traspaso debía transmitirse, llamadas=%d", falso.contador())
	}

	var cuentaReply int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-reply-1'").Scan(&cuentaReply)
	if cuentaReply != 0 {
		t.Fatalf("la respuesta admitida antes del disparo debía descartarse con dureza, cuenta=%d", cuentaReply)
	}

	var enviadoTraspaso sql.NullInt64
	db.QueryRow("SELECT enviado_en_ms FROM cola_salida WHERE id_mensaje='msg-traspaso-1'").Scan(&enviadoTraspaso)
	if !enviadoTraspaso.Valid {
		t.Fatal("el traspaso debía quedar marcado como enviado")
	}

	if outbox.ContadorDescartadasPorCortacircuitos.Load() <= descartadasPrevias {
		t.Errorf("no se incrementó ContadorDescartadasPorCortacircuitos")
	}
	if !strings.Contains(buf.String(), outbox.EventoSalidaDescartadaPorCortacircuitos) {
		t.Errorf("no se registró EventoSalidaDescartadaPorCortacircuitos: %s", buf.String())
	}
	if cola.PresenciasPendientesParaPruebas() != 0 {
		t.Errorf("el marcador de presencia debía liberarse al descartar la respuesta, quedaron %d entradas", cola.PresenciasPendientesParaPruebas())
	}
}

// TestDrenarAplazaFilaPendienteSiFallaLaConsultaDeCortacircuitos prueba el fallo cerrado del
// recheque en el drenaje: un error al leer el estado del cortacircuitos nunca transmite ni
// descarta la fila -la aplaza, como verificarDisciplina aplaza ante un error de rampa.
func TestDrenarAplazaFilaPendienteSiFallaLaConsultaDeCortacircuitos(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "no-debe-enviarse"}
	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, reg, falso, nil).ConCortacircuitos(cortacircuitosConError{})
	ctx := context.Background()

	cola.Encolar(ctx, "msg-1", "conv-1", "hola", 100)

	previas := outbox.ContadorAplazadasPorErrorCortacircuitos.Load()
	if err := cola.Drenar(ctx, 200); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	if falso.contador() != 0 {
		t.Fatalf("no debía transmitirse ante un error de lectura del cortacircuitos, llamadas=%d", falso.contador())
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&cuenta)
	if cuenta != 1 {
		t.Fatalf("ante error de lectura la fila debe conservarse (fallo cerrado, aplazada, no descartada), cuenta=%d", cuenta)
	}

	if outbox.ContadorAplazadasPorErrorCortacircuitos.Load() <= previas {
		t.Errorf("no se incrementó ContadorAplazadasPorErrorCortacircuitos")
	}
	if !strings.Contains(buf.String(), "outbox.error_consultar_cortacircuitos_en_drenaje") {
		t.Errorf("no se registró el evento de error de consulta: %s", buf.String())
	}
}

type controlDeBajaConExencion struct {
	idConfirmacionPermitida string
}

func (c *controlDeBajaConExencion) EnvioPermitido(_ context.Context, _, idMensaje string) (bool, error) {
	return idMensaje == c.idConfirmacionPermitida, nil
}

func (c *controlDeBajaConExencion) ReclamarConfirmacionDeBaja(_ context.Context, _, _ string, _ int64) (bool, error) {
	return false, nil
}

func TestDrenarTransmiteConfirmacionDeBajaAContactoDadoDeBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-conf"}
	control := &controlDeBajaConExencion{idConfirmacionPermitida: "msg-conf-1"}
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, falso, control)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-ordinario", "conv-1", "hola ordinario", 100)
	cola.Encolar(ctx, "msg-conf-1", "conv-1", "confirmacion", 100)

	if err := cola.Drenar(ctx, 200); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	if falso.contador() != 1 {
		t.Fatalf("se esperaba exactamente 1 transmisión (la confirmación), hubo %d", falso.contador())
	}

	var enviadoEn sql.NullInt64
	db.QueryRow("SELECT enviado_en_ms FROM cola_salida WHERE id_mensaje='msg-conf-1'").Scan(&enviadoEn)
	if !enviadoEn.Valid {
		t.Fatal("la confirmación debía quedar marcada como enviada")
	}

	var cuentaOrdinario int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-ordinario'").Scan(&cuentaOrdinario)
	if cuentaOrdinario != 0 {
		t.Fatalf("el mensaje ordinario no permitido debía eliminarse")
	}
}

func TestDrenarAplicaLatenciaMinima(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-1"}
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: time.UTC,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, nil, falso, nil).ConDisciplina(disc, nil)
	ctx := context.Background()

	// Encolado en t=1000
	cola.Encolar(ctx, "msg-lat", "conv-1", "hola", 1000)

	// Drenar en t=2500 (transcurrieron 1500 ms < 3000 ms): debe quedar pendiente sin incrementar intentos
	prevAplazadas := outbox.ContadorAplazadasPorLatencia.Load()
	if err := cola.Drenar(ctx, 2500); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}
	if falso.contador() != 0 {
		t.Errorf("no debía transmitirse antes del suelo de latencia, llamadas=%d", falso.contador())
	}
	if outbox.ContadorAplazadasPorLatencia.Load() <= prevAplazadas {
		t.Errorf("ContadorAplazadasPorLatencia no se incrementó")
	}

	var intentos int64
	var enviadoEn sql.NullInt64
	db.QueryRow("SELECT intentos, enviado_en_ms FROM cola_salida WHERE id_mensaje='msg-lat'").Scan(&intentos, &enviadoEn)
	if intentos != 0 {
		t.Errorf("los intentos no debían incrementarse por aplazamiento, intentos=%d", intentos)
	}
	if enviadoEn.Valid {
		t.Errorf("el mensaje no debía marcarse como enviado")
	}

	// Drenar en t=4000 (transcurrieron 3000 ms >= 3000 ms): debe transmitirse
	if err := cola.Drenar(ctx, 4000); err != nil {
		t.Fatalf("error al drenar en t=4000: %v", err)
	}
	if falso.contador() != 1 {
		t.Errorf("debía transmitirse tras cumplirse la latencia, llamadas=%d", falso.contador())
	}
	db.QueryRow("SELECT enviado_en_ms FROM cola_salida WHERE id_mensaje='msg-lat'").Scan(&enviadoEn)
	if !enviadoEn.Valid {
		t.Errorf("debía quedar marcado como enviado")
	}
}

func TestDrenarAplicaVentanaDeAtencion(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-1"}
	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 1000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 9, MinutoApertura: 0,
			HoraCierre: 19, MinutoCierre: 0,
			Dias: []int{1, 2, 3, 4, 5},
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 86400000, 3, nil, falso, nil).ConDisciplina(disc, nil)
	ctx := context.Background()

	// Martes 2026-08-11 08:00 BA (11:00 UTC)
	tFuera := time.Date(2026, 8, 11, 11, 0, 0, 0, time.UTC).UnixMilli()
	cola.Encolar(ctx, "msg-horario", "conv-1", "hola", tFuera-5000)

	prevHorario := outbox.ContadorAplazadasPorHorario.Load()
	if err := cola.Drenar(ctx, tFuera); err != nil {
		t.Fatalf("error al drenar fuera de horario: %v", err)
	}
	if falso.contador() != 0 {
		t.Errorf("no debía transmitirse fuera de horario")
	}
	if outbox.ContadorAplazadasPorHorario.Load() <= prevHorario {
		t.Errorf("ContadorAplazadasPorHorario no se incrementó")
	}

	var intentos int64
	db.QueryRow("SELECT intentos FROM cola_salida WHERE id_mensaje='msg-horario'").Scan(&intentos)
	if intentos != 0 {
		t.Errorf("intentos no debía moverse, obtenido: %d", intentos)
	}

	// Martes 2026-08-11 10:00 BA (13:00 UTC) - dentro de horario
	tDentro := time.Date(2026, 8, 11, 13, 0, 0, 0, time.UTC).UnixMilli()
	if err := cola.Drenar(ctx, tDentro); err != nil {
		t.Fatalf("error al drenar dentro de horario: %v", err)
	}
	if falso.contador() != 1 {
		t.Errorf("debía transmitirse dentro de horario, llamadas=%d", falso.contador())
	}
}

func TestDrenarInteraccionHorarioYTtl(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	falso := &transmisorFalso{idCorrelacion: "corr-1"}
	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 1000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 9, MinutoApertura: 0,
			HoraCierre: 19, MinutoCierre: 0,
			Dias: []int{1, 2, 3, 4, 5},
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	// TTL de 15 minutos (900000 ms)
	cola := outbox.NuevaColaDeSalida(db, 900000, 3, reg, falso, nil).ConDisciplina(disc, nil)
	ctx := context.Background()

	// Mensaje recibido a las 20:00 BA (fuera de horario)
	tOrigen := time.Date(2026, 8, 11, 23, 0, 0, 0, time.UTC).UnixMilli()
	cola.Encolar(ctx, "msg-vencido", "conv-1", "hola", tOrigen)

	// A las 09:30 BA del día siguiente (12:30 UTC): la ventana abre pero pasaron 13.5 horas > 15 min TTL
	tManana := time.Date(2026, 8, 12, 12, 30, 0, 0, time.UTC).UnixMilli()
	if err := cola.Drenar(ctx, tManana); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	if falso.contador() != 0 {
		t.Errorf("un mensaje retenido que excedió el TTL nunca debe transmitirse")
	}

	var count int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-vencido'").Scan(&count)
	if count != 0 {
		t.Errorf("el mensaje debía ser descartado con dureza por TTL, count=%d", count)
	}
	if !strings.Contains(buf.String(), outbox.EventoSalidaExpirada) {
		t.Errorf("se esperaba outbox.salida_expirada en el registro")
	}
}

func TestDrenarEmitePresenciaSoloPorLatencia(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-1"}
	emisor := &emisorPresenciaFalso{}
	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 9, MinutoApertura: 0,
			HoraCierre: 19, MinutoCierre: 0,
			Dias: []int{1, 2, 3, 4, 5},
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 20, IncrementoSemanal: 20, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, nil, falso, nil).ConDisciplina(disc, emisor)
	ctx := context.Background()

	// Caso 1: Retenido fuera de horario -> NO debe emitir presencia
	tFuera := time.Date(2026, 8, 11, 11, 0, 0, 0, time.UTC).UnixMilli()
	cola.Encolar(ctx, "msg-fuera", "conv-1", "hola fuera", tFuera-500)
	cola.Drenar(ctx, tFuera)
	if emisor.contador() != 0 {
		t.Errorf("no debía emitir presencia fuera de horario, llamadas=%d", emisor.contador())
	}

	// Caso 2: Dentro de horario, retenido únicamente por latencia mínima -> DEBE emitir presencia exactamente una vez
	tDentro := time.Date(2026, 8, 11, 14, 0, 0, 0, time.UTC).UnixMilli()
	cola.Encolar(ctx, "msg-lat", "conv-1", "hola lat", tDentro)
	cola.Drenar(ctx, tDentro+1000)
	if emisor.contador() != 1 {
		t.Errorf("debía emitir presencia por latencia mínima, llamadas=%d", emisor.contador())
	}

	// Segundo ciclo antes de que expire la latencia: no debe duplicar la emisión (en memoria)
	cola.Drenar(ctx, tDentro+2000)
	if emisor.contador() != 1 {
		t.Errorf("no debía duplicar la emisión de presencia para el mismo mensaje, llamadas=%d", emisor.contador())
	}
}

func TestPresenciaSeLiberaTrasEnviarLaFila(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-1"}
	emisor := &emisorPresenciaFalso{}
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: time.UTC,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, nil, falso, nil).ConDisciplina(disc, emisor)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-pres-liberada", "conv-1", "hola", 1000)

	// t=1500: aplazado por latencia, se anuncia presencia y queda el marcador en memoria.
	cola.Drenar(ctx, 1500)
	if emisor.contador() != 1 {
		t.Fatalf("debía emitir presencia por latencia mínima, llamadas=%d", emisor.contador())
	}
	if cola.PresenciasPendientesParaPruebas() != 1 {
		t.Fatalf("el marcador debía conservar exactamente 1 entrada tras el aplazamiento, obtenido %d", cola.PresenciasPendientesParaPruebas())
	}

	// t=4500: se cumple la latencia y la fila se transmite y marca enviada.
	if err := cola.Drenar(ctx, 4500); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}
	if falso.contador() != 1 {
		t.Fatalf("debía transmitirse tras cumplirse la latencia, llamadas=%d", falso.contador())
	}
	if cola.PresenciasPendientesParaPruebas() != 0 {
		t.Errorf("el marcador de presencia debía liberarse tras enviar la fila, quedaron %d entradas", cola.PresenciasPendientesParaPruebas())
	}
}

func TestDrenarFalloDePresenciaEsNoFatal(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-1"}
	emisor := &emisorPresenciaFalso{fallar: true}
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 2000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: time.UTC,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, nil, falso, nil).ConDisciplina(disc, emisor)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-pres-fallo", "conv-1", "hola", 1000)

	// En t=1500 falla presencia, mensaje sigue vivo
	cola.Drenar(ctx, 1500)
	if emisor.contador() != 1 {
		t.Errorf("debía intentar emitir presencia, llamadas=%d", emisor.contador())
	}

	// En t=3500 se transmite con éxito
	cola.Drenar(ctx, 3500)
	if falso.contador() != 1 {
		t.Errorf("debía transmitirse a pesar del fallo previo de presencia, llamadas=%d", falso.contador())
	}
}

func TestDrenarAplicaRampaDeVolumenYPersisteContador(t *testing.T) {
	t.Parallel()
	db, dsn := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "corr-exito"}
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 1000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: time.UTC,
		},
		Rampa: configuracion.RampaDeVolumen{
			DiariaInicial:     2,
			IncrementoSemanal: 5,
			Semanas:           4,
		},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, nil, falso, nil).ConDisciplina(disc, nil)
	ctx := context.Background()

	ahoraMs := time.Date(2026, 8, 11, 12, 0, 0, 0, time.UTC).UnixMilli()

	// Encolar 3 mensajes
	cola.Encolar(ctx, "msg-1", "conv-1", "hola 1", ahoraMs-5000)
	cola.Encolar(ctx, "msg-2", "conv-1", "hola 2", ahoraMs-5000)
	cola.Encolar(ctx, "msg-3", "conv-1", "hola 3", ahoraMs-5000)

	prevRampa := outbox.ContadorAplazadasPorRampa.Load()
	if err := cola.Drenar(ctx, ahoraMs); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}

	// Cupo es 2 en semana 0: solo los dos primeros deben enviarse
	if falso.contador() != 2 {
		t.Errorf("debían transmitirse exactamente 2 mensajes por cupo de rampa, llamadas=%d", falso.contador())
	}
	if outbox.ContadorAplazadasPorRampa.Load() <= prevRampa {
		t.Errorf("ContadorAplazadasPorRampa no se incrementó")
	}

	var countEnviados, countPendientes int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE enviado_en_ms IS NOT NULL").Scan(&countEnviados)
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE enviado_en_ms IS NULL").Scan(&countPendientes)
	if countEnviados != 2 || countPendientes != 1 {
		t.Errorf("enviados=%d (esperado 2), pendientes=%d (esperado 1)", countEnviados, countPendientes)
	}

	var intentos int64
	db.QueryRow("SELECT intentos FROM cola_salida WHERE enviado_en_ms IS NULL").Scan(&intentos)
	if intentos != 0 {
		t.Errorf("un aplazamiento por rampa no debía mover intentos, obtenido: %d", intentos)
	}

	// Verificar persistencia en SQLite tras reapertura
	db2, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("error reabriendo bd: %v", err)
	}
	defer db2.Close()

	enviosDia, err := outbox.EnviosDelDia(ctx, db2, "2026-08-11")
	if err != nil || enviosDia != 2 {
		t.Errorf("envíos persistidos = %d (esperado 2), err=%v", enviosDia, err)
	}
}

func TestPresenciaSeLiberaTrasAgotarIntentos(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{fallar: true}
	emisor := &emisorPresenciaFalso{}
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: time.UTC,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	// intentosMaximos = 2
	cola := outbox.NuevaColaDeSalida(db, 60000, 2, nil, falso, nil).ConDisciplina(disc, emisor)
	ctx := context.Background()

	cola.Encolar(ctx, "msg-pres-agotar", "conv-1", "hola", 1000)

	// t=1500: aplazado por latencia, se anuncia presencia
	cola.Drenar(ctx, 1500)
	if emisor.contador() != 1 {
		t.Fatalf("debía emitir presencia por latencia mínima, llamadas=%d", emisor.contador())
	}
	if cola.PresenciasPendientesParaPruebas() != 1 {
		t.Fatalf("el marcador debía conservar 1 entrada, obtenido %d", cola.PresenciasPendientesParaPruebas())
	}

	// t=4500: intento 1 de transmisión (falla, intentos pasa a 1)
	cola.Drenar(ctx, 4500)
	if cola.PresenciasPendientesParaPruebas() != 1 {
		t.Fatalf("tras intento 1 el marcador debe seguir activo, obtenido %d", cola.PresenciasPendientesParaPruebas())
	}

	// t=5000: intento 2 de transmisión (falla y se agota, elimina la fila)
	cola.Drenar(ctx, 5000)
	if cola.PresenciasPendientesParaPruebas() != 0 {
		t.Errorf("el marcador de presencia debía liberarse tras agotar reintentos, quedaron %d entradas", cola.PresenciasPendientesParaPruebas())
	}
}

type controlDeBajaMutable struct {
	permitido bool
}

func (c *controlDeBajaMutable) EnvioPermitido(_ context.Context, _, _ string) (bool, error) {
	return c.permitido, nil
}

func (c *controlDeBajaMutable) ReclamarConfirmacionDeBaja(_ context.Context, _, _ string, _ int64) (bool, error) {
	return false, nil
}

func TestPresenciaSeLiberaTrasDescartePorBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	falso := &transmisorFalso{idCorrelacion: "no-debe-enviarse"}
	emisor := &emisorPresenciaFalso{}
	control := &controlDeBajaMutable{permitido: true}
	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 9, MinutoApertura: 0,
			HoraCierre: 19, MinutoCierre: 0,
			Dias: []int{1, 2, 3, 4, 5},
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{DiariaInicial: 100, IncrementoSemanal: 10, Semanas: 4},
	}
	disc := outbox.NuevaDisciplinaDeSalida(cfg)
	cola := outbox.NuevaColaDeSalida(db, 60000, 3, nil, falso, control).ConDisciplina(disc, emisor)
	ctx := context.Background()

	tDentro := time.Date(2026, 8, 11, 14, 0, 0, 0, time.UTC).UnixMilli()
	cola.Encolar(ctx, "msg-baja-pres", "conv-1", "hola", tDentro)

	// t=tDentro+1000: se difiere por latencia mínima, anuncia presencia
	if err := cola.Drenar(ctx, tDentro+1000); err != nil {
		t.Fatalf("error al drenar: %v", err)
	}
	if emisor.contador() != 1 {
		t.Fatalf("debía emitir presencia por latencia mínima, llamadas=%d", emisor.contador())
	}
	if cola.PresenciasPendientesParaPruebas() != 1 {
		t.Fatalf("el marcador debía conservar 1 entrada, obtenido %d", cola.PresenciasPendientesParaPruebas())
	}

	// Contacto solicita baja entre ciclos
	control.permitido = false

	// t=tDentro+4000: se cumple la latencia, pero el control de baja rechaza "msg-baja-pres".
	// La fila se descarta por baja y el marcador de presencia debe liberarse (AC-6).
	if err := cola.Drenar(ctx, tDentro+4000); err != nil {
		t.Fatalf("error al drenar en tDentro+4000: %v", err)
	}
	if falso.contador() != 0 {
		t.Fatalf("el mensaje no debía transmitirse tras descarte por baja, llamadas=%d", falso.contador())
	}
	if cola.PresenciasPendientesParaPruebas() != 0 {
		t.Errorf("el marcador de presencia debía liberarse tras el descarte por baja (AC-6), quedaron %d entradas", cola.PresenciasPendientesParaPruebas())
	}
}
