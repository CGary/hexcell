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
	"strconv"
)

// Nombres de las variables de entorno que el sidecar reconoce. No hay más.
const (
	// VariableSocket fija la ruta del socket de dominio Unix del protocolo IPC.
	VariableSocket = "HEXCELL_SOCKET_IPC"
	// VariableNivelDeRegistro fija el umbral del registro estructurado.
	VariableNivelDeRegistro = "HEXCELL_NIVEL_REGISTRO"
	// VariableIdCelula fija el identificador opaco de la célula estampado en cada línea.
	VariableIdCelula = "HEXCELL_ID_CELULA"
	// VariableRutaSqlstore fija la ruta del archivo de la base de datos sqlstore de whatsmeow.
	VariableRutaSqlstore = "HEXCELL_RUTA_SQLSTORE"
	// VariableRutaIdentidad fija la ruta del almacén de identidad.
	VariableRutaIdentidad = "HEXCELL_RUTA_IDENTIDAD"
	// VariableTelefonoCelula fija el número de teléfono de la célula, sin el prefijo +,
	// necesario para el emparejamiento por código de vinculación. Nunca viaja en el cable IPC.
	VariableTelefonoCelula = "HEXCELL_TELEFONO_CELULA"
	// VariableRetrocesoInicialMs fija el intervalo inicial de retroceso exponencial, en milisegundos.
	VariableRetrocesoInicialMs = "HEXCELL_RETROCESO_INICIAL_MS"
	// VariableRetrocesoFactor fija el multiplicador entero del retroceso exponencial.
	VariableRetrocesoFactor = "HEXCELL_RETROCESO_FACTOR"
	// VariableRetrocesoMaximoMs fija el techo del retroceso exponencial, en milisegundos.
	VariableRetrocesoMaximoMs = "HEXCELL_RETROCESO_MAXIMO_MS"
	// VariableRetrocesoBaneoInicialMs fija el intervalo inicial de retroceso largo por baneo temporal.
	VariableRetrocesoBaneoInicialMs = "HEXCELL_RETROCESO_BANEO_INICIAL_MS"
	// VariableRetrocesoBaneoMaximoMs fija el techo del retroceso largo por baneo temporal.
	VariableRetrocesoBaneoMaximoMs = "HEXCELL_RETROCESO_BANEO_MAXIMO_MS"
	// VariableTtlSalidaMs fija el tiempo máximo de vida de un mensaje saliente antes de expirar.
	VariableTtlSalidaMs = "HEXCELL_TTL_SALIDA_MS"
	// VariableIntentosMaximosSalida fija el máximo de intentos de entrega de un mensaje saliente.
	VariableIntentosMaximosSalida = "HEXCELL_INTENTOS_MAXIMOS_SALIDA"
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
	// RutaSqlstorePorOmision es la ruta del archivo sqlstore en el volumen compartido.
	RutaSqlstorePorOmision = "/var/lib/hexcell/sqlstore.db"
	// RutaIdentidadPorOmision es la ruta del archivo del almacén de identidad en el volumen compartido.
	RutaIdentidadPorOmision = "/var/lib/hexcell/identidad.db"
	// RetrocesoInicialMsPorOmision es el intervalo inicial del retroceso exponencial.
	// PENDIENTE DE CALIBRACIÓN: valor inicial razonable, no validado bajo carga.
	RetrocesoInicialMsPorOmision int64 = 1000
	// RetrocesoFactorPorOmision es el multiplicador entero del retroceso.
	// PENDIENTE DE CALIBRACIÓN.
	RetrocesoFactorPorOmision int64 = 2
	// RetrocesoMaximoMsPorOmision es el techo del retroceso exponencial.
	// PENDIENTE DE CALIBRACIÓN.
	RetrocesoMaximoMsPorOmision int64 = 60000
	// RetrocesoBaneoInicialMsPorOmision es el intervalo inicial del retroceso largo por baneo.
	// PENDIENTE DE CALIBRACIÓN.
	RetrocesoBaneoInicialMsPorOmision int64 = 30000
	// RetrocesoBaneoMaximoMsPorOmision es el techo del retroceso largo por baneo.
	// PENDIENTE DE CALIBRACIÓN.
	RetrocesoBaneoMaximoMsPorOmision int64 = 300000
	// TtlSalidaMsPorOmision es el TTL por omisión para mensajes salientes (15 min). Es la única
	// fuente de este literal: sidecar/internal/outbox reexporta esta misma constante como
	// outbox.TtlPorOmision en vez de repetirla.
	// PENDIENTE DE CALIBRACIÓN: punto de partida razonable, no validado bajo tráfico real.
	TtlSalidaMsPorOmision int64 = 900000
	// IntentosMaximosSalidaPorOmision es el límite de intentos por omisión. Es la única fuente
	// de este literal: sidecar/internal/outbox reexporta esta misma constante como
	// outbox.IntentosMaximosPorOmision en vez de repetirla.
	// PENDIENTE DE CALIBRACIÓN.
	IntentosMaximosSalidaPorOmision int64 = 3
)

