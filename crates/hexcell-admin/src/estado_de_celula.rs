//! Agregado del plano de control: estados de la célula y su tabla de transiciones válidas.
//!
//! Esta tarea es la primera de tres hijas de la tarea 10 de la etapa A-6 (esqueleto de la CLI
//! `hexcell-admin`). Fija solo el vocabulario de estados, la tabla exhaustiva de transiciones
//! legales y el rechazo tipado de todo par ausente de esa tabla. El analizador de argumentos, los
//! seis subcomandos, el modo de simulación, los códigos de salida y los sumideros de salida son
//! trabajo de las tareas hermanas HEX-074-b y HEX-074-c. Esta tarea tampoco persiste nada: no hay
//! base SQLite, archivo de estado, esquema ni migración — el almacén de estado del plano de
//! control es una entrega distinta de la etapa A-6, y este agregado se ejercita solo en memoria.
//!
//! Los cinco estados del plano de control (`Aprovisionada`, `EnEjecucion`, `Suspendida`,
//! `Reemparejando`, `Retirada`) son un vocabulario propio de la célula como unidad desplegable, y
//! deliberadamente NO reutilizan la taxonomía de estado de sesión de
//! `docs/protocolo-ipc-nucleo-sidecar.md` (activa, reconectando, desvinculada, pausada): esa
//! taxonomía describe la sesión de WhatsApp dentro del sidecar, no la célula completa. El estado
//! terminal del plano de control es `Retirada`; el estado de pausa operativa es `Suspendida`.

/// Los cinco estados posibles de una célula en el plano de control.
///
/// Enumerado cerrado a propósito (sin `#[non_exhaustive]`): tanto la tarea hermana HEX-074-b como
/// las tareas 11 a 15 del plan de la etapa A-6 necesitan poder emparejar sobre él desde fuera de
/// este crate sin un brazo por defecto, siguiendo el precedente de `ResultadoEnvio` en
/// `hexcell-core`. No deriva nada de `serde`: `serde` figura entre las dependencias del crate
/// para el análisis del JSON del motor Docker, y derivar su serialización aquí sería el primer
/// paso de la persistencia que esta tarea aplaza explícitamente.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EstadoDeCelula {
    /// La célula existe (contenedores creados) pero todavía no se puso en marcha.
    Aprovisionada,
    /// La célula está en marcha: ambos contenedores corriendo y sirviendo tráfico.
    EnEjecucion,
    /// El operador pausó la célula: ambos contenedores detenidos, nada se sirve.
    Suspendida,
    /// La célula está atravesando un reemparejamiento de sesión (tarea 24 de la etapa A-6).
    Reemparejando,
    /// Estado terminal: la célula fue dada de baja de forma definitiva.
    Retirada,
}

impl EstadoDeCelula {
    /// Los cinco estados declarados, en el mismo orden que las variantes del enumerado.
    ///
    /// Constante de arreglo de longitud fija: cualquier prueba externa que quiera recorrer el
    /// conjunto completo de estados sin depender de una función auxiliar puede hacerlo a partir
    /// de aquí, y su longitud (`5`) es un ancla que una prueba puede comparar contra el número de
    /// variantes declaradas.
    pub const TODOS: [EstadoDeCelula; 5] = [
        EstadoDeCelula::Aprovisionada,
        EstadoDeCelula::EnEjecucion,
        EstadoDeCelula::Suspendida,
        EstadoDeCelula::Reemparejando,
        EstadoDeCelula::Retirada,
    ];

