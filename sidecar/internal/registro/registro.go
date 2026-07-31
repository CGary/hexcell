// Package registro emite el registro estructurado del sidecar: un objeto JSON por línea, con el
// mismo conjunto cerrado de campos que `crates/hexcell/src/registro.rs` usa del lado Rust
// (`docs/adr/adr-0019-registro-estructurado.md`).
//
// # Por qué log/slog y ninguna biblioteca de registro
//
// adr-0019 rechazó `tracing` más una capa JSON en el núcleo por presupuesto de memoria (NFR-01,
// ≤ 80 MB por célula sobre canal propio). El mismo argumento vale aquí: `log/slog` con
// `slog.JSONHandler` produce una línea JSON por evento sin arrastrar ninguna dependencia.
//
// # El conjunto de campos es el mecanismo de privacidad
//
// Los campos son los de adr-0019: `nivel`, `evento`, `id_celula`, `id_evento`, `id_conversacion`,
// `latencia_ms` y `detalle`. `evento` nombra el suceso y se escribe siempre como constante del
// código; `detalle` es el único campo de texto libre y está reservado al texto del propio proceso
// —una ruta, un error de almacenamiento, una línea de la biblioteca—, nunca al de un mensaje.
//
// El campo de tiempo que `slog` añade por omisión se **descarta** a propósito: el lado Rust no lo
// emite, el runtime de contenedores ya estampa cada línea de `stdout`, y mantener los dos extremos
// con el mismo conjunto de campos vale más que un segundo reloj dentro de la línea.
package registro

import (
	"fmt"
	"io"
	"log/slog"
	"strings"

	waLog "go.mau.fi/whatsmeow/util/log"
)

// Nombres de los campos del conjunto cerrado. Se declaran como constantes para que los tests
// puedan comprobar el conjunto exacto sin repetir cadenas sueltas.
const (
	CampoNivel          = "nivel"
	CampoEvento         = "evento"
	CampoIdCelula       = "id_celula"
	CampoIdEvento       = "id_evento"
	CampoIdConversacion = "id_conversacion"
	CampoLatenciaMs     = "latencia_ms"
	CampoDetalle        = "detalle"
)

// EventoWhatsmeow es el nombre fijo de suceso de toda línea que nace dentro de la biblioteca
// whatsmeow. Es constante a propósito: ningún texto de la biblioteca puede acabar en `evento`.
const EventoWhatsmeow = "canal.whatsmeow"

// nombresDeNivel traduce el nivel de slog al vocabulario en español de adr-0019.
var nombresDeNivel = map[slog.Level]string{
	slog.LevelDebug: "depuracion",
	slog.LevelInfo:  "info",
	slog.LevelWarn:  "aviso",
	slog.LevelError: "error",
}

// Campos son los campos opcionales de una entrada. Los que queden en su valor cero no se emiten.
//
// Es una estructura cerrada y no un mapa de clave-valor libre por el mismo motivo que adr-0019
// descartó el mapa del lado Rust: un mapa admite cualquier clave, incluida una que alguien llame
// `mensaje` o `contenido` sin que nada lo impida.
type Campos struct {
	// IdEvento es el identificador opaco del evento entrante, cuando aplica.
	IdEvento string
	// IdConversacion es el identificador interno del hilo, cuando aplica. Nunca un JID.
	IdConversacion string
	// LatenciaMs es una medida de latencia en milisegundos, cuando aplica.
	LatenciaMs int64
	// Detalle es el único campo de texto libre, reservado al texto del propio proceso.
	Detalle string
}

// comoAtributos convierte los campos poblados en atributos de slog, en el orden de adr-0019.
func (c Campos) comoAtributos() []any {
	atributos := make([]any, 0, 4)
	if c.IdEvento != "" {
		atributos = append(atributos, slog.String(CampoIdEvento, c.IdEvento))
	}
	if c.IdConversacion != "" {
		atributos = append(atributos, slog.String(CampoIdConversacion, c.IdConversacion))
	}
	if c.LatenciaMs != 0 {
		atributos = append(atributos, slog.Int64(CampoLatenciaMs, c.LatenciaMs))
	}
	if c.Detalle != "" {
		atributos = append(atributos, slog.String(CampoDetalle, c.Detalle))
	}
	return atributos
}

// Registro emite líneas de registro estructurado con el identificador de célula ya estampado.
type Registro struct {
	registrador *slog.Logger
	umbral      slog.Level
}

// Nuevo construye un registro que escribe una línea JSON por entrada sobre la salida dada.
//
// El identificador de célula se estampa una sola vez aquí y aparece en toda línea posterior, del
// mismo modo que `registro::inicializar` lo fija en el binario Rust.
func Nuevo(salida io.Writer, umbral slog.Level, idCelula string) *Registro {
	manejador := slog.NewJSONHandler(salida, &slog.HandlerOptions{
		Level:       umbral,
		ReplaceAttr: renombrarAlVocabularioDeAdr0019,
	})
	return &Registro{
		registrador: slog.New(manejador).With(slog.String(CampoIdCelula, idCelula)),
		umbral:      umbral,
	}
}

