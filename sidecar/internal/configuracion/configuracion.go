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
	"strings"
	"time"
	_ "time/tzdata"
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
	// VariablePalabrasDeBaja fija la lista de palabras clave (separadas por coma) para opt-out.
	VariablePalabrasDeBaja = "HEXCELL_PALABRAS_DE_BAJA"
	// VariableTextoConfirmacionDeBaja fija el texto de la única confirmación tras la baja.
	VariableTextoConfirmacionDeBaja = "HEXCELL_TEXTO_CONFIRMACION_BAJA"
	// VariableLatenciaMinimaMs fija el suelo de latencia mínima de respuesta antes de transmitir, en milisegundos.
	// [causa documentada]
	VariableLatenciaMinimaMs = "HEXCELL_LATENCIA_MINIMA_MS"
	// VariableIntervaloDrenajeMs fija la cadencia del bucle de drenaje de salida, en milisegundos.
	VariableIntervaloDrenajeMs = "HEXCELL_INTERVALO_DRENAJE_MS"
	// VariableVentanaApertura fija la hora de apertura de la ventana de atención (formato HH:MM).
	// [causa documentada]
	VariableVentanaApertura = "HEXCELL_VENTANA_APERTURA"
	// VariableVentanaCierre fija la hora de cierre de la ventana de atención (formato HH:MM).
	// [causa documentada]
	VariableVentanaCierre = "HEXCELL_VENTANA_CIERRE"
	// VariableVentanaDias fija los días de atención como lista de enteros ISO 1..7 separados por coma.
	// [causa documentada]
	VariableVentanaDias = "HEXCELL_VENTANA_DIAS"
	// VariableVentanaZona fija la zona horaria IANA de la ventana de atención.
	// [causa documentada]
	VariableVentanaZona = "HEXCELL_VENTANA_ZONA"
	// VariableRampaDiariaInicial fija el cupo diario inicial de envíos durante la primera semana.
	// [precautorio]
	VariableRampaDiariaInicial = "HEXCELL_RAMPA_DIARIA_INICIAL"
	// VariableRampaIncrementoSemanal fija el incremento semanal al cupo diario de envíos.
	// [precautorio]
	VariableRampaIncrementoSemanal = "HEXCELL_RAMPA_INCREMENTO_SEMANAL"
	// VariableRampaSemanas fija la cantidad de semanas durante las cuales la rampa incrementa el cupo diario.
	// [precautorio]
	VariableRampaSemanas = "HEXCELL_RAMPA_SEMANAS"
	// VariableCortacircuitosUmbralRepeticion fija el número de repeticiones consecutivas que disparan el cortacircuitos.
	// [causa documentada]
	VariableCortacircuitosUmbralRepeticion = "HEXCELL_CORTACIRCUITOS_UMBRAL_REPETICION"
	// VariableCortacircuitosPalabrasFrustracion fija la lista de palabras clave (separadas por coma) que disparan el cortacircuitos.
	// [causa documentada]
	VariableCortacircuitosPalabrasFrustracion = "HEXCELL_CORTACIRCUITOS_PALABRAS_FRUSTRACION"
	// VariableCortacircuitosTextoTraspaso fija el texto del único mensaje emitido al dispararse el cortacircuitos.
	// [causa documentada]
	VariableCortacircuitosTextoTraspaso = "HEXCELL_CORTACIRCUITOS_TEXTO_TRASPASO"
	// VariableTextoIdentificacion fija el texto de identificación como bot y oferta de traspaso en el primer turno.
	// [causa documentada]
	VariableTextoIdentificacion = "HEXCELL_TEXTO_IDENTIFICACION"
	// VariablePlantillasPresentacion fija la lista de plantillas de saludo/presentación separadas por punto y coma.
	// [causa documentada]
	VariablePlantillasPresentacion = "HEXCELL_PLANTILLAS_PRESENTACION"
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
	// PalabrasDeBajaPorOmision es la lista separada por comas de palabras clave de baja por defecto.
	PalabrasDeBajaPorOmision = "baja,stop"
	// TextoConfirmacionDeBajaPorOmision es el texto por omisión de confirmación de baja.
	TextoConfirmacionDeBajaPorOmision = "Baja confirmada. No volverás a recibir mensajes de este número."

	// LatenciaMinimaMsPorOmision es el suelo de latencia mínima de respuesta (3s).
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	LatenciaMinimaMsPorOmision int64 = 3000
	// LatenciaMinimaMsMaximo es el techo de la latencia mínima permitida (5 min).
	LatenciaMinimaMsMaximo int64 = 300000

	// IntervaloDrenajeMsPorOmision es la cadencia por omisión del bucle de drenaje (2s). No es un
	// parámetro de calibración de negocio ni una técnica anti-baneo, es solo el paso del bucle de fondo.
	// PENDIENTE DE CALIBRACIÓN.
	IntervaloDrenajeMsPorOmision int64 = 2000
	// IntervaloDrenajeMsMaximo es el techo del intervalo de drenaje (1 min).
	IntervaloDrenajeMsMaximo int64 = 60000

	// VentanaAperturaPorOmision es la hora de apertura por omisión (09:00).
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	VentanaAperturaPorOmision = "09:00"
	// VentanaCierrePorOmision es la hora de cierre por omisión (19:00).
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	VentanaCierrePorOmision = "19:00"
	// VentanaDiasPorOmision son los días hábiles ISO (lunes a viernes).
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	VentanaDiasPorOmision = "1,2,3,4,5"
	// VentanaZonaPorOmision es la zona horaria por omisión.
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	VentanaZonaPorOmision = "America/Argentina/Buenos_Aires"

	// RampaDiariaInicialPorOmision es el cupo de envíos diarios inicial (20 msgs/día).
	// [precautorio]
	// PENDIENTE DE CALIBRACIÓN.
	RampaDiariaInicialPorOmision int64 = 20
	// RampaDiariaInicialMaximo es el techo del cupo diario inicial (10000 msgs/día).
	RampaDiariaInicialMaximo int64 = 10000

	// RampaIncrementoSemanalPorOmision es el incremento semanal del cupo (20 msgs/día por semana).
	// [precautorio]
	// PENDIENTE DE CALIBRACIÓN.
	RampaIncrementoSemanalPorOmision int64 = 20
	// RampaIncrementoSemanalMaximo es el techo del incremento semanal (10000 msgs/día).
	RampaIncrementoSemanalMaximo int64 = 10000

	// RampaSemanasPorOmision es la duración de la rampa en semanas (4 semanas).
	// [precautorio]
	// PENDIENTE DE CALIBRACIÓN.
	RampaSemanasPorOmision int64 = 4
	// RampaSemanasMaximo es el techo de semanas de rampa (52 semanas).
	RampaSemanasMaximo int64 = 52

	// CortacircuitosUmbralRepeticionPorOmision es el umbral por omisión de repeticiones (3).
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	CortacircuitosUmbralRepeticionPorOmision int64 = 3
	// CortacircuitosUmbralRepeticionMaximo es el techo del umbral de repeticiones (100).
	CortacircuitosUmbralRepeticionMaximo int64 = 100

	// CortacircuitosPalabrasFrustracionPorOmision son las palabras de frustración o solicitud humana por omisión.
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	CortacircuitosPalabrasFrustracionPorOmision = "humano,persona,agente,operador"

	// CortacircuitosTextoTraspasoPorOmision es el texto por omisión del mensaje de traspaso a humano.
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	CortacircuitosTextoTraspasoPorOmision = "Te paso con una persona del equipo. En cuanto esté disponible te responde por acá."

	// TextoIdentificacionPorOmision es el texto por omisión de identificación y oferta de traspaso en el primer turno.
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	TextoIdentificacionPorOmision = "Te atiende un asistente automático. Si preferís hablar con una persona, escribí «humano»."

	// PlantillasPresentacionPorOmision son las variantes neutrales de presentación por omisión separadas por punto y coma.
	// [causa documentada]
	// PENDIENTE DE CALIBRACIÓN.
	PlantillasPresentacionPorOmision = "¡Hola! Gracias por escribir.;Hola, ¿en qué te puedo ayudar?;Buenas, gracias por tu mensaje."
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

