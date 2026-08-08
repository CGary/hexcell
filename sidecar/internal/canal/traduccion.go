package canal

import (
	"context"
	"strings"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// SumideroDeEvento recibe cada evento entrante traducido, listo para codificarse por IPC.
// Sigue la convención del SumideroDeEstado ya existente en reconexion.go.
type SumideroDeEvento func(ipc.EventoEntrante)

// BuzonDurable es la interfaz que *outbox.Outbox satisface. Desacoplada para testear
// el orden persist-first con un espía.
type BuzonDurable interface {
	Persistir(ctx context.Context, idDeduplicacion string, carga []byte) error
}

// ResolutorDeAlias resuelve el JID alternativo de un contacto.
// En producción es store.Device.GetAltJID; nil es legal (no hay alias disponible).
type ResolutorDeAlias interface {
	AliasDe(ctx context.Context, jid types.JID) (types.JID, error)
}

// Nombres fijos de suceso
const (
	EventoMensajeTraducido  = "canal.mensaje_traducido"
	EventoMensajeDescartado = "canal.mensaje_descartado"
)

// Traductor convierte eventos entrantes de whatsmeow en EventoEntrante de IPC
// asegurando persist-first y deduplicación.
type Traductor struct {
	almacen   *identidad.Almacen
	buzon     BuzonDurable
	sumidero  SumideroDeEvento
	resolutor ResolutorDeAlias
	registro  *registro.Registro
	ahoraMs   func() int64 // no se usa para la marca de tiempo de los mensajes; costura para tests
}

// NuevoTraductor construye un Traductor con las dependencias inyectadas.
func NuevoTraductor(
	almacen *identidad.Almacen,
	buzon BuzonDurable,
	sumidero SumideroDeEvento,
	resolutor ResolutorDeAlias,
	reg *registro.Registro,
) *Traductor {
	return &Traductor{
		almacen:   almacen,
		buzon:     buzon,
		sumidero:  sumidero,
		resolutor: resolutor,
		registro:  reg,
	}
}

func admisible(info types.MessageInfo) (string, bool) {
	if info.IsFromMe {
		return "mensaje enviado por el propio usuario", false
	}
	if info.IsGroup {
		return "mensaje de grupo", false
	}
	if info.Chat.Server != "s.whatsapp.net" && info.Chat.Server != "lid" {
		return "servidor de chat no admitido", false
	}
	if info.Edit != "" {
		return "mensaje editado", false
	}
	return "", true
}

func extraerTexto(msg *waE2E.Message) (string, bool) {
	if msg == nil {
		return "", false
	}
	texto := msg.GetConversation()
	if texto == "" {
		if ext := msg.GetExtendedTextMessage(); ext != nil {
			texto = ext.GetText()
		}
	}
	texto = strings.TrimSpace(texto)
	if texto == "" {
		return "sin texto extraíble", false
	}
	return texto, true
}

func construirIdDeduplicacion(idInterno, idMensaje string) string {
	return idInterno + ":" + idMensaje
}

func (t *Traductor) procesarMensaje(ctx context.Context, evt *events.Message) {
	razon, esAdmisible := admisible(evt.Info)
	if !esAdmisible {
		if t.registro != nil {
			t.registro.Info(EventoMensajeDescartado, registro.Campos{
				Detalle: razon,
			})
		}
		return
	}

	texto, tieneTexto := extraerTexto(evt.Message)
	if !tieneTexto {
		if t.registro != nil {
			t.registro.Info(EventoMensajeDescartado, registro.Campos{
				Detalle: texto, // que aquí contiene "sin texto extraíble"
			})
		}
		return
	}

	var obs identidad.Observacion

	// Asignar basándonos en AddressingMode y en el servidor del JID.
	if evt.Info.AddressingMode == types.AddressingModePN || evt.Info.Sender.Server == "s.whatsapp.net" {
		obs.PN = evt.Info.Sender
	} else if evt.Info.AddressingMode == types.AddressingModeLID || evt.Info.Sender.Server == "lid" {
		obs.LID = evt.Info.Sender
	}

	alt := evt.Info.SenderAlt
	// Consultar resolutor únicamente si falta SenderAlt
	if alt.IsEmpty() && t.resolutor != nil {
		if alias, err := t.resolutor.AliasDe(ctx, evt.Info.Sender); err == nil && !alias.IsEmpty() {
			alt = alias
		}
	}

	if !alt.IsEmpty() {
		if alt.Server == "s.whatsapp.net" {
			obs.PN = alt
		} else if alt.Server == "lid" {
			obs.LID = alt
		}
	}

	ident, err := t.almacen.Resolver(ctx, obs)
	if err != nil {
		if t.registro != nil {
			t.registro.Error("canal.error_resolucion", registro.Campos{
				Detalle: "no se pudo resolver la identidad interna",
			})
		}
		return
	}

	idDeduplicacion := construirIdDeduplicacion(ident.IdInterno, evt.Info.ID)

	evento := ipc.EventoEntrante{
		IdDeduplicacion: idDeduplicacion,
		IdConversacion:  ident.IdInterno, // DM, la conversación es el remitente
		IdRemitente:     ident.IdInterno,
		Contenido:       texto,
		MarcaTemporalMs: evt.Info.Timestamp.UnixMilli(),
	}

	sobre := ipc.NuevoSobre(evento)
	carga, err := ipc.Codificar(sobre)
	if err != nil {
		if t.registro != nil {
			t.registro.Error("canal.error_codificacion", registro.Campos{
				Detalle: "no se pudo codificar el evento IPC",
			})
		}
		return
	}

	// Persistir-first
	err = t.buzon.Persistir(ctx, idDeduplicacion, carga)
	if err != nil {
		if t.registro != nil {
			t.registro.Error("canal.error_persistencia", registro.Campos{
				Detalle: "no se pudo persistir el evento en el buzón",
			})
		}
		return
	}

	if t.sumidero != nil {
		t.sumidero(evento)
	}

	if t.registro != nil {
		t.registro.Info(EventoMensajeTraducido, registro.Campos{
			IdEvento: idDeduplicacion,
		})
	}
}

// RegistrarTraductor acopla el traductor al cliente whatsmeow.
func (s *Sesion) RegistrarTraductor(traductor *Traductor) uint32 {
	return s.cliente.AddEventHandler(func(evento any) {
		msg, esMensaje := evento.(*events.Message)
		if !esMensaje {
			return
		}
		traductor.procesarMensaje(s.ctx, msg)
	})
}
