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

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"

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

func TestEncolarEsIdempotente(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil)
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
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil)
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
	cola2 := outbox.NuevaColaDeSalida(db2, 1000, 3, nil, nil)

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
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, reg, nil)
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
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil)
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
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, falso)
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
	cola := outbox.NuevaColaDeSalida(db, 1_000_000, 2, nil, falso)
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
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, falso)
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
