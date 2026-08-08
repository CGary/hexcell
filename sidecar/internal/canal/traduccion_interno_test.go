package canal

import (
	"context"
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
	return NuevoTraductor(almacen, r, r.sumidero, resolutor, nil), r
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
	traductor := NuevoTraductor(almacen, buzon, func(e ipc.EventoEntrante) { eventos = append(eventos, e) }, nil, nil)
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
