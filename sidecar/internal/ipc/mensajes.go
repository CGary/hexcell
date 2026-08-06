// Package ipc es la representación tipada del protocolo que fija
// `docs/protocolo-ipc-nucleo-sidecar.md`, versión 1.2 (versión de cable 3).
//
// Aquí no hay socket, ni escucha, ni outbox: solo los objetos de valor de los nueve tipos de
// mensaje y dos funciones puras, [Codificar] y [Decodificar]. El transporte llega con la tarea 3
// del plan de la etapa A-3; separarlo permite comprobar el formato con tests normales, sin abrir
// ningún descriptor, por el mismo motivo por el que `registro::formatear` está separado de
// `registro::emitir` del lado Rust (adr-0019).
//
// # Las restricciones del formato son deliberadas
//
// Un objeto JSON plano por línea, profundidad 1, valores solo cadena o entero, conjunto de campos
// cerrado por tipo, todos los campos siempre presentes y en orden fijo. El workspace Rust no
// declara `serde` en ningún crate, y el otro extremo de este protocolo tendrá que **analizar**
// estas líneas, que es más caro que emitirlas: cualquier anidamiento admitido aquí crearía esa
// dependencia en silencio.
//
// La codificación se escribe a mano en lugar de delegarse en `encoding/json` con etiquetas de
// estructura: así el orden de los campos y la profundidad 1 son propiedades del código, y no
// consecuencias de cómo una biblioteca decida serializar una estructura que alguien anide mañana.
package ipc

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

// VersionProtocolo es la versión que este binario habla. Un desajuste cierra la conexión.
const VersionProtocolo int64 = 3

// LongitudMaximaDeLinea es el techo de una línea del protocolo, en bytes, salto de línea
// incluido. Existe para que el lector del otro extremo dimensione un búfer acotado.
const LongitudMaximaDeLinea = 131072

// Nombres de los dos campos que encabezan toda línea, en este orden.
const (
	CampoVersion = "version"
	CampoTipo    = "tipo"
)

// TipoMensaje es el conjunto cerrado de tipos del documento.
type TipoMensaje string

// Los nueve tipos del protocolo, ni uno más.
const (
	TipoSaludo                TipoMensaje = "saludo"
	TipoEventoEntrante        TipoMensaje = "evento_entrante"
	TipoConfirmacion          TipoMensaje = "confirmacion"
	TipoEstadoSesion          TipoMensaje = "estado_sesion"
	TipoOrdenRespaldoSqlstore TipoMensaje = "orden_respaldo_sqlstore"
	TipoAcuseRespaldoSqlstore TipoMensaje = "acuse_respaldo_sqlstore"
	TipoOrdenEmparejar        TipoMensaje = "orden_emparejar"
	TipoCodigoEmparejamiento  TipoMensaje = "codigo_emparejamiento"
	TipoAcuseEmparejamiento   TipoMensaje = "acuse_emparejamiento"
)

// Valores cerrados del campo `emisor` de un saludo.
const (
	EmisorNucleo  = "nucleo"
	EmisorSidecar = "sidecar"
)

// Valores cerrados del campo `estado` de un estado de sesión. El vocabulario del campo `causa`
// lo cierra la tarea 7 del plan, la que produce la taxonomía de desconexión; aquí no se inventa.
const (
	EstadoActiva       = "activa"
	EstadoReconectando = "reconectando"
	EstadoDesvinculada = "desvinculada"
	EstadoPausada      = "pausada"
)

// Valores cerrados del campo `causa` de un estado de sesión. El vocabulario
// lo cierra la versión 1.2 del documento, con una variante por cada rama de la
// taxonomía de desconexión de whatsmeow.
const (
	CausaDispositivoRemovido     = "desvinculada_dispositivo_removido"
	CausaSesionCerrada           = "desvinculada_sesion_cerrada"
	CausaBaneoTemporal           = "baneo_temporal"
	CausaSesionReemplazada       = "sesion_reemplazada"
	CausaFalloDeConexion         = "fallo_de_conexion"
	CausaErrorDeFlujo            = "error_de_flujo"
	CausaDesconexionDeTransporte = "desconexion_de_transporte"
	CausaClienteObsoleto         = "cliente_obsoleto"
)

// OrdenRespaldarSqlstore es la cadena fija que el contrato de la etapa A-2 exige en el campo
// `orden` (`docs/contrato-ipc-respaldo-del-sqlstore.md`, sección 1).
const OrdenRespaldarSqlstore = "respaldar_sqlstore"