// ErrRutaSocketVacia se devuelve cuando la variable del socket está definida pero vacía.
//
// Ausente y vacía no son el mismo caso: ausente significa «usa el valor por omisión», mientras que
// vacía es casi siempre una plantilla de despliegue que no sustituyó su marcador. Arrancar así
// dejaría al sidecar escuchando en ningún sitio y al núcleo reintentando para siempre.
var ErrRutaSocketVacia = errors.New("configuracion: la ruta del socket IPC está vacía")

// ErrRutaSqlstoreVacia se devuelve cuando la variable del sqlstore está definida pero vacía.
var ErrRutaSqlstoreVacia = errors.New("configuracion: la ruta del sqlstore está vacía")

// ErrRutaIdentidadVacia se devuelve cuando la variable del almacén de identidad está definida pero vacía.
var ErrRutaIdentidadVacia = errors.New("configuracion: la ruta del almacén de identidad está vacía")

// ErrNivelDeRegistroDesconocido se devuelve ante un umbral de registro que no está en la tabla.
var ErrNivelDeRegistroDesconocido = errors.New("configuracion: nivel de registro desconocido")

// ErrRetrocesoInvalido se devuelve cuando un parámetro de retroceso no es numérico, es cero,
// negativo o el techo es menor que el intervalo inicial.
var ErrRetrocesoInvalido = errors.New("configuracion: parámetro de retroceso inválido")

// ErrParametroSalidaInvalido se devuelve cuando un parámetro de salida es inválido.
var ErrParametroSalidaInvalido = errors.New("configuracion: parámetro de salida inválido")

// nivelesReconocidos es el conjunto cerrado de umbrales admitidos, en español como el resto del
// repositorio. Un valor fuera de la tabla es un error y no se degrada en silencio a «info».
var nivelesReconocidos = map[string]slog.Level{
	"depuracion": slog.LevelDebug,
	"info":       slog.LevelInfo,
	"aviso":      slog.LevelWarn,
	"error":      slog.LevelError,
}

// Retroceso agrupa los parámetros de retroceso exponencial del sidecar.
// Todos los intervalos están en milisegundos y el factor es un entero, porque este
// repositorio no admite punto flotante en ningún sitio y el protocolo IPC lo prohíbe.
type Retroceso struct {
	// IntervaloInicial es el primer intervalo de espera, en milisegundos.
	IntervaloInicial int64
	// Factor es el multiplicador entero que se aplica en cada intento.
	Factor int64
	// IntervaloMaximo es el techo del retroceso exponencial, en milisegundos.
	IntervaloMaximo int64
	// BaneoInicial es el primer intervalo de espera tras un baneo temporal, en milisegundos.
	BaneoInicial int64
	// BaneoMaximo es el techo del retroceso largo por baneo temporal, en milisegundos.
	BaneoMaximo int64
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
	// RutaSqlstore es la ruta del archivo de la base de datos sqlstore de whatsmeow.
	RutaSqlstore string
	// RutaIdentidad es la ruta del archivo del almacén de identidad.
	RutaIdentidad string
	// TelefonoCelula es el número de teléfono de la célula, sin prefijo +. Solo es necesario
	// para el emparejamiento por código de vinculación. Vacío es válido: significa que ese
	// método no está disponible.
	TelefonoCelula string
	// Retroceso agrupa los parámetros de retroceso exponencial.
	Retroceso Retroceso
	// TtlSalidaMs es el tiempo máximo de vida de un mensaje saliente antes de expirar.
	TtlSalidaMs int64
	// IntentosMaximosSalida es el número máximo de intentos para enviar un mensaje saliente.
	IntentosMaximosSalida int64
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

	rutaSqlstore := RutaSqlstorePorOmision
	if valor, presente := consultar(VariableRutaSqlstore); presente {
		if valor == "" {
			return Configuracion{}, ErrRutaSqlstoreVacia
		}
		rutaSqlstore = valor
	}

	rutaIdentidad := RutaIdentidadPorOmision
	if valor, presente := consultar(VariableRutaIdentidad); presente {
		if valor == "" {
			return Configuracion{}, ErrRutaIdentidadVacia
		}
		rutaIdentidad = valor
	}

	telefonoCelula := ""
	if valor, presente := consultar(VariableTelefonoCelula); presente && valor != "" {
		telefonoCelula = valor
	}

	retroceso, err := cargarRetroceso(consultar)
	if err != nil {
		return Configuracion{}, err
	}

	ttlSalida, err := enteroDelEntorno(consultar, VariableTtlSalidaMs, TtlSalidaMsPorOmision)
	if err != nil {
		return Configuracion{}, err
	}
	if ttlSalida <= 0 {
		return Configuracion{}, fmt.Errorf("%w: %s debe ser positivo, recibido %d", ErrParametroSalidaInvalido, VariableTtlSalidaMs, ttlSalida)
	}

	intentosSalida, err := enteroDelEntorno(consultar, VariableIntentosMaximosSalida, IntentosMaximosSalidaPorOmision)
	if err != nil {
		return Configuracion{}, err
	}
	if intentosSalida <= 0 {
		return Configuracion{}, fmt.Errorf("%w: %s debe ser positivo, recibido %d", ErrParametroSalidaInvalido, VariableIntentosMaximosSalida, intentosSalida)
	}

	return Configuracion{
		RutaSocket:            rutaSocket,
		NivelDeRegistro:       nivel,
		IdCelula:              idCelula,
		RutaSqlstore:          rutaSqlstore,
		RutaIdentidad:         rutaIdentidad,
		TelefonoCelula:        telefonoCelula,
		Retroceso:             retroceso,
		TtlSalidaMs:           ttlSalida,
		IntentosMaximosSalida: intentosSalida,
	}, nil
}