// ErrParametroDeBajaInvalido se devuelve cuando un parámetro de baja está definido pero vacío.
var ErrParametroDeBajaInvalido = errors.New("configuracion: parámetro de baja inválido")

// ErrParametroDeDisciplinaInvalido se devuelve cuando un parámetro de disciplina es inválido o viola los límites acotados.
var ErrParametroDeDisciplinaInvalido = errors.New("configuracion: parámetro de disciplina inválido")

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

// VentanaDeAtencion define el horario comercial y los días en que el sidecar tiene permitido transmitir.
// [causa documentada]
type VentanaDeAtencion struct {
	HoraApertura   int
	MinutoApertura int
	HoraCierre     int
	MinutoCierre   int
	Dias           []int
	Zona           *time.Location
}

// RampaDeVolumen define el escalonamiento de envíos diarios para células nuevas.
// [precautorio]
type RampaDeVolumen struct {
	DiariaInicial     int64
	IncrementoSemanal int64
	Semanas           int64
}

// Disciplina agrupa los parámetros de disciplina de salida del sidecar.
// No contiene ningún campo booleano: la disciplina no es desactivable por configuración.
type Disciplina struct {
	LatenciaMinimaMs   int64
	IntervaloDrenajeMs int64
	Ventana            VentanaDeAtencion
	Rampa              RampaDeVolumen
}