// Valores cerrados del campo `resultado` del acuse de respaldo, según ese mismo contrato.
const (
	ResultadoCompletado = "completado"
	ResultadoFallido    = "fallido"
)

// Valores cerrados del campo `metodo` de una orden de emparejamiento y su payload.
const (
	MetodoQr                  = "qr"
	MetodoCodigoDeVinculacion = "codigo_de_vinculacion"
)

// Valores cerrados del campo `resultado` de un acuse de emparejamiento.
const (
	ResultadoEmparejamientoCompletado = "completado"
	ResultadoEmparejamientoExpirado   = "expirado"
	ResultadoEmparejamientoFallido    = "fallido"
)

// Errores de protocolo. Cualquiera de ellos cierra la conexión: una vez que el delimitado por
// líneas es dudoso, seguir leyendo es adivinar.
var (
	ErrLineaVacia           = errors.New("ipc: línea vacía")
	ErrLineaDemasiadoLarga  = errors.New("ipc: línea por encima del límite del protocolo")
	ErrLineaMalformada      = errors.New("ipc: la línea no es un objeto JSON de profundidad 1")
	ErrVersionIncompatible  = errors.New("ipc: versión de protocolo incompatible")
	ErrTipoDesconocido      = errors.New("ipc: tipo de mensaje desconocido")
	ErrCampoAusente         = errors.New("ipc: campo obligatorio ausente")
	ErrCampoDesconocido     = errors.New("ipc: campo desconocido")
	ErrValorNoEscalar       = errors.New("ipc: el valor no es una cadena ni un entero")
	ErrCuerpoIncoherente    = errors.New("ipc: el cuerpo no corresponde al tipo declarado")
	ErrCuerpoNoEspecificado = errors.New("ipc: sobre sin cuerpo")
)

// claseDeCampo distingue los dos únicos tipos de valor que el protocolo admite.
type claseDeCampo int

const (
	claseCadena claseDeCampo = iota
	claseEntero
)

// declaracion es un campo del cuerpo de un mensaje: su nombre y su clase.
type declaracion struct {
	nombre string
	clase  claseDeCampo
}

// Cuerpo es el contenido de un mensaje. La interfaz tiene métodos no exportados a propósito:
// ningún paquete de fuera puede añadir un décimo tipo sin tocar este archivo, de modo que el
// conjunto cerrado del documento lo impone el compilador y no la revisión.
type Cuerpo interface {
	tipo() TipoMensaje
	valores() []any
}

// Saludo es el primer mensaje de toda conexión, en las dos direcciones.
type Saludo struct {
	Emisor   string
	IdCelula string
}

func (Saludo) tipo() TipoMensaje { return TipoSaludo }
func (s Saludo) valores() []any  { return []any{s.Emisor, s.IdCelula} }

// EventoEntrante es un mensaje recibido del canal, ya normalizado. No hay campo para el JID, ni
// para el identificador de dispositivo, ni para el número de teléfono, y no lo habrá: el mapeo del
// JID vive dentro del adaptador (tarea 9, adr-0010) y el núcleo trata lo interno como opaco.
type EventoEntrante struct {
	IdDeduplicacion string
	IdConversacion  string
	IdRemitente     string
	Contenido       string
	MarcaTemporalMs int64
}

func (EventoEntrante) tipo() TipoMensaje { return TipoEventoEntrante }
func (e EventoEntrante) valores() []any {
	return []any{e.IdDeduplicacion, e.IdConversacion, e.IdRemitente, e.Contenido, e.MarcaTemporalMs}
}

// Confirmacion es el acuse durable de un evento entrante. Referencia el identificador de
// deduplicación y **nunca** un número de secuencia por conexión: un contador por conexión se
// reinicia con ella, de modo que tras una reconexión el acuse N y el evento N pueden diferir.
type Confirmacion struct {
	IdDeduplicacion string
}

func (Confirmacion) tipo() TipoMensaje { return TipoConfirmacion }
func (c Confirmacion) valores() []any  { return []any{c.IdDeduplicacion} }

// EstadoSesion transporta la proyección del estado de la sesión de WhatsApp junto a la señal
// cruda de la taxonomía de desconexión, nunca en su lugar.
type EstadoSesion struct {
	Estado     string
	Causa      string
	Codigo     int64
	ExpiraEnMs int64
}

