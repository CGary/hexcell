package canal

import (
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
)

func mensajePruebaInterno() *events.Message {
	return &events.Message{
		Info: types.MessageInfo{
			MessageSource: types.MessageSource{
				IsFromMe: false,
				IsGroup:  false,
				Chat:     types.JID{User: "5491155551234", Server: types.DefaultUserServer},
				Sender:   types.JID{User: "5491155551234", Server: types.DefaultUserServer},
			},
			ID:        "MSG_ID",
			Timestamp: time.Unix(1722816000, 0),
		},
		Message: &waE2E.Message{
			Conversation: proto.String("Hola"),
		},
	}
}

// registroDeLlamadas es a la vez BuzonDurable y SumideroDeEvento: acumula el orden real de
// las llamadas que procesarMensaje efectúa, para probar persist-first sin simularlo aparte.
type registroDeLlamadas struct {
	orden          []string
	idsPersistidos []string
	eventos        []ipc.EventoEntrante
	errPersistir   error
}

func (r *registroDeLlamadas) Persistir(_ context.Context, id string, _ []byte) error {
	r.orden = append(r.orden, "persistir")
	r.idsPersistidos = append(r.idsPersistidos, id)
	return r.errPersistir
}

func (r *registroDeLlamadas) sumidero(e ipc.EventoEntrante) {
	r.orden = append(r.orden, "sumidero")
	r.eventos = append(r.eventos, e)
}

// resolutorEspiaInterno simula un ResolutorDeAlias cuyo AliasDe siempre falla.
type resolutorEspiaInterno struct{}

func (resolutorEspiaInterno) AliasDe(_ context.Context, _ types.JID) (types.JID, error) {
	return types.JID{}, errors.New("dispositivo eliminado")
}

func construirTraductorDePrueba(t *testing.T, resolutor ResolutorDeAlias) (*Traductor, *registroDeLlamadas) {
	t.Helper()
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(t.TempDir(), "identidad.db")})
	if err != nil {
		t.Fatalf("Abrir identidad: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })
	r := &registroDeLlamadas{}
	return NuevoTraductor(almacen, r, r.sumidero, resolutor, nil, nil), r
}

func TestAdmisible_ListaPermitida(t *testing.T) {
	t.Parallel()
	m := mensajePruebaInterno()
	if razon, ok := admisible(m.Info); !ok || razon != "" {
		t.Errorf("el mensaje básico debe ser admisible con razón vacía, se obtuvo ok=%v razon=%q", ok, razon)
	}

	m.Info.IsFromMe = true
	if razon, ok := admisible(m.Info); ok || razon != "mensaje enviado por el propio usuario" {
		t.Errorf("isFromMe debe ser inadmisible con razón fija, se obtuvo ok=%v razon=%q", ok, razon)
	}
	m.Info.IsFromMe = false

	m.Info.IsGroup = true
	if razon, ok := admisible(m.Info); ok || razon != "mensaje de grupo" {
		t.Errorf("isGroup debe ser inadmisible con razón fija, se obtuvo ok=%v razon=%q", ok, razon)
	}
	m.Info.IsGroup = false

	m.Info.Chat.Server = types.BroadcastServer
	if razon, ok := admisible(m.Info); ok || razon != "servidor de chat no admitido" {
		t.Errorf("broadcast server debe ser inadmisible con razón fija, se obtuvo ok=%v razon=%q", ok, razon)
	}
	m.Info.Chat.Server = types.DefaultUserServer

	m.Info.Edit = types.EditAttributeMessageEdit
	if razon, ok := admisible(m.Info); ok || razon != "mensaje editado" {
		t.Errorf("mensaje editado debe ser inadmisible con razón fija, se obtuvo ok=%v razon=%q", ok, razon)
	}
}

func TestExtraerTexto_SoloConversacionYExtendedText(t *testing.T) {
	t.Parallel()

	m1 := &waE2E.Message{Conversation: proto.String("Conv")}
	if extraido, _ := extraerTexto(m1); extraido != "Conv" {
		t.Errorf("se esperaba Conv, se obtuvo %q", extraido)
	}

	m2 := &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("Ext")}}
	if extraido, _ := extraerTexto(m2); extraido != "Ext" {
		t.Errorf("se esperaba Ext, se obtuvo %q", extraido)
	}

	m3 := &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("Img")}}
	if _, ok := extraerTexto(m3); ok {
		t.Errorf("no se debe extraer texto de un cuerpo no textual (caption de imagen)")
	}
}