    /// La tabla de transiciones legales: para cada estado de origen, los estados de destino
    /// alcanzables en un solo paso.
    ///
    /// Coincidencia con cinco brazos y ningún brazo por defecto: añadir un sexto estado al
    /// enumerado sin extender esta función deja de compilar, en vez de caer silenciosamente en
    /// una reacción genérica. Once pares ordenados legales de los veinticinco posibles:
    ///
    /// - `Aprovisionada` -> `EnEjecucion` (arranque, tarea 11), `Retirada` (baja antes de arrancar).
    /// - `EnEjecucion` -> `Suspendida` (pausa del operador, tarea 11), `Reemparejando` (rebind de
    ///   una célula en marcha, tarea 13 — un gate de envío saliente interno al rebind no es esta
    ///   pausa de plano de control, así que no exige pasar por `Suspendida` primero), `Retirada`
    ///   (baja en marcha).
    /// - `Suspendida` -> `EnEjecucion` (reanudación), `Reemparejando` (recuperación tras baneo
    ///   permanente, adr-0015, sobre una célula ya pausada), `Retirada` (baja en pausa).
    /// - `Reemparejando` -> `EnEjecucion` (rebind exitoso), `Suspendida` (rebind interrumpido, el
    ///   operador la deja en pausa), `Retirada` (baja durante el rebind).
    /// - `Retirada` -> ninguno: es el único estado terminal.
    ///
    /// Ninguna transición hacia el mismo estado de origen está en la tabla: las cinco parejas de
    /// identidad se rechazan a propósito. La reejecución idempotente de un comando parcialmente
    /// fallido es la tarea 15 del plan de la etapa A-6 y vive en la capa de comando (leer el
    /// estado actual, no repetir el trabajo si ya está ahí), no en este agregado.
    pub fn transiciones_permitidas(self) -> &'static [EstadoDeCelula] {
        match self {
            EstadoDeCelula::Aprovisionada => {
                &[EstadoDeCelula::EnEjecucion, EstadoDeCelula::Retirada]
            }
            EstadoDeCelula::EnEjecucion => &[
                EstadoDeCelula::Suspendida,
                EstadoDeCelula::Reemparejando,
                EstadoDeCelula::Retirada,
            ],
            EstadoDeCelula::Suspendida => &[
                EstadoDeCelula::EnEjecucion,
                EstadoDeCelula::Reemparejando,
                EstadoDeCelula::Retirada,
            ],
            EstadoDeCelula::Reemparejando => &[
                EstadoDeCelula::EnEjecucion,
                EstadoDeCelula::Suspendida,
                EstadoDeCelula::Retirada,
            ],
            EstadoDeCelula::Retirada => &[],
        }
    }

    /// ¿Es `hacia` un destino legal desde este estado?
    ///
    /// Derivada de [`Self::transiciones_permitidas`] y nunca reescrita como una segunda tabla:
    /// una segunda tabla de mano podría dejar de coincidir con la primera con el tiempo.
    pub fn permite(self, hacia: EstadoDeCelula) -> bool {
        self.transiciones_permitidas().contains(&hacia)
    }

    /// ¿Es este un estado terminal, es decir, sin transiciones salientes?
    ///
    /// Derivada de [`Self::transiciones_permitidas`] en vez de mantenerse como un segundo
    /// enumerado o una segunda coincidencia: así vaciar o llenar una fila de la tabla mueve esta
    /// respuesta junto con `permite`, y las dos no pueden quedar en desacuerdo.
    pub fn es_terminal(self) -> bool {
        self.transiciones_permitidas().is_empty()
    }

    /// Intenta la transición hacia `hacia`; devuelve el nuevo estado o el rechazo tipado.
    pub fn transitar(self, hacia: EstadoDeCelula) -> Result<EstadoDeCelula, TransicionInvalida> {
        if self.permite(hacia) {
            Ok(hacia)
        } else if self.es_terminal() {
            Err(TransicionInvalida::OrigenTerminal { desde: self, hacia })
        } else {
            Err(TransicionInvalida::ParNoPermitido { desde: self, hacia })
        }
    }
}

impl std::fmt::Display for EstadoDeCelula {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let etiqueta = match self {
            EstadoDeCelula::Aprovisionada => "aprovisionada",
            EstadoDeCelula::EnEjecucion => "en ejecución",
            EstadoDeCelula::Suspendida => "suspendida",
            EstadoDeCelula::Reemparejando => "reemparejando",
            EstadoDeCelula::Retirada => "retirada",
        };
        f.write_str(etiqueta)
    }
}