func (EstadoSesion) tipo() TipoMensaje { return TipoEstadoSesion }
func (e EstadoSesion) valores() []any {
	return []any{e.Estado, e.Causa, e.Codigo, e.ExpiraEnMs}
}

// OrdenRespaldoSqlstore es la orden de copia del `sqlstore`, con los tres campos exactos que fija
// la sección 1 del contrato de la etapa A-2.
type OrdenRespaldoSqlstore struct {
	Orden                string
	Destino              string
	IdentificadorDeRonda string
}

func (OrdenRespaldoSqlstore) tipo() TipoMensaje { return TipoOrdenRespaldoSqlstore }
func (o OrdenRespaldoSqlstore) valores() []any {
	return []any{o.Orden, o.Destino, o.IdentificadorDeRonda}
}

// AcuseRespaldoSqlstore es el desenlace de esa copia, con los cinco campos exactos que fija la
// sección 3 de ese mismo contrato.
//
// El contrato describe `ruta_de_la_copia`, `bytes` y `motivo` como presentes solo según el
// resultado; este protocolo expresa esa condicionalidad con el valor vacío en lugar de con la
// omisión del campo, para que el otro extremo no tenga que tratar campos opcionales.
type AcuseRespaldoSqlstore struct {
	IdentificadorDeRonda string
	Resultado            string
	RutaDeLaCopia        string
	Bytes                int64
	Motivo               string
}

func (AcuseRespaldoSqlstore) tipo() TipoMensaje { return TipoAcuseRespaldoSqlstore }
func (a AcuseRespaldoSqlstore) valores() []any {
	return []any{a.IdentificadorDeRonda, a.Resultado, a.RutaDeLaCopia, a.Bytes, a.Motivo}
}

// OrdenEmparejar es la orden del núcleo al sidecar para iniciar un emparejamiento.
// El número de teléfono de la célula NO viaja en el mensaje: se lee de la configuración
// (adr-0010, invariante de que el JID nunca cruza la frontera del puerto).
type OrdenEmparejar struct {
	Metodo string
}

func (OrdenEmparejar) tipo() TipoMensaje { return TipoOrdenEmparejar }
func (o OrdenEmparejar) valores() []any  { return []any{o.Metodo} }

// CodigoEmparejamiento es el payload del emparejamiento: un código QR o un código de vinculación
// de ocho caracteres, según el método solicitado. La cadena viaja como dato opaco: quien la
// consume decide si la codifica como imagen QR o la muestra como texto.
type CodigoEmparejamiento struct {
	Metodo     string
	Valor      string
	ExpiraEnMs int64
}

func (CodigoEmparejamiento) tipo() TipoMensaje { return TipoCodigoEmparejamiento }
func (c CodigoEmparejamiento) valores() []any {
	return []any{c.Metodo, c.Valor, c.ExpiraEnMs}
}

// AcuseEmparejamiento es el resultado terminal del proceso de emparejamiento.
type AcuseEmparejamiento struct {
	Resultado string
	Motivo    string
}

func (AcuseEmparejamiento) tipo() TipoMensaje { return TipoAcuseEmparejamiento }
func (a AcuseEmparejamiento) valores() []any  { return []any{a.Resultado, a.Motivo} }

// descriptor declara los campos del cuerpo de un tipo y cómo reconstruirlo al decodificar.
type descriptor struct {
	campos    []declaracion
	construir func(cadenas []string, enteros []int64) Cuerpo
}

