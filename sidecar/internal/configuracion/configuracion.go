// Package configuracion lee del entorno los pocos parámetros que el sidecar necesita para
// arrancar y los valida antes de que nada más se construya.
//
// El tipo [Configuracion] es un objeto de valor: se construye una vez en el arranque, ya
// validado, y no cambia después. Nadie lee variables de entorno fuera de este paquete, de modo
// que la lista completa de lo que el proceso configura cabe en un archivo y se puede documentar
// entera.
package configuracion

import (
	"errors"
	"fmt"
	"log/slog"
)

// Nombres de las variables de entorno que el sidecar reconoce. No hay más.
const (
	// VariableSocket fija la ruta del socket de dominio Unix del protocolo IPC.
	VariableSocket = "HEXCELL_SOCKET_IPC"
	// VariableNivelDeRegistro fija el umbral del registro estructurado.
	VariableNivelDeRegistro = "HEXCELL_NIVEL_REGISTRO"
	// VariableIdCelula fija el identificador opaco de la célula estampado en cada línea.
	VariableIdCelula = "HEXCELL_ID_CELULA"
)

// Valores por omisión, documentados en docs/protocolo-ipc-nucleo-sidecar.md, sección 2.
const (
	// RutaSocketPorOmision es la ruta del socket dentro del volumen compartido de la célula.
	RutaSocketPorOmision = "/var/lib/hexcell/ipc/sidecar.sock"
	// NivelDeRegistroPorOmision deja fuera del registro las líneas de depuración, que son las
	// únicas que whatsmeow puede llenar con contenido de mensaje.
	NivelDeRegistroPorOmision = "info"
	// IdCelulaPorOmision documenta el caso de un arranque sin identificador configurado en vez
	// de abortarlo: una célula sin nombre sigue siendo diagnosticable, solo peor.
	IdCelulaPorOmision = "sin-configurar"
)

// ErrRutaSocketVacia se devuelve cuando la variable del socket está definida pero vacía.
//
// Ausente y vacía no son el mismo caso: ausente significa «usa el valor por omisión», mientras que
// vacía es casi siempre una plantilla de despliegue que no sustituyó su marcador. Arrancar así
// dejaría al sidecar escuchando en ningún sitio y al núcleo reintentando para siempre.
var ErrRutaSocketVacia = errors.New("configuracion: la ruta del socket IPC está vacía")

// ErrNivelDeRegistroDesconocido se devuelve ante un umbral de registro que no está en la tabla.
var ErrNivelDeRegistroDesconocido = errors.New("configuracion: nivel de registro desconocido")

// nivelesReconocidos es el conjunto cerrado de umbrales admitidos, en español como el resto del
// repositorio. Un valor fuera de la tabla es un error y no se degrada en silencio a «info».
var nivelesReconocidos = map[string]slog.Level{
	"depuracion": slog.LevelDebug,
	"info":       slog.LevelInfo,
	"aviso":      slog.LevelWarn,
	"error":      slog.LevelError,
}

// Configuracion son los parámetros de arranque del sidecar, ya validados.
type Configuracion struct {
	// RutaSocket es la ruta del socket de dominio Unix sobre el volumen compartido.
	RutaSocket string
	// NivelDeRegistro es el umbral por debajo del cual no se emite ninguna línea. El puente a
	// whatsmeow lo usa además como corte de su salida de depuración.
	NivelDeRegistro slog.Level
	// IdCelula es el identificador opaco estampado en cada línea de registro.
	IdCelula string
}

// Cargar construye la configuración a partir de una función de consulta del entorno.
//
// La función se recibe como parámetro —con la misma forma que os.LookupEnv— en lugar de leerse
// de os directamente: así los tests fijan un entorno completo sin mutar el del proceso, que es
// estado global compartido entre tests que corren en paralelo.
func Cargar(consultar func(string) (string, bool)) (Configuracion, error) {
	rutaSocket := RutaSocketPorOmision
	if valor, presente := consultar(VariableSocket); presente {
		if valor == "" {
			return Configuracion{}, ErrRutaSocketVacia
		}
		rutaSocket = valor
	}

	nombreNivel := NivelDeRegistroPorOmision
	if valor, presente := consultar(VariableNivelDeRegistro); presente && valor != "" {
		nombreNivel = valor
	}
	nivel, reconocido := nivelesReconocidos[nombreNivel]
	if !reconocido {
		return Configuracion{}, fmt.Errorf("%w: %q", ErrNivelDeRegistroDesconocido, nombreNivel)
	}

	idCelula := IdCelulaPorOmision
	if valor, presente := consultar(VariableIdCelula); presente && valor != "" {
		idCelula = valor
	}

	return Configuracion{
		RutaSocket:      rutaSocket,
		NivelDeRegistro: nivel,
		IdCelula:        idCelula,
	}, nil
}