// renombrarAlVocabularioDeAdr0019 descarta la marca de tiempo de slog y renombra sus dos campos
// propios (`time`, `level`, `msg`) al conjunto cerrado del ADR.
func renombrarAlVocabularioDeAdr0019(_ []string, atributo slog.Attr) slog.Attr {
	switch atributo.Key {
	case slog.TimeKey:
		return slog.Attr{}
	case slog.LevelKey:
		nivel, esNivel := atributo.Value.Any().(slog.Level)
		if !esNivel {
			return atributo
		}
		nombre, conocido := nombresDeNivel[nivel]
		if !conocido {
			nombre = nivel.String()
		}
		return slog.String(CampoNivel, nombre)
	case slog.MessageKey:
		return slog.String(CampoEvento, atributo.Value.String())
	default:
		return atributo
	}
}

// Umbral devuelve el nivel por debajo del cual este registro no emite nada.
func (r *Registro) Umbral() slog.Level {
	return r.umbral
}

// Depuracion emite una entrada de depuración. Solo sale si el umbral configurado la admite.
func (r *Registro) Depuracion(evento string, campos Campos) {
	r.registrador.Debug(evento, campos.comoAtributos()...)
}

// Info emite una entrada de progreso normal.
func (r *Registro) Info(evento string, campos Campos) {
	r.registrador.Info(evento, campos.comoAtributos()...)
}

// Aviso emite una entrada de degradación que no detiene el proceso.
func (r *Registro) Aviso(evento string, campos Campos) {
	r.registrador.Warn(evento, campos.comoAtributos()...)
}

// Error emite una entrada de operación fallida.
func (r *Registro) Error(evento string, campos Campos) {
	r.registrador.Error(evento, campos.comoAtributos()...)
}

// AdaptadorWaLog implementa la interfaz `waLog.Logger` de whatsmeow sobre este registro.
//
// # Por qué la salida de depuración se corta aquí
//
// whatsmeow escribe por `Debugf` el detalle de los nodos del protocolo, y esos nodos **pueden
// llevar contenido de mensaje**. adr-0019 hace de esa ausencia una frontera estructural de
// privacidad y no una convención, así que reenviar `Debugf` sin filtro la rompería por el lado Go.
// Este adaptador descarta esas líneas mientras el umbral esté por encima de depuración.
//
// El límite de la garantía queda escrito, no disimulado: `Infof`, `Warnf` y `Errorf` sí se
// reenvían, y aunque describen transiciones de conexión y no contenido, nada del tipo lo impide.
// Bajar el umbral a depuración en producción reintroduce el riesgo a sabiendas.
type AdaptadorWaLog struct {
	registro *Registro
	modulo   string
}

// Comprobación en tiempo de compilación de que el puente satisface la interfaz de whatsmeow.
var _ waLog.Logger = (*AdaptadorWaLog)(nil)

// NuevoAdaptadorWaLog construye el puente hacia whatsmeow para el módulo indicado.
func NuevoAdaptadorWaLog(registro *Registro, modulo string) *AdaptadorWaLog {
	return &AdaptadorWaLog{registro: registro, modulo: modulo}
}

// Modulo devuelve la ruta de módulos acumulada por las llamadas a Sub.
func (a *AdaptadorWaLog) Modulo() string {
	return a.modulo
}

// detallar compone el texto de la biblioteca prefijado por su módulo. El resultado va siempre al
// campo `detalle`, nunca a `evento`.
func (a *AdaptadorWaLog) detallar(mensaje string, argumentos ...any) string {
	texto := fmt.Sprintf(mensaje, argumentos...)
	if a.modulo == "" {
		return texto
	}
	return a.modulo + ": " + texto
}

// Debugf descarta la línea salvo que el umbral configurado sea el de depuración.
func (a *AdaptadorWaLog) Debugf(mensaje string, argumentos ...any) {
	if a.registro.Umbral() > slog.LevelDebug {
		return
	}
	a.registro.Depuracion(EventoWhatsmeow, Campos{Detalle: a.detallar(mensaje, argumentos...)})
}

// Infof reenvía una línea informativa de la biblioteca.
func (a *AdaptadorWaLog) Infof(mensaje string, argumentos ...any) {
	a.registro.Info(EventoWhatsmeow, Campos{Detalle: a.detallar(mensaje, argumentos...)})
}

// Warnf reenvía un aviso de la biblioteca.
func (a *AdaptadorWaLog) Warnf(mensaje string, argumentos ...any) {
	a.registro.Aviso(EventoWhatsmeow, Campos{Detalle: a.detallar(mensaje, argumentos...)})
}

// Errorf reenvía un error de la biblioteca.
func (a *AdaptadorWaLog) Errorf(mensaje string, argumentos ...any) {
	a.registro.Error(EventoWhatsmeow, Campos{Detalle: a.detallar(mensaje, argumentos...)})
}

// Sub devuelve un adaptador para un submódulo, con la ruta de módulos acumulada.
func (a *AdaptadorWaLog) Sub(modulo string) waLog.Logger {
	if a.modulo == "" {
		return &AdaptadorWaLog{registro: a.registro, modulo: modulo}
	}
	return &AdaptadorWaLog{registro: a.registro, modulo: strings.Join([]string{a.modulo, modulo}, "/")}
}