// descriptores es la tabla del conjunto cerrado. El orden de `campos` es el orden de emisión.
var descriptores = map[TipoMensaje]descriptor{
	TipoSaludo: {
		campos: []declaracion{{"emisor", claseCadena}, {"id_celula", claseCadena}},
		construir: func(cadenas []string, _ []int64) Cuerpo {
			return Saludo{Emisor: cadenas[0], IdCelula: cadenas[1]}
		},
	},
	TipoEventoEntrante: {
		campos: []declaracion{
			{"id_deduplicacion", claseCadena},
			{"id_conversacion", claseCadena},
			{"id_remitente", claseCadena},
			{"contenido", claseCadena},
			{"marca_temporal_ms", claseEntero},
		},
		construir: func(cadenas []string, enteros []int64) Cuerpo {
			return EventoEntrante{
				IdDeduplicacion: cadenas[0],
				IdConversacion:  cadenas[1],
				IdRemitente:     cadenas[2],
				Contenido:       cadenas[3],
				MarcaTemporalMs: enteros[0],
			}
		},
	},
	TipoConfirmacion: {
		campos: []declaracion{{"id_deduplicacion", claseCadena}},
		construir: func(cadenas []string, _ []int64) Cuerpo {
			return Confirmacion{IdDeduplicacion: cadenas[0]}
		},
	},
	TipoEstadoSesion: {
		campos: []declaracion{
			{"estado", claseCadena},
			{"causa", claseCadena},
			{"codigo", claseEntero},
			{"expira_en_ms", claseEntero},
		},
		construir: func(cadenas []string, enteros []int64) Cuerpo {
			return EstadoSesion{
				Estado:     cadenas[0],
				Causa:      cadenas[1],
				Codigo:     enteros[0],
				ExpiraEnMs: enteros[1],
			}
		},
	},
	TipoOrdenRespaldoSqlstore: {
		campos: []declaracion{
			{"orden", claseCadena},
			{"destino", claseCadena},
			{"identificador_de_ronda", claseCadena},
		},
		construir: func(cadenas []string, _ []int64) Cuerpo {
			return OrdenRespaldoSqlstore{
				Orden:                cadenas[0],
				Destino:              cadenas[1],
				IdentificadorDeRonda: cadenas[2],
			}
		},
	},
	TipoAcuseRespaldoSqlstore: {
		campos: []declaracion{
			{"identificador_de_ronda", claseCadena},
			{"resultado", claseCadena},
			{"ruta_de_la_copia", claseCadena},
			{"bytes", claseEntero},
			{"motivo", claseCadena},
		},
		construir: func(cadenas []string, enteros []int64) Cuerpo {
			return AcuseRespaldoSqlstore{
				IdentificadorDeRonda: cadenas[0],
				Resultado:            cadenas[1],
				RutaDeLaCopia:        cadenas[2],
				Bytes:                enteros[0],
				Motivo:               cadenas[3],
			}
		},
	},
	TipoOrdenEmparejar: {
		campos: []declaracion{{"metodo", claseCadena}},
		construir: func(cadenas []string, _ []int64) Cuerpo {
			return OrdenEmparejar{Metodo: cadenas[0]}
		},
	},
	TipoCodigoEmparejamiento: {
		campos: []declaracion{
			{"metodo", claseCadena},
			{"valor", claseCadena},
			{"expira_en_ms", claseEntero},
		},
		construir: func(cadenas []string, enteros []int64) Cuerpo {
			return CodigoEmparejamiento{
				Metodo:     cadenas[0],
				Valor:      cadenas[1],
				ExpiraEnMs: enteros[0],
			}
		},
	},
	TipoAcuseEmparejamiento: {
		campos: []declaracion{
			{"resultado", claseCadena},
			{"motivo", claseCadena},
		},
		construir: func(cadenas []string, _ []int64) Cuerpo {
			return AcuseEmparejamiento{Resultado: cadenas[0], Motivo: cadenas[1]}
		},
	},
}

// CausasDeclaradas devuelve el vocabulario cerrado de causas en orden estable,
// para los mismos usos que TiposDeclarados(): tests de documento, tests de
// coherencia y generación de tablas.
func CausasDeclaradas() []string {
	return []string{
		CausaBaneoTemporal,
		CausaClienteObsoleto,
		CausaDesconexionDeTransporte,
		CausaDispositivoRemovido,
		CausaSesionCerrada,
		CausaErrorDeFlujo,
		CausaFalloDeConexion,
		CausaSesionReemplazada,
	}
}

// EstadosDeclarados devuelve los cuatro valores del campo `estado` en orden
// estable.
func EstadosDeclarados() []string {
	return []string{
		EstadoActiva,
		EstadoDesvinculada,
		EstadoPausada,
		EstadoReconectando,
	}
}

// TiposDeclarados devuelve el conjunto cerrado de tipos del protocolo, en orden estable.
func TiposDeclarados() []TipoMensaje {
	return []TipoMensaje{
		TipoSaludo, TipoEventoEntrante, TipoConfirmacion,
		TipoEstadoSesion, TipoOrdenRespaldoSqlstore, TipoAcuseRespaldoSqlstore,
		TipoOrdenEmparejar, TipoCodigoEmparejamiento, TipoAcuseEmparejamiento,
	}
}