func TestConstruirIdDeduplicacion_Formato(t *testing.T) {
	t.Parallel()
	idInterno := "ct-12345"
	idMensaje := "MSG001"

	esperado := "ct-12345:MSG001"
	obtenido := construirIdDeduplicacion(idInterno, idMensaje)

	if obtenido != esperado {
		t.Errorf("se esperaba %q, se obtuvo %q", esperado, obtenido)
	}
}

func TestProcesarMensaje_PersistPrimeroYDeduplicacionEstable(t *testing.T) {
	t.Parallel()
	traductor, r := construirTraductorDePrueba(t, nil)
	msg := mensajePruebaInterno()

	traductor.procesarMensaje(context.Background(), msg)
	traductor.procesarMensaje(context.Background(), msg)

	if len(r.orden) != 4 || r.orden[0] != "persistir" || r.orden[1] != "sumidero" ||
		r.orden[2] != "persistir" || r.orden[3] != "sumidero" {
		t.Errorf("el orden debe ser persistir, sumidero, persistir, sumidero; se obtuvo %v", r.orden)
	}
	if len(r.idsPersistidos) != 2 || r.idsPersistidos[0] != r.idsPersistidos[1] {
		t.Errorf("el id de deduplicación no es estable entre dos entregas: %v", r.idsPersistidos)
	}
}

func TestProcesarMensaje_PersistirFalla_SumideroNuncaSeLlama(t *testing.T) {
	t.Parallel()
	traductor, r := construirTraductorDePrueba(t, nil)
	r.errPersistir = errors.New("error de disco")

	traductor.procesarMensaje(context.Background(), mensajePruebaInterno())

	if len(r.eventos) != 0 {
		t.Errorf("el sumidero no debe llamarse si persistir falla")
	}
	if len(r.orden) != 1 || r.orden[0] != "persistir" {
		t.Errorf("se esperaba únicamente el intento de persistir, se obtuvo %v", r.orden)
	}
}

func TestProcesarMensaje_TextoPlano_ProduceEventoEntranteValido(t *testing.T) {
	t.Parallel()
	traductor, r := construirTraductorDePrueba(t, nil)
	msg := mensajePruebaInterno()

	traductor.procesarMensaje(context.Background(), msg)

	if len(r.eventos) != 1 {
		t.Fatalf("se esperaba 1 evento, se obtuvieron %d", len(r.eventos))
	}
	evento := r.eventos[0]
	if !strings.HasPrefix(evento.IdRemitente, identidad.PrefijoIdentidad) {
		t.Errorf("id_remitente no es opaco: %s", evento.IdRemitente)
	}
	if evento.Contenido != "Hola" {
		t.Errorf("contenido incorrecto: %s", evento.Contenido)
	}
	if evento.MarcaTemporalMs != msg.Info.Timestamp.UnixMilli() {
		t.Errorf("marca_temporal_ms incorrecta: %d", evento.MarcaTemporalMs)
	}
}

func TestProcesarMensaje_RoundTripYSinIdentificadorDeTransporte(t *testing.T) {
	t.Parallel()
	traductor, r := construirTraductorDePrueba(t, nil)
	msg := mensajePruebaInterno()
	numero := msg.Info.Sender.User

	traductor.procesarMensaje(context.Background(), msg)
	if len(r.eventos) != 1 {
		t.Fatalf("se esperaba 1 evento, se obtuvieron %d", len(r.eventos))
	}
	evento := r.eventos[0]

	codificado, err := ipc.Codificar(ipc.NuevoSobre(evento))
	if err != nil {
		t.Fatalf("error al codificar: %v", err)
	}
	decodificado, err := ipc.Decodificar(codificado)
	if err != nil {
		t.Fatalf("error al decodificar: %v", err)
	}
	eventoDecodificado, ok := decodificado.Cuerpo.(ipc.EventoEntrante)
	if !ok || eventoDecodificado != evento {
		t.Errorf("el evento decodificado no coincide con el original: %+v vs %+v", eventoDecodificado, evento)
	}

	for _, valor := range []string{evento.IdDeduplicacion, evento.IdConversacion, evento.IdRemitente, evento.Contenido} {
		if strings.Contains(valor, "@") || strings.Contains(valor, "s.whatsapp.net") || strings.Contains(valor, "lid") || strings.Contains(valor, numero) {
			t.Errorf("se filtró identificador de transporte en: %s", valor)
		}
	}
}