// cargarRetroceso lee y valida los cinco parámetros de retroceso del entorno.
func cargarRetroceso(consultar func(string) (string, bool)) (Retroceso, error) {
	inicial, err := enteroDelEntorno(consultar, VariableRetrocesoInicialMs, RetrocesoInicialMsPorOmision)
	if err != nil {
		return Retroceso{}, err
	}
	factor, err := enteroDelEntorno(consultar, VariableRetrocesoFactor, RetrocesoFactorPorOmision)
	if err != nil {
		return Retroceso{}, err
	}
	maximo, err := enteroDelEntorno(consultar, VariableRetrocesoMaximoMs, RetrocesoMaximoMsPorOmision)
	if err != nil {
		return Retroceso{}, err
	}
	baneoInicial, err := enteroDelEntorno(consultar, VariableRetrocesoBaneoInicialMs, RetrocesoBaneoInicialMsPorOmision)
	if err != nil {
		return Retroceso{}, err
	}
	baneoMaximo, err := enteroDelEntorno(consultar, VariableRetrocesoBaneoMaximoMs, RetrocesoBaneoMaximoMsPorOmision)
	if err != nil {
		return Retroceso{}, err
	}

	// Validación: ningún valor puede ser cero o negativo.
	for _, par := range []struct {
		nombre string
		valor  int64
	}{
		{VariableRetrocesoInicialMs, inicial},
		{VariableRetrocesoFactor, factor},
		{VariableRetrocesoMaximoMs, maximo},
		{VariableRetrocesoBaneoInicialMs, baneoInicial},
		{VariableRetrocesoBaneoMaximoMs, baneoMaximo},
	} {
		if par.valor <= 0 {
			return Retroceso{}, fmt.Errorf("%w: %s debe ser positivo, recibido %d", ErrRetrocesoInvalido, par.nombre, par.valor)
		}
	}

	// Validación: el techo no puede ser menor que el intervalo inicial.
	if maximo < inicial {
		return Retroceso{}, fmt.Errorf("%w: %s (%d) es menor que %s (%d)",
			ErrRetrocesoInvalido, VariableRetrocesoMaximoMs, maximo, VariableRetrocesoInicialMs, inicial)
	}
	if baneoMaximo < baneoInicial {
		return Retroceso{}, fmt.Errorf("%w: %s (%d) es menor que %s (%d)",
			ErrRetrocesoInvalido, VariableRetrocesoBaneoMaximoMs, baneoMaximo, VariableRetrocesoBaneoInicialMs, baneoInicial)
	}

	return Retroceso{
		IntervaloInicial: inicial,
		Factor:           factor,
		IntervaloMaximo:  maximo,
		BaneoInicial:     baneoInicial,
		BaneoMaximo:      baneoMaximo,
	}, nil
}

// enteroDelEntorno lee un valor entero de una variable de entorno, usando el valor por omisión
// si la variable está ausente o vacía.
func enteroDelEntorno(consultar func(string) (string, bool), variable string, porOmision int64) (int64, error) {
	valor, presente := consultar(variable)
	if !presente || valor == "" {
		return porOmision, nil
	}
	entero, err := strconv.ParseInt(valor, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("%w: %s no es un entero válido: %q", ErrRetrocesoInvalido, variable, valor)
	}
	return entero, nil
}