// CamposDe devuelve los nombres de campo de una línea de ese tipo, en el orden de emisión,
// incluidos `version` y `tipo`. Devuelve nil si el tipo no está declarado.
func CamposDe(tipo TipoMensaje) []string {
	descrito, declarado := descriptores[tipo]
	if !declarado {
		return nil
	}
	nombres := make([]string, 0, len(descrito.campos)+2)
	nombres = append(nombres, CampoVersion, CampoTipo)
	for _, campo := range descrito.campos {
		nombres = append(nombres, campo.nombre)
	}
	return nombres
}

// Sobre es una línea del protocolo: su versión, su tipo y su cuerpo.
type Sobre struct {
	Version int64
	Tipo    TipoMensaje
	Cuerpo  Cuerpo
}

// NuevoSobre envuelve un cuerpo con la versión y el tipo que le corresponden.
func NuevoSobre(cuerpo Cuerpo) Sobre {
	return Sobre{Version: VersionProtocolo, Tipo: cuerpo.tipo(), Cuerpo: cuerpo}
}

// escaparCadena escapa un valor como texto JSON, con la misma tabla que `escapar_json` del lado
// Rust: comillas, barra invertida, tres controles con secuencia corta y el resto como `\u00XX`.
func escaparCadena(valor string) string {
	var escapado strings.Builder
	escapado.Grow(len(valor))
	for _, caracter := range valor {
		switch caracter {
		case '"':
			escapado.WriteString(`\"`)
		case '\\':
			escapado.WriteString(`\\`)
		case '\n':
			escapado.WriteString(`\n`)
		case '\r':
			escapado.WriteString(`\r`)
		case '\t':
			escapado.WriteString(`\t`)
		default:
			if caracter < 0x20 {
				fmt.Fprintf(&escapado, `\u%04x`, caracter)
				continue
			}
			escapado.WriteRune(caracter)
		}
	}
	return escapado.String()
}

// Codificar serializa un sobre como una única línea terminada en salto de línea. Los campos salen
// en el orden declarado y la profundidad es 1 por construcción: la función escribe pares
// clave-valor escalares y no tiene ninguna rama capaz de anidar un objeto.
func Codificar(sobre Sobre) ([]byte, error) {
	if sobre.Cuerpo == nil {
		return nil, ErrCuerpoNoEspecificado
	}
	if sobre.Tipo != sobre.Cuerpo.tipo() {
		return nil, fmt.Errorf("%w: sobre %q, cuerpo %q", ErrCuerpoIncoherente, sobre.Tipo, sobre.Cuerpo.tipo())
	}
	descrito, declarado := descriptores[sobre.Tipo]
	if !declarado {
		return nil, fmt.Errorf("%w: %q", ErrTipoDesconocido, sobre.Tipo)
	}

	var linea bytes.Buffer
	linea.WriteByte('{')
	fmt.Fprintf(&linea, `"%s":%d`, CampoVersion, sobre.Version)
	fmt.Fprintf(&linea, `,"%s":"%s"`, CampoTipo, escaparCadena(string(sobre.Tipo)))

	valores := sobre.Cuerpo.valores()
	for indice, campo := range descrito.campos {
		switch campo.clase {
		case claseCadena:
			texto, esTexto := valores[indice].(string)
			if !esTexto {
				return nil, fmt.Errorf("%w: %s", ErrValorNoEscalar, campo.nombre)
			}
			fmt.Fprintf(&linea, `,"%s":"%s"`, campo.nombre, escaparCadena(texto))
		case claseEntero:
			entero, esEntero := valores[indice].(int64)
			if !esEntero {
				return nil, fmt.Errorf("%w: %s", ErrValorNoEscalar, campo.nombre)
			}
			fmt.Fprintf(&linea, `,"%s":%d`, campo.nombre, entero)
		}
	}
	linea.WriteString("}\n")

	if linea.Len() > LongitudMaximaDeLinea {
		return nil, fmt.Errorf("%w: %d bytes", ErrLineaDemasiadoLarga, linea.Len())
	}
	return linea.Bytes(), nil
}