func TestProcesarMensaje_FiltroAdmisibleYTextoVacio(t *testing.T) {
	t.Parallel()
	casos := []struct {
		nombre string
		mod    func(*events.Message)
	}{
		{"IsFromMe_true", func(m *events.Message) { m.Info.IsFromMe = true }},
		{"IsGroup_true", func(m *events.Message) { m.Info.IsGroup = true }},
		{"Broadcast_server", func(m *events.Message) { m.Info.Chat.Server = types.BroadcastServer }},
		{"Edit_no_vacio", func(m *events.Message) { m.Info.Edit = types.EditAttributeMessageEdit }},
		{"Cuerpo_no_texto", func(m *events.Message) { m.Message.Conversation = nil; m.Message.ImageMessage = &waE2E.ImageMessage{} }},
		{"Texto_vacio_tras_limpiar", func(m *events.Message) { m.Message.Conversation = proto.String("   \n \t ") }},
	}

	for _, c := range casos {
		t.Run(c.nombre, func(t *testing.T) {
			traductor, r := construirTraductorDePrueba(t, nil)
			msg := mensajePruebaInterno()
			c.mod(msg)
			traductor.procesarMensaje(context.Background(), msg)

			if len(r.idsPersistidos) > 0 || len(r.eventos) > 0 {
				t.Errorf("se debía descartar el mensaje, se obtuvo orden=%v", r.orden)
			}
		})
	}
}

func TestProcesarMensaje_ResolutorDeAliasConError_NoFatal(t *testing.T) {
	t.Parallel()
	traductor, r := construirTraductorDePrueba(t, resolutorEspiaInterno{})

	traductor.procesarMensaje(context.Background(), mensajePruebaInterno())

	if len(r.eventos) != 1 {
		t.Fatalf("se esperaba que tradujera pese al error del resolutor, se obtuvieron %d eventos", len(r.eventos))
	}
}