// Cortacircuitos agrupa los parámetros del cortacircuitos conversacional.
// [causa documentada]
// No contiene ningún campo booleano: el cortacircuitos no es desactivable por configuración.
type Cortacircuitos struct {
	UmbralRepeticion    int64
	PalabrasFrustracion []string
	TextoTraspaso       string
}

// Presentacion agrupa los parámetros de presentación e identificación de primer turno.
// [causa documentada]
// No contiene ningún campo booleano: la identificación y variación de plantillas no son desactivables por configuración.
type Presentacion struct {
	TextoIdentificacion string
	Variantes           []string
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
	// PalabrasDeBaja es la lista de palabras clave configuradas para solicitar la baja.
	PalabrasDeBaja []string
	// TextoConfirmacionDeBaja es el texto que se enviará como confirmación de la baja.
	TextoConfirmacionDeBaja string
	// Disciplina agrupa los parámetros de disciplina de salida.
	Disciplina Disciplina
	// Cortacircuitos agrupa los parámetros del cortacircuitos conversacional.
	Cortacircuitos Cortacircuitos
	// Presentacion agrupa los parámetros de presentación e identificación de primer turno.
	Presentacion Presentacion
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

	palabrasBaja, err := cargarPalabrasDeBaja(consultar)
	if err != nil {
		return Configuracion{}, err
	}

	textoConfirmacion, err := cargarTextoConfirmacion(consultar)
	if err != nil {
		return Configuracion{}, err
	}

	disciplina, err := cargarDisciplina(consultar)
	if err != nil {
		return Configuracion{}, err
	}

	cortacircuitos, err := cargarCortacircuitos(consultar)
	if err != nil {
		return Configuracion{}, err
	}

	presentacion, err := cargarPresentacion(consultar)
	if err != nil {
		return Configuracion{}, err
	}

	return Configuracion{
		RutaSocket:              rutaSocket,
		NivelDeRegistro:         nivel,
		IdCelula:                idCelula,
		RutaSqlstore:            rutaSqlstore,
		RutaIdentidad:           rutaIdentidad,
		TelefonoCelula:          telefonoCelula,
		Retroceso:               retroceso,
		TtlSalidaMs:             ttlSalida,
		IntentosMaximosSalida:   intentosSalida,
		PalabrasDeBaja:          palabrasBaja,
		TextoConfirmacionDeBaja: textoConfirmacion,
		Disciplina:              disciplina,
		Cortacircuitos:          cortacircuitos,
		Presentacion:            presentacion,
	}, nil
}