/// Rechazo tipado de una transición de estado, con el par de estados implicado como datos
/// públicos y emparejables.
///
/// Superficie pública y estable a propósito: la tarea hermana HEX-074-b la empareja para mapearla
/// a un código de salida, así que no es una cadena opaca, un booleano ni un `Box<dyn Error>`.
/// Enumerado cerrado (sin `#[non_exhaustive]`) por el mismo motivo que `EstadoDeCelula`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransicionInvalida {
    /// El par ordenado (`desde`, `hacia`) no figura en la tabla de transiciones, y `desde` no es
    /// terminal.
    ParNoPermitido {
        desde: EstadoDeCelula,
        hacia: EstadoDeCelula,
    },
    /// Se intentó una transición partiendo de `Retirada`, el único estado terminal.
    OrigenTerminal {
        desde: EstadoDeCelula,
        hacia: EstadoDeCelula,
    },
}

impl TransicionInvalida {
    /// El estado de origen desde el que se intentó la transición rechazada.
    pub fn desde(&self) -> EstadoDeCelula {
        match self {
            TransicionInvalida::ParNoPermitido { desde, .. } => *desde,
            TransicionInvalida::OrigenTerminal { desde, .. } => *desde,
        }
    }

    /// El estado de destino que se intentó alcanzar y fue rechazado.
    pub fn hacia(&self) -> EstadoDeCelula {
        match self {
            TransicionInvalida::ParNoPermitido { hacia, .. } => *hacia,
            TransicionInvalida::OrigenTerminal { hacia, .. } => *hacia,
        }
    }
}

impl std::fmt::Display for TransicionInvalida {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransicionInvalida::ParNoPermitido { desde, hacia } => write!(
                f,
                "transición no permitida: de «{desde}» a «{hacia}» no figura en la tabla"
            ),
            TransicionInvalida::OrigenTerminal { desde, hacia } => write!(
                f,
                "el estado «{desde}» es terminal: no admite ninguna transición hacia «{hacia}»"
            ),
        }
    }
}

impl std::error::Error for TransicionInvalida {}

/// Raíz del agregado: una célula gobernada cuyo estado almacenado solo cambia a través de
/// [`Self::aplicar`].
///
/// El campo `estado` es privado y no existe ningún constructor, `setter` público, ni impl de
/// `From`/`TryFrom` que instale un estado arbitrario. Esa es la propiedad que este tipo
/// garantiza: ninguna llamadora puede mover una célula entre estados salvo a través de
/// `aplicar`, que valida antes de escribir.
///
/// No se agrega un constructor de rehidratación: reconstruir una célula desde el almacén de
/// estado persistente aplazado es responsabilidad de esa entrega futura, y diseñar hoy su API
/// sería anticipar una tarea que todavía no existe.
#[derive(Clone, Copy, Debug)]
pub struct CicloDeVidaDeCelula {
    estado: EstadoDeCelula,
}

impl CicloDeVidaDeCelula {
    /// El único constructor público: toda célula nueva empieza en `Aprovisionada`.
    pub fn nueva() -> Self {
        CicloDeVidaDeCelula {
            estado: EstadoDeCelula::Aprovisionada,
        }
    }

    /// El estado actual de la célula.
    pub fn estado(&self) -> EstadoDeCelula {
        self.estado
    }

    /// El único mutador: intenta mover la célula hacia `hacia`.
    ///
    /// En caso de éxito, el estado almacenado avanza. En caso de rechazo, el estado almacenado
    /// queda exactamente como estaba: la validación ocurre antes de cualquier escritura, nunca
    /// después.
    pub fn aplicar(&mut self, hacia: EstadoDeCelula) -> Result<(), TransicionInvalida> {
        let nuevo = self.estado.transitar(hacia)?;
        self.estado = nuevo;
        Ok(())
    }

    /// ¿Está la célula en el estado terminal?
    pub fn es_terminal(&self) -> bool {
        self.estado.es_terminal()
    }
}