// Decodificar analiza una línea del protocolo y devuelve su sobre.
//
// Rechaza, con error tipado y sin devolver un sobre a medio poblar: la línea vacía, la que supera
// el límite, la que no es un objeto JSON, la que anida un objeto o una lista, la que trae un
// booleano o un nulo, la de versión distinta, la de tipo desconocido, la que omite un campo
// declarado y la que trae uno de más.
func Decodificar(linea []byte) (Sobre, error) {
	recortada := bytes.TrimRight(linea, "\r\n")
	if len(bytes.TrimSpace(recortada)) == 0 {
		return Sobre{}, ErrLineaVacia
	}
	if len(linea) > LongitudMaximaDeLinea {
		return Sobre{}, fmt.Errorf("%w: %d bytes", ErrLineaDemasiadoLarga, len(linea))
	}

	crudos, err := leerObjetoPlano(recortada)
	if err != nil {
		return Sobre{}, err
	}

	version, err := enteroDe(crudos, CampoVersion)
	if err != nil {
		return Sobre{}, err
	}
	if version != VersionProtocolo {
		return Sobre{}, fmt.Errorf("%w: recibida %d, esperada %d", ErrVersionIncompatible, version, VersionProtocolo)
	}

	nombreTipo, err := cadenaDe(crudos, CampoTipo)
	if err != nil {
		return Sobre{}, err
	}
	tipo := TipoMensaje(nombreTipo)
	descrito, declarado := descriptores[tipo]
	if !declarado {
		return Sobre{}, fmt.Errorf("%w: %q", ErrTipoDesconocido, nombreTipo)
	}

	declarados := make(map[string]struct{}, len(descrito.campos)+2)
	for _, nombre := range CamposDe(tipo) {
		declarados[nombre] = struct{}{}
	}
	for nombre := range crudos {
		if _, esperado := declarados[nombre]; !esperado {
			return Sobre{}, fmt.Errorf("%w: %q en %q", ErrCampoDesconocido, nombre, nombreTipo)
		}
	}

	cadenas := make([]string, 0, len(descrito.campos))
	enteros := make([]int64, 0, len(descrito.campos))
	for _, campo := range descrito.campos {
		switch campo.clase {
		case claseCadena:
			texto, err := cadenaDe(crudos, campo.nombre)
			if err != nil {
				return Sobre{}, err
			}
			cadenas = append(cadenas, texto)
		case claseEntero:
			entero, err := enteroDe(crudos, campo.nombre)
			if err != nil {
				return Sobre{}, err
			}
			enteros = append(enteros, entero)
		}
	}

	return Sobre{Version: version, Tipo: tipo, Cuerpo: descrito.construir(cadenas, enteros)}, nil
}

// leerObjetoPlano decodifica la línea como objeto JSON y comprueba, valor a valor, que la
// profundidad es 1 y que cada valor es una cadena o un entero.
func leerObjetoPlano(linea []byte) (map[string]any, error) {
	decodificador := json.NewDecoder(bytes.NewReader(linea))
	decodificador.UseNumber()

	var crudos map[string]any
	if err := decodificador.Decode(&crudos); err != nil {
		return nil, fmt.Errorf("%w: %v", ErrLineaMalformada, err)
	}
	if decodificador.More() {
		return nil, fmt.Errorf("%w: más de un valor JSON en la línea", ErrLineaMalformada)
	}
	if crudos == nil {
		return nil, fmt.Errorf("%w: la línea no es un objeto", ErrLineaMalformada)
	}

	for nombre, valor := range crudos {
		switch tipado := valor.(type) {
		case string:
		case json.Number:
			if _, err := tipado.Int64(); err != nil {
				return nil, fmt.Errorf("%w: %s no es un entero", ErrValorNoEscalar, nombre)
			}
		default:
			return nil, fmt.Errorf("%w: %s", ErrValorNoEscalar, nombre)
		}
	}
	return crudos, nil
}

// cadenaDe extrae un campo declarado como cadena.
func cadenaDe(crudos map[string]any, nombre string) (string, error) {
	valor, presente := crudos[nombre]
	if !presente {
		return "", fmt.Errorf("%w: %s", ErrCampoAusente, nombre)
	}
	texto, esTexto := valor.(string)
	if !esTexto {
		return "", fmt.Errorf("%w: %s no es una cadena", ErrValorNoEscalar, nombre)
	}
	return texto, nil
}

// enteroDe extrae un campo declarado como entero.
func enteroDe(crudos map[string]any, nombre string) (int64, error) {
	valor, presente := crudos[nombre]
	if !presente {
		return 0, fmt.Errorf("%w: %s", ErrCampoAusente, nombre)
	}
	numero, esNumero := valor.(json.Number)
	if !esNumero {
		return 0, fmt.Errorf("%w: %s no es un entero", ErrValorNoEscalar, nombre)
	}
	entero, err := numero.Int64()
	if err != nil {
		return 0, fmt.Errorf("%w: %s no es un entero", ErrValorNoEscalar, nombre)
	}
	return entero, nil
}