func cargarPalabrasDeBaja(consultar func(string) (string, bool)) ([]string, error) {
	valor, presente := consultar(VariablePalabrasDeBaja)
	if !presente {
		valor = PalabrasDeBajaPorOmision
	} else if valor == "" {
		return nil, ErrParametroDeBajaInvalido
	}

	partes := strings.Split(valor, ",")
	var palabras []string
	for _, p := range partes {
		recortada := strings.TrimSpace(p)
		if recortada != "" {
			palabras = append(palabras, recortada)
		}
	}
	if len(palabras) == 0 {
		return nil, ErrParametroDeBajaInvalido
	}
	return palabras, nil
}

func cargarTextoConfirmacion(consultar func(string) (string, bool)) (string, error) {
	valor, presente := consultar(VariableTextoConfirmacionDeBaja)
	if !presente {
		return TextoConfirmacionDeBajaPorOmision, nil
	}
	if valor == "" || strings.TrimSpace(valor) == "" {
		return "", ErrParametroDeBajaInvalido
	}
	return valor, nil
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

// enteroAcotadoDelEntorno valida, sobre enteroDelEntorno, que el valor caiga en [1, maximo]: el
// patrón que repiten los cinco parámetros acotados de disciplina.
func enteroAcotadoDelEntorno(consultar func(string) (string, bool), variable string, porOmision, maximo int64) (int64, error) {
	valor, err := enteroDelEntorno(consultar, variable, porOmision)
	if err != nil {
		return 0, fmt.Errorf("%w: %v", ErrParametroDeDisciplinaInvalido, err)
	}
	if valor <= 0 || valor > maximo {
		return 0, fmt.Errorf("%w: %s debe estar entre 1 y %d, recibido %d", ErrParametroDeDisciplinaInvalido, variable, maximo, valor)
	}
	return valor, nil
}

// cadenaDelEntorno lee una variable opcional que, presente, no puede estar vacía: el patrón que
// repiten apertura, cierre, días y zona de la ventana de atención.
func cadenaDelEntorno(consultar func(string) (string, bool), variable, porOmision string) (string, error) {
	if v, ok := consultar(variable); ok {
		if v == "" {
			return "", fmt.Errorf("%w: %s no puede estar vacía", ErrParametroDeDisciplinaInvalido, variable)
		}
		return v, nil
	}
	return porOmision, nil
}

func cargarDisciplina(consultar func(string) (string, bool)) (Disciplina, error) {
	latencia, err := enteroAcotadoDelEntorno(consultar, VariableLatenciaMinimaMs, LatenciaMinimaMsPorOmision, LatenciaMinimaMsMaximo)
	if err != nil {
		return Disciplina{}, err
	}

	intervaloDrenaje, err := enteroAcotadoDelEntorno(consultar, VariableIntervaloDrenajeMs, IntervaloDrenajeMsPorOmision, IntervaloDrenajeMsMaximo)
	if err != nil {
		return Disciplina{}, err
	}

	ventana, err := cargarVentanaDeAtencion(consultar)
	if err != nil {
		return Disciplina{}, err
	}

	rampa, err := cargarRampaDeVolumen(consultar)
	if err != nil {
		return Disciplina{}, err
	}

	return Disciplina{
		LatenciaMinimaMs:   latencia,
		IntervaloDrenajeMs: intervaloDrenaje,
		Ventana:            ventana,
		Rampa:              rampa,
	}, nil
}

func parsearHoraMinuto(s string) (int, int, error) {
	partes := strings.Split(s, ":")
	if len(partes) != 2 {
		return 0, 0, fmt.Errorf("formato debe ser HH:MM, recibido %q", s)
	}
	h, err := strconv.Atoi(partes[0])
	if err != nil || h < 0 || h > 23 {
		return 0, 0, fmt.Errorf("hora inválida: %q", partes[0])
	}
	m, err := strconv.Atoi(partes[1])
	if err != nil || m < 0 || m > 59 {
		return 0, 0, fmt.Errorf("minuto inválido: %q", partes[1])
	}
	return h, m, nil
}

func cargarVentanaDeAtencion(consultar func(string) (string, bool)) (VentanaDeAtencion, error) {
	aperturaStr, err := cadenaDelEntorno(consultar, VariableVentanaApertura, VentanaAperturaPorOmision)
	if err != nil {
		return VentanaDeAtencion{}, err
	}
	hAp, mAp, err := parsearHoraMinuto(aperturaStr)
	if err != nil {
		return VentanaDeAtencion{}, fmt.Errorf("%w: %s: %v", ErrParametroDeDisciplinaInvalido, VariableVentanaApertura, err)
	}

	cierreStr, err := cadenaDelEntorno(consultar, VariableVentanaCierre, VentanaCierrePorOmision)
	if err != nil {
		return VentanaDeAtencion{}, err
	}
	hCi, mCi, err := parsearHoraMinuto(cierreStr)
	if err != nil {
		return VentanaDeAtencion{}, fmt.Errorf("%w: %s: %v", ErrParametroDeDisciplinaInvalido, VariableVentanaCierre, err)
	}

	minutosApertura := hAp*60 + mAp
	minutosCierre := hCi*60 + mCi
	duracion := minutosCierre - minutosApertura

	if duracion <= 0 {
		return VentanaDeAtencion{}, fmt.Errorf("%w: la hora de cierre (%s) debe ser posterior a la de apertura (%s)", ErrParametroDeDisciplinaInvalido, cierreStr, aperturaStr)
	}
	if duracion > 16*60 {
		return VentanaDeAtencion{}, fmt.Errorf("%w: la ventana de atención no puede exceder 16 horas (anti-24/7): duración actual %d minutos", ErrParametroDeDisciplinaInvalido, duracion)
	}

	diasStr, err := cadenaDelEntorno(consultar, VariableVentanaDias, VentanaDiasPorOmision)
	if err != nil {
		return VentanaDeAtencion{}, err
	}
	partesDias := strings.Split(diasStr, ",")
	var dias []int
	for _, p := range partesDias {
		p = strings.TrimSpace(p)
		if p == "" {
			continue
		}
		d, err := strconv.Atoi(p)
		if err != nil || d < 1 || d > 7 {
			return VentanaDeAtencion{}, fmt.Errorf("%w: día de atención inválido %q (debe ser 1..7)", ErrParametroDeDisciplinaInvalido, p)
		}
		dias = append(dias, d)
	}
	if len(dias) == 0 {
		return VentanaDeAtencion{}, fmt.Errorf("%w: %s debe especificar al menos un día válido", ErrParametroDeDisciplinaInvalido, VariableVentanaDias)
	}

	zonaStr, err := cadenaDelEntorno(consultar, VariableVentanaZona, VentanaZonaPorOmision)
	if err != nil {
		return VentanaDeAtencion{}, err
	}
	loc, err := time.LoadLocation(zonaStr)
	if err != nil {
		return VentanaDeAtencion{}, fmt.Errorf("%w: zona horaria inválida %q: %v", ErrParametroDeDisciplinaInvalido, zonaStr, err)
	}

	return VentanaDeAtencion{
		HoraApertura:   hAp,
		MinutoApertura: mAp,
		HoraCierre:     hCi,
		MinutoCierre:   mCi,
		Dias:           dias,
		Zona:           loc,
	}, nil
}

func cargarRampaDeVolumen(consultar func(string) (string, bool)) (RampaDeVolumen, error) {
	inicial, err := enteroAcotadoDelEntorno(consultar, VariableRampaDiariaInicial, RampaDiariaInicialPorOmision, RampaDiariaInicialMaximo)
	if err != nil {
		return RampaDeVolumen{}, err
	}

	incremento, err := enteroAcotadoDelEntorno(consultar, VariableRampaIncrementoSemanal, RampaIncrementoSemanalPorOmision, RampaIncrementoSemanalMaximo)
	if err != nil {
		return RampaDeVolumen{}, err
	}

	semanas, err := enteroAcotadoDelEntorno(consultar, VariableRampaSemanas, RampaSemanasPorOmision, RampaSemanasMaximo)
	if err != nil {
		return RampaDeVolumen{}, err
	}

	return RampaDeVolumen{
		DiariaInicial:     inicial,
		IncrementoSemanal: incremento,
		Semanas:           semanas,
	}, nil
}

func listaDelEntorno(consultar func(string) (string, bool), variable, porOmision string) ([]string, error) {
	valor, presente := consultar(variable)
	if !presente {
		valor = porOmision
	} else if strings.TrimSpace(valor) == "" {
		return nil, fmt.Errorf("%w: %s no puede estar vacía", ErrParametroDeDisciplinaInvalido, variable)
	}

	partes := strings.Split(valor, ",")
	var elementos []string
	for _, p := range partes {
		recortada := strings.TrimSpace(p)
		if recortada != "" {
			elementos = append(elementos, recortada)
		}
	}
	if len(elementos) == 0 {
		return nil, fmt.Errorf("%w: %s no contiene elementos válidos", ErrParametroDeDisciplinaInvalido, variable)
	}
	return elementos, nil
}

func cargarCortacircuitos(consultar func(string) (string, bool)) (Cortacircuitos, error) {
	umbral, err := enteroAcotadoDelEntorno(consultar, VariableCortacircuitosUmbralRepeticion, CortacircuitosUmbralRepeticionPorOmision, CortacircuitosUmbralRepeticionMaximo)
	if err != nil {
		return Cortacircuitos{}, err
	}

	palabras, err := listaDelEntorno(consultar, VariableCortacircuitosPalabrasFrustracion, CortacircuitosPalabrasFrustracionPorOmision)
	if err != nil {
		return Cortacircuitos{}, err
	}

	texto, err := cadenaDelEntorno(consultar, VariableCortacircuitosTextoTraspaso, CortacircuitosTextoTraspasoPorOmision)
	if err != nil {
		return Cortacircuitos{}, err
	}

	return Cortacircuitos{
		UmbralRepeticion:    umbral,
		PalabrasFrustracion: palabras,
		TextoTraspaso:       texto,
	}, nil
}

func cargarTextoIdentificacion(consultar func(string) (string, bool)) (string, error) {
	valor, presente := consultar(VariableTextoIdentificacion)
	if !presente {
		return TextoIdentificacionPorOmision, nil
	}
	if valor == "" || strings.TrimSpace(valor) == "" {
		return "", fmt.Errorf("%w: %s no puede estar vacía", ErrParametroDeDisciplinaInvalido, VariableTextoIdentificacion)
	}
	return valor, nil
}

func cargarPlantillasPresentacion(consultar func(string) (string, bool)) ([]string, error) {
	valor, presente := consultar(VariablePlantillasPresentacion)
	if !presente {
		valor = PlantillasPresentacionPorOmision
	} else if strings.TrimSpace(valor) == "" {
		return nil, fmt.Errorf("%w: %s no puede estar vacía", ErrParametroDeDisciplinaInvalido, VariablePlantillasPresentacion)
	}

	partes := strings.Split(valor, ";")
	var variantes []string
	for _, p := range partes {
		recortada := strings.TrimSpace(p)
		if recortada != "" {
			variantes = append(variantes, recortada)
		}
	}
	if len(variantes) < 2 {
		return nil, fmt.Errorf("%w: %s debe contener al menos 2 variantes no vacías", ErrParametroDeDisciplinaInvalido, VariablePlantillasPresentacion)
	}
	return variantes, nil
}

func cargarPresentacion(consultar func(string) (string, bool)) (Presentacion, error) {
	texto, err := cargarTextoIdentificacion(consultar)
	if err != nil {
		return Presentacion{}, err
	}

	variantes, err := cargarPlantillasPresentacion(consultar)
	if err != nil {
		return Presentacion{}, err
	}

	return Presentacion{
		TextoIdentificacion: texto,
		Variantes:           variantes,
	}, nil
}
