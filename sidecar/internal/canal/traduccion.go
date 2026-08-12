package canal

import (
	"context"
	"strings"
	"sync/atomic"
	"time"
	"unicode"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Cortacircuitos conversacional [causa documentada]

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

// ControlDeBaja registra la baja de un contacto de forma idempotente.
// Satisfecho por *identidad.Almacen; desacoplado para testear el detector con un espía.
type ControlDeBaja interface {
	RegistrarBaja(ctx context.Context, idInterno string, dadaDeBajaEnMs int64) (bool, error)
}

// AdmisorDeConfirmacion encola la confirmación única de baja tras ganar su reclamo atómico.
// Satisfecho por *outbox.PorteroDeSalida; desacoplado para testear el detector con un espía.
type AdmisorDeConfirmacion interface {
	AdmitirConfirmacionDeBaja(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) (bool, error)
}

// ControlDeCortacircuitos registra observaciones y gestiona el cortacircuitos para el canal entrante.
// Satisfecho por *identidad.Almacen; desacoplado para testear el detector con un espía.
// [causa documentada]
type ControlDeCortacircuitos interface {
	RegistrarObservacion(ctx context.Context, idInterno, textoNormalizado string, esFrustracion bool, umbralRepeticion int64, ahoraMs int64) (bool, error)
}

// AdmisorDeTraspaso encola el mensaje único de traspaso tras dispararse el cortacircuitos.
// Satisfecho por *outbox.PorteroDeSalida; desacoplado para testear el detector con un espía.
// [causa documentada]
type AdmisorDeTraspaso interface {
	AdmitirMensajeDeTraspaso(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) (bool, error)
}

// Nombres fijos de suceso
const (
	EventoMensajeTraducido        = "canal.mensaje_traducido"
	EventoMensajeDescartado       = "canal.mensaje_descartado"
	EventoBajaDetectada           = "canal.baja_detectada"
	EventoCortacircuitosDisparado = "canal.cortacircuitos_disparado"
	EventoErrorCortacircuitos     = "canal.error_cortacircuitos"
	EventoErrorEmisionTraspaso    = "canal.error_emision_traspaso"
)

// ContadorErroresCortacircuitos cuenta, a través de todas las instancias del proceso, fallos al
// registrar la observación entrante del cortacircuitos (fallo cerrado: el mensaje no se reenvía
// al núcleo para que no se genere una respuesta automática mientras el estado del cortacircuitos
// no pueda leerse ni escribirse con certeza). Sigue el mismo idioma de contador atómico exportado
// que outbox.ContadorErroresCortacircuitos, fix cortacircuitos-fallo-cerrado-entrante.
var ContadorErroresCortacircuitos atomic.Int64

// normalizarParaBaja recorta espacios, pasa a minúsculas y elimina signos de puntuación circundantes.
func normalizarParaBaja(texto string) string {
	return strings.TrimFunc(strings.ToLower(strings.TrimSpace(texto)), func(r rune) bool {
		return unicode.IsPunct(r) || unicode.IsSpace(r)
	})
}

// contieneFrustracion verifica si el texto contiene alguna de las palabras clave de frustración
// mediante coincidencia de palabra completa (tokenización), evitando falsos positivos por subcadenas.
// [causa documentada]
func contieneFrustracion(texto string, palabras map[string]struct{}) bool {
	if len(palabras) == 0 {
		return false
	}
	if _, ok := palabras[normalizarParaBaja(texto)]; ok {
		return true
	}
	tokens := strings.FieldsFunc(texto, func(r rune) bool {
		return unicode.IsSpace(r) || unicode.IsPunct(r)
	})
	for _, tok := range tokens {
		if norm := normalizarParaBaja(tok); norm != "" {
			if _, ok := palabras[norm]; ok {
				return true
			}
		}
	}
	return false
}

// DetectorDeBaja detecta palabras clave de baja en mensajes entrantes y procesa su confirmación.
type DetectorDeBaja struct {
	palabras          map[string]struct{}
	textoConfirmacion string
	control           ControlDeBaja
	admisor           AdmisorDeConfirmacion
}

// NuevoDetectorDeBaja inicializa el detector con las palabras clave normalizadas y el texto configurado.
func NuevoDetectorDeBaja(palabras []string, textoConfirmacion string, control ControlDeBaja, admisor AdmisorDeConfirmacion) *DetectorDeBaja {
	mapa := make(map[string]struct{}, len(palabras))
	for _, p := range palabras {
		if norm := normalizarParaBaja(p); norm != "" {
			mapa[norm] = struct{}{}
		}
	}
	return &DetectorDeBaja{palabras: mapa, textoConfirmacion: textoConfirmacion, control: control, admisor: admisor}
}

// Coincide verifica si el texto coincide exactamente con alguna palabra clave de baja tras normalización.
func (d *DetectorDeBaja) Coincide(texto string) bool {
	if d == nil || len(d.palabras) == 0 {
		return false
	}
	_, ok := d.palabras[normalizarParaBaja(texto)]
	return ok
}

// RegistrarBaja delega el registro de la baja en el control inyectado.
func (d *DetectorDeBaja) RegistrarBaja(ctx context.Context, idInterno string, dadaDeBajaEnMs int64) (bool, error) {
	if d == nil || d.control == nil {
		return false, nil
	}
	return d.control.RegistrarBaja(ctx, idInterno, dadaDeBajaEnMs)
}

// ProcesarConfirmacion delega la admisión de la confirmación en el admisor inyectado.
func (d *DetectorDeBaja) ProcesarConfirmacion(ctx context.Context, idInterno string, marcaTemporalMs int64) (bool, error) {
	if d == nil || d.admisor == nil {
		return false, nil
	}
	return d.admisor.AdmitirConfirmacionDeBaja(ctx, "conf-"+idInterno, idInterno, d.textoConfirmacion, marcaTemporalMs)
}

// DetectorDeCortacircuitos detecta repetición y frustración en mensajes entrantes y procesa su traspaso a humano.
// [causa documentada]
type DetectorDeCortacircuitos struct {
	umbralRepeticion    int64
	palabrasFrustracion map[string]struct{}
	textoTraspaso       string
	control             ControlDeCortacircuitos
	admisor             AdmisorDeTraspaso
}

// NuevoDetectorDeCortacircuitos inicializa el detector con los parámetros del cortacircuitos.
// [causa documentada]
func NuevoDetectorDeCortacircuitos(
	umbralRepeticion int64,
	palabrasFrustracion []string,
	textoTraspaso string,
	control ControlDeCortacircuitos,
	admisor AdmisorDeTraspaso,
) *DetectorDeCortacircuitos {
	mapa := make(map[string]struct{}, len(palabrasFrustracion))
	for _, p := range palabrasFrustracion {
		if norm := normalizarParaBaja(p); norm != "" {
			mapa[norm] = struct{}{}
		}
	}
	return &DetectorDeCortacircuitos{
		umbralRepeticion:    umbralRepeticion,
		palabrasFrustracion: mapa,
		textoTraspaso:       textoTraspaso,
		control:             control,
		admisor:             admisor,
	}
}

// Observar registra la observación del mensaje entrante y devuelve si esta llamada disparó el cortacircuitos.
func (d *DetectorDeCortacircuitos) Observar(ctx context.Context, idInterno, texto string, ahoraMs int64) (bool, error) {
	if d == nil || d.control == nil {
		return false, nil
	}
	textoNorm := normalizarParaBaja(texto)
	esFrustracion := contieneFrustracion(texto, d.palabrasFrustracion)
	return d.control.RegistrarObservacion(ctx, idInterno, textoNorm, esFrustracion, d.umbralRepeticion, ahoraMs)
}

// EmitirTraspaso delega la admisión del mensaje único de traspaso en el admisor inyectado.
func (d *DetectorDeCortacircuitos) EmitirTraspaso(ctx context.Context, idInterno string, marcaTemporalMs int64) (bool, error) {
	if d == nil || d.admisor == nil {
		return false, nil
	}
	return d.admisor.AdmitirMensajeDeTraspaso(ctx, "traspaso-"+idInterno, idInterno, d.textoTraspaso, marcaTemporalMs)
}

// Traductor convierte eventos entrantes de whatsmeow en EventoEntrante de IPC
// asegurando persist-first y deduplicación.
type Traductor struct {
	almacen               *identidad.Almacen
	buzon                 BuzonDurable
	sumidero              SumideroDeEvento
	resolutor             ResolutorDeAlias
	registro              *registro.Registro
	detectorBaja          *DetectorDeBaja
	detectorCortacircuito *DetectorDeCortacircuitos
	// ahoraMs es el reloj del traductor, inyectable en tests. Los mensajes conservan su marca
	// de origen, pero la CONFIRMACIÓN de baja y el TRASPASO de cortacircuitos se encolan con este reloj:
	// si usaran la marca del mensaje entrante, un mensaje entregado del backlog offline tras una
	// caída mayor que el TTL de salida nacería ya expirado y, con el reclamo único gastado, jamás se reencolaría.
	ahoraMs func() int64
}

// NuevoTraductor construye un Traductor con las dependencias inyectadas.
func NuevoTraductor(
	almacen *identidad.Almacen,
	buzon BuzonDurable,
	sumidero SumideroDeEvento,
	resolutor ResolutorDeAlias,
	reg *registro.Registro,
	baja *DetectorDeBaja,
	cortacircuitos *DetectorDeCortacircuitos,
) *Traductor {
	return &Traductor{
		almacen:               almacen,
		buzon:                 buzon,
		sumidero:              sumidero,
		resolutor:             resolutor,
		registro:              reg,
		detectorBaja:          baja,
		detectorCortacircuito: cortacircuitos,
		ahoraMs:               func() int64 { return time.Now().UnixMilli() },
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

	esBaja := t.detectorBaja != nil && t.detectorBaja.Coincide(texto)
	var disparoCortacircuitos bool
	if esBaja {
		if t.registro != nil {
			t.registro.Info(EventoBajaDetectada, registro.Campos{IdEvento: ident.IdInterno})
		}
		if _, err := t.detectorBaja.RegistrarBaja(ctx, ident.IdInterno, evt.Info.Timestamp.UnixMilli()); err != nil && t.registro != nil {
			t.registro.Error("canal.error_registro_baja", registro.Campos{Detalle: err.Error()})
		}
	} else if t.detectorCortacircuito != nil {
		disparado, err := t.detectorCortacircuito.Observar(ctx, ident.IdInterno, texto, evt.Info.Timestamp.UnixMilli())
		if err != nil {
			// Fallo CERRADO (LES-2026-08-11-000000025, contract behavior 7): un error de lectura
			// o escritura del estado del cortacircuitos nunca se trata como "no disparado". No se
			// persiste ni se reenvía este mensaje al núcleo -igual que los demás errores duros de
			// esta función (resolución, codificación, persistencia)-, así que no puede generarse
			// una respuesta automática para él; el mensaje se pierde en silencio, que es el lado
			// seguro (silencio recuperable) frente a responder como si el cortacircuitos no
			// existiera. (fix cortacircuitos-fallo-cerrado-entrante)
			ContadorErroresCortacircuitos.Add(1)
			if t.registro != nil {
				t.registro.Error(EventoErrorCortacircuitos, registro.Campos{IdEvento: ident.IdInterno, Detalle: err.Error()})
			}
			return
		} else if disparado {
			disparoCortacircuitos = true
			if t.registro != nil {
				t.registro.Info(EventoCortacircuitosDisparado, registro.Campos{IdEvento: ident.IdInterno})
			}
		}
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

	if esBaja {
		// La confirmación se encola con el reloj actual, NUNCA con la marca del mensaje
		// entrante: un STOP del backlog offline más viejo que el TTL de salida produciría
		// una fila ya expirada e irrecuperable (el reclamo único ya estaría gastado).
		if _, err := t.detectorBaja.ProcesarConfirmacion(ctx, ident.IdInterno, t.ahoraMs()); err != nil && t.registro != nil {
			t.registro.Error("canal.error_confirmacion_baja", registro.Campos{Detalle: err.Error()})
		}
	} else if disparoCortacircuitos && t.detectorCortacircuito != nil {
		if _, err := t.detectorCortacircuito.EmitirTraspaso(ctx, ident.IdInterno, t.ahoraMs()); err != nil && t.registro != nil {
			t.registro.Error(EventoErrorEmisionTraspaso, registro.Campos{IdEvento: ident.IdInterno, Detalle: err.Error()})
		}
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