func TestProcesarMensaje_MismoMensajeDosVeces_UnaSolaFilaEnOutboxReal(t *testing.T) {
	t.Parallel()
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(t.TempDir(), "identidad.db")})
	if err != nil {
		t.Fatalf("Abrir identidad: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })
	buzon, err := outbox.Abrir(outbox.Opciones{Ruta: filepath.Join(t.TempDir(), "outbox.db")})
	if err != nil {
		t.Fatalf("Abrir outbox: %v", err)
	}
	t.Cleanup(func() { buzon.Cerrar() })

	var eventos []ipc.EventoEntrante
	traductor := NuevoTraductor(almacen, buzon, func(e ipc.EventoEntrante) { eventos = append(eventos, e) }, nil, nil, nil)
	msg := mensajePruebaInterno()

	traductor.procesarMensaje(context.Background(), msg)
	traductor.procesarMensaje(context.Background(), msg)

	pendientes, err := buzon.Pendientes(context.Background())
	if err != nil {
		t.Fatalf("Pendientes: %v", err)
	}
	if len(pendientes) != 1 {
		t.Errorf("se esperaba una sola fila en el outbox tras dos entregas del mismo mensaje, se obtuvieron %d", len(pendientes))
	}
	if len(eventos) != 2 {
		t.Errorf("se esperaban 2 eventos emitidos al sumidero, se obtuvieron %d", len(eventos))
	}
}

func TestNormalizarParaBaja_Casos(t *testing.T) {
	t.Parallel()
	casos := []struct {
		entrada  string
		esperado string
	}{
		{"BAJA", "baja"},
		{"baja", "baja"},
		{" Baja ", "baja"},
		{"Baja.", "baja"},
		{"¿Baja?", "baja"},
		{"¡baja!", "baja"},
		{"  STOP...  ", "stop"},
		{"quiero dar de baja mi servicio", "quiero dar de baja mi servicio"},
	}

	for _, c := range casos {
		obtenido := normalizarParaBaja(c.entrada)
		if obtenido != c.esperado {
			t.Errorf("normalizarParaBaja(%q) = %q, se esperaba %q", c.entrada, obtenido, c.esperado)
		}
	}
}

func TestDetectorDeBaja_CoincideExacto(t *testing.T) {
	t.Parallel()
	detector := NuevoDetectorDeBaja([]string{"baja", "stop"}, "confirmacion", nil, nil)

	if !detector.Coincide("BAJA") {
		t.Errorf("debia coincidir con BAJA")
	}
	if !detector.Coincide("  baja.  ") {
		t.Errorf("debia coincidir con '  baja.  '")
	}
	if !detector.Coincide("¿STOP?") {
		t.Errorf("debia coincidir con ¿STOP?")
	}
	if detector.Coincide("quiero dar de baja mi servicio") {
		t.Errorf("no debia coincidir como subcadena dentro de frase")
	}
	if detector.Coincide("no") {
		t.Errorf("no debia coincidir con palabra no configurada")
	}
}

func TestProcesarMensaje_PalabraDeBaja_RegistraEnIdentidadYEncolaConfirmacion(t *testing.T) {
	t.Parallel()
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(t.TempDir(), "identidad.db")})
	if err != nil {
		t.Fatalf("Abrir identidad: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })

	buzon, err := outbox.Abrir(outbox.Opciones{Ruta: filepath.Join(t.TempDir(), "outbox.db")})
	if err != nil {
		t.Fatalf("Abrir outbox: %v", err)
	}
	t.Cleanup(func() { buzon.Cerrar() })

	cola := outbox.NuevaColaDeSalida(buzon.DB(), 1000, 3, nil, nil, almacen)
	portero := outbox.NuevoPorteroDeSalida(cola, almacen, nil)
	textoConf := "Has sido dado de baja correctamente."
	detector := NuevoDetectorDeBaja([]string{"baja", "stop"}, textoConf, almacen, portero)

	var eventos []ipc.EventoEntrante
	sumidero := func(e ipc.EventoEntrante) {
		eventos = append(eventos, e)
	}

	traductor := NuevoTraductor(almacen, buzon, sumidero, nil, nil, detector)

	msg := mensajePruebaInterno()
	msg.Message.Conversation = proto.String("BAJA")

	ctx := context.Background()
	traductor.procesarMensaje(ctx, msg)

	if len(eventos) != 1 {
		t.Fatalf("se esperaba 1 evento entrante emitido al sumidero, hubo %d", len(eventos))
	}
	idRemitente := eventos[0].IdRemitente

	permitido, err := almacen.EnvioPermitido(ctx, idRemitente, "otro-mensaje")
	if err != nil || permitido {
		t.Fatalf("el contacto debía estar registrado como dado de baja, permitido=%v, err=%v", permitido, err)
	}

	var contenidoConf, idMsgConf string
	err = buzon.DB().QueryRow("SELECT id_mensaje, contenido FROM cola_salida WHERE id_conversacion = ?", idRemitente).Scan(&idMsgConf, &contenidoConf)
	if err != nil {
		t.Fatalf("no se encontró confirmación encolada: %v", err)
	}
	if contenidoConf != textoConf {
		t.Errorf("contenido de confirmación = %q, se esperaba %q", contenidoConf, textoConf)
	}

	traductor.procesarMensaje(ctx, msg)

	var totalConfirmaciones int
	buzon.DB().QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_conversacion = ?", idRemitente).Scan(&totalConfirmaciones)
	if totalConfirmaciones != 1 {
		t.Errorf("reentrega generó confirmaciones duplicadas, total=%d", totalConfirmaciones)
	}
}

func TestProcesarMensaje_ConfirmacionUsaRelojActualNoMarcaDelEntrante(t *testing.T) {
	t.Parallel()
	rutaIdentidad := filepath.Join(t.TempDir(), "identidad.db")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: rutaIdentidad})
	if err != nil {
		t.Fatalf("Abrir identidad: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })

	buzon, err := outbox.Abrir(outbox.Opciones{Ruta: filepath.Join(t.TempDir(), "outbox.db")})
	if err != nil {
		t.Fatalf("Abrir outbox: %v", err)
	}
	t.Cleanup(func() { buzon.Cerrar() })

	const ttlMs = int64(900000)
	transmisor := &transmisorRegistrador{}
	cola := outbox.NuevaColaDeSalida(buzon.DB(), ttlMs, 3, nil, transmisor, almacen)
	portero := outbox.NuevoPorteroDeSalida(cola, almacen, nil)
	detector := NuevoDetectorDeBaja([]string{"stop"}, "Baja confirmada.", almacen, portero)
	traductor := NuevoTraductor(almacen, buzon, func(ipc.EventoEntrante) {}, nil, nil, detector)

	// Reloj inyectado: "ahora" está una hora por delante de la marca del mensaje entrante,
	// como ocurre al drenar el backlog offline tras una caída más larga que el TTL.
	marcaEntrante := time.Unix(1722816000, 0)
	ahoraMs := marcaEntrante.UnixMilli() + 3600_000
	traductor.ahoraMs = func() int64 { return ahoraMs }

	msg := mensajePruebaInterno()
	msg.Message.Conversation = proto.String("STOP")

	ctx := context.Background()
	traductor.procesarMensaje(ctx, msg)

	var marcaConfirmacion int64
	if err := buzon.DB().QueryRow("SELECT marca_temporal_origen_ms FROM cola_salida").Scan(&marcaConfirmacion); err != nil {
		t.Fatalf("no se encontró confirmación encolada: %v", err)
	}
	if marcaConfirmacion != ahoraMs {
		t.Errorf("la confirmación se encoló con marca %d; debía usar el reloj actual %d, no la del entrante %d",
			marcaConfirmacion, ahoraMs, marcaEntrante.UnixMilli())
	}

	// La fila de baja sí conserva la marca del mensaje entrante: es el hecho histórico.
	// El Almacen no expone su *sql.DB; se abre una segunda conexión de solo lectura al archivo.
	dbIdentidad, err := sql.Open("sqlite", "file:"+rutaIdentidad+"?_pragma=journal_mode(WAL)")
	if err != nil {
		t.Fatalf("sql.Open identidad: %v", err)
	}
	t.Cleanup(func() { dbIdentidad.Close() })
	var dadaDeBajaEnMs int64
	if err := dbIdentidad.QueryRow("SELECT dada_de_baja_en_ms FROM baja_de_contacto").Scan(&dadaDeBajaEnMs); err != nil {
		t.Fatalf("no se encontró fila de baja: %v", err)
	}
	if dadaDeBajaEnMs != marcaEntrante.UnixMilli() {
		t.Errorf("dada_de_baja_en_ms = %d; debía conservar la marca del entrante %d", dadaDeBajaEnMs, marcaEntrante.UnixMilli())
	}

	// Y la confirmación se TRANSMITE en un drenaje ejecutado "ahora": no nace expirada.
	if err := cola.Drenar(ctx, ahoraMs); err != nil {
		t.Fatalf("Drenar: %v", err)
	}
	if transmisor.llamadas != 1 {
		t.Errorf("la confirmación debía transmitirse en el primer drenaje, transmisiones=%d", transmisor.llamadas)
	}
}

// transmisorRegistrador es un stub de outbox.Transmisor que cuenta las transmisiones.
type transmisorRegistrador struct {
	llamadas int
}

func (tr *transmisorRegistrador) Transmitir(_ context.Context, _, _ string) (string, error) {
	tr.llamadas++
	return "corr-test", nil
}

func TestProcesarMensaje_DosCelulasConfiguracionesDistintas(t *testing.T) {
	t.Parallel()
	almacenA, _ := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(t.TempDir(), "identidad_a.db")})
	defer almacenA.Cerrar()
	almacenB, _ := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(t.TempDir(), "identidad_b.db")})
	defer almacenB.Cerrar()

	buzonA, _ := outbox.Abrir(outbox.Opciones{Ruta: filepath.Join(t.TempDir(), "outbox_a.db")})
	defer buzonA.Cerrar()
	buzonB, _ := outbox.Abrir(outbox.Opciones{Ruta: filepath.Join(t.TempDir(), "outbox_b.db")})
	defer buzonB.Cerrar()

	colaA := outbox.NuevaColaDeSalida(buzonA.DB(), 1000, 3, nil, nil, almacenA)
	porteroA := outbox.NuevoPorteroDeSalida(colaA, almacenA, nil)
	detectorA := NuevoDetectorDeBaja([]string{"baja"}, "Confirmacion A", almacenA, porteroA)
	traductorA := NuevoTraductor(almacenA, buzonA, func(ipc.EventoEntrante) {}, nil, nil, detectorA)

	colaB := outbox.NuevaColaDeSalida(buzonB.DB(), 1000, 3, nil, nil, almacenB)
	porteroB := outbox.NuevoPorteroDeSalida(colaB, almacenB, nil)
	detectorB := NuevoDetectorDeBaja([]string{"stop"}, "Confirmacion B", almacenB, porteroB)
	traductorB := NuevoTraductor(almacenB, buzonB, func(ipc.EventoEntrante) {}, nil, nil, detectorB)

	msgStop := mensajePruebaInterno()
	msgStop.Message.Conversation = proto.String("STOP")
	traductorA.procesarMensaje(context.Background(), msgStop)

	var cuentaA int
	buzonA.DB().QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&cuentaA)
	if cuentaA != 0 {
		t.Errorf("Célula A no debía encolar confirmación ante 'STOP'")
	}

	traductorB.procesarMensaje(context.Background(), msgStop)
	var cuentaB int
	buzonB.DB().QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&cuentaB)
	if cuentaB != 1 {
		t.Errorf("Célula B debía encolar confirmación ante 'STOP'")
	}
}
