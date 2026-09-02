//! Configuración de arranque del binario `hexcell`, leída de variables de entorno.
//!
//! La configuración se lee de variables de entorno — no de argumentos de línea de comandos ni de
//! un archivo — y se valida por completo antes de levantar el servidor HTTP de salud o el motor
//! de mensajería. Si falta una variable obligatoria o su valor no parsea, el proceso debe
//! terminar antes de tocar la red o el disco, con un mensaje que nombre la variable concreta y su
//! formato esperado: nunca un `panic` sin contexto ni un fallo silencioso diferido al primer uso.
//!
//! Esto importa más de lo habitual porque `[profile.release]` fija `panic = "abort"`: un `panic`
//! en el binario de producción no deja ningún mensaje utilizable. Por eso este módulo no llama a
//! `unwrap()` ni a `expect()` en ningún punto, y `main` trata el error devuelto imprimiendo su
//! forma `Display` antes de terminar con `std::process::ExitCode::FAILURE`.
//!
//! De dónde salen esos valores es una decisión de la raíz de composición, no de este módulo: la
//! lectura pasa por el puerto `FuenteDeConfiguracion`, que en producción resuelve al entorno real
//! del proceso (`EntornoDelProceso`) y en pruebas a una tabla en memoria (`FuenteEnMemoria`).

use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::apagado::LIMITE_DE_DRENAJE_POR_DEFECTO;
use crate::concurrencia::LIMITE_DE_CONCURRENCIA_POR_DEFECTO;
use crate::deduplicacion::VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO;

/// Puerto de lectura de la configuración de arranque.
///
/// Existe por corrección, no por estética. En la edición 2024 escribir el entorno del proceso es
/// `unsafe` porque `setenv` de glibc puede reasignar el array `environ` mientras otro hilo lo lee, y
/// `cargo test` corre los tests de un binario en hilos del **mismo proceso**: mientras un test
/// escribiera el entorno para preparar su caso, cualquier otro hilo que leyera una variable
/// —incluida la que consulta `std::env::temp_dir`— incurría en comportamiento indefinido. Con este
/// puerto ningún test necesita escribir nada: prepara su caso en una tabla propia y se la entrega al
/// constructor, así que ya no hay escritor contra el que competir.
///
/// Sigue el precedente que el repositorio ya fijó para el tiempo (`RelojDePrueba` frente a
/// `RelojDelSistema`): el estado ambiental se **inyecta**, no se manipula en sitio.
pub trait FuenteDeConfiguracion {
    /// Devuelve el valor asociado a `nombre`, o `None` si no está definido.
    fn leer(&self, nombre: &str) -> Option<String>;
}

/// Fuente de producción: el entorno real del proceso.
///
/// Es el único punto de todo el crate que llama a `std::env::var`, y **solo lee**.
#[derive(Clone, Copy, Debug, Default)]
pub struct EntornoDelProceso;

impl FuenteDeConfiguracion for EntornoDelProceso {
    fn leer(&self, nombre: &str) -> Option<String> {
        // `Err` cubre tanto «variable ausente» como «valor que no es UTF-8 válido». Ambos casos se
        // tratan igual que antes de la inyección —la variable se considera no definida—, para que
        // el comportamiento de producción sea idéntico al de antes de este cambio.
        std::env::var(nombre).ok()
    }
}

/// Fuente en memoria: tabla de nombre a valor, privada de quien la construye.
///
/// **No** está detrás de `#[cfg(test)]` a propósito: los tests de integración de
/// `crates/hexcell/tests/` compilan como crates externos y no verían un elemento condicionado a la
/// compilación de pruebas de esta biblioteca. Al ser un valor local, dos tests concurrentes no
/// comparten absolutamente nada.
#[derive(Clone, Debug, Default)]
pub struct FuenteEnMemoria {
    valores: BTreeMap<String, String>,
}

impl FuenteEnMemoria {
    /// Construye una fuente sin ninguna variable definida.
    #[must_use]
    pub fn vacia() -> Self {
        Self::default()
    }

    /// Define una variable y devuelve la fuente, para encadenar la preparación de un caso.
    #[must_use]
    pub fn con(mut self, nombre: &str, valor: impl Into<String>) -> Self {
        self.fijar(nombre, valor);
        self
    }

    /// Define o reemplaza una variable sobre una fuente ya construida.
    pub fn fijar(&mut self, nombre: &str, valor: impl Into<String>) {
        self.valores.insert(nombre.to_string(), valor.into());
    }

    /// Elimina una variable, para ejercer el caso «no definida» sin reconstruir la fuente entera.
    pub fn quitar(&mut self, nombre: &str) {
        self.valores.remove(nombre);
    }
}

impl FuenteDeConfiguracion for FuenteEnMemoria {
    fn leer(&self, nombre: &str) -> Option<String> {
        self.valores.get(nombre).cloned()
    }
}

/// Canal seleccionado para esta célula.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanalSeleccionado {
    /// Adaptador en memoria con semántica restrictiva de Cloud API (`hexcell-canal-simulado`).
    Simulado,
    /// Adaptador sobre IPC con el sidecar whatsmeow (`hexcell-canal-whatsmeow`).
    Whatsmeow,
}

impl CanalSeleccionado {
    fn desde_str(valor: &str) -> Option<Self> {
        match valor {
            "simulado" => Some(Self::Simulado),
            "whatsmeow" => Some(Self::Whatsmeow),
            _ => None,
        }
    }
}

/// Configuración de embeddings según el proveedor seleccionado.
#[derive(Clone, Debug)]
pub enum ConfiguracionDeEmbeddingsSegunProveedor {
    /// Proveedor compatible con OpenAI/OpenRouter.
    OpenRouter(crate::proveedor_embeddings::ConfiguracionDeEmbeddings),
    /// Proveedor de Gemini (Google AI Studio).
    Gemini(crate::proveedor_embeddings_gemini::ConfiguracionDeEmbeddingsGemini),
}

/// Configuración de arranque, ya validada, del binario de la célula.
#[derive(Clone, Debug)]
pub struct Configuracion {
    /// Identificador de esta célula, usado para distinguirla en los registros y en el futuro
    /// panel de administración.
    pub id_celula: String,
    /// Ruta del volumen de datos de la célula, validada como existente en disco al arrancar.
    pub ruta_datos: PathBuf,
    /// Dirección donde escucha el servidor HTTP interno de salud. Por defecto, loopback: esta
    /// ruta no es de cara al público, la sondea la CLI de administración.
    pub direccion_salud: SocketAddr,
    /// Canal configurado para esta célula.
    pub canal: CanalSeleccionado,
    /// Ruta del socket Unix de comunicación IPC con el sidecar whatsmeow.
    ///
    /// Solo la lee el brazo `CanalSeleccionado::Whatsmeow` de la raíz de composición. Por
    /// defecto, `RUTA_SOCKET_IPC_POR_DEFECTO`: `/var/lib/hexcell/ipc/sidecar.sock`.
    pub ruta_socket_ipc: PathBuf,
    /// Capacidad del canal `mpsc` acotado por el que el adaptador entrega sus eventos al motor.
    pub capacidad_cola: usize,
    /// Ventana de retención del registro de deduplicación del motor (`crate::deduplicacion`).
    ///
    /// Por defecto, `VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO`: una hora, cuya
    /// justificación completa vive en `crate::deduplicacion`, no aquí. La cifra definitiva sigue
    /// siendo una decisión de producto abierta (`docs/STATUS.md`, entrada `Pendiente` del
    /// 2026-07-30); esta variable es la puerta explícita para ajustarla sin recompilar.
    pub ventana_deduplicacion: Duration,
    /// Límite temporal de drenaje tras la señal de apagado (`crate::apagado`).
    ///
    /// Por defecto, `LIMITE_DE_DRENAJE_POR_DEFECTO`: veinte segundos, frente al plazo de gracia
    /// total de treinta segundos que fija el PRD para todo el proceso.
    pub limite_de_drenaje: Duration,
    /// Latencia artificial del proveedor de inferencia simulado, antes de responder.
    ///
    /// Solo la lee `crate::inferencia::ProveedorSimulado`. Por defecto cero: no crea ningún
    /// temporizador y no cambia ninguna salida. Existe para que un test de proceso real pueda
    /// demostrar que un evento en vuelo durante `SIGTERM` se completa (AC-7): sin ella, la
    /// inferencia simulada responde en microsegundos y la condición dejaría de ser falsificable.
    pub latencia_inferencia_simulada: Duration,
    /// Contenido de un evento sintético que `main` inyecta al arrancar por el canal simulado.
    ///
    /// Solo lo lee el brazo `CanalSeleccionado::Simulado` de la raíz de composición. El canal
    /// simulado no tiene ninguna fuente externa de eventos —`AdaptadorSimulado::inyectar` es un
    /// método en proceso—, así que sin esta variable un binario real corriendo sobre el canal
    /// simulado nunca podría recibir un evento desde fuera, y los criterios de aceptación AC-5 a
    /// AC-9, que exigen un proceso real, serían imposibles de comprobar.
    pub evento_simulado_de_arranque: Option<String>,
    /// Si está presente (con cualquier valor), el proveedor de inferencia simulado falla siempre.
    ///
    /// Solo la lee el brazo `CanalSeleccionado::Simulado` de la raíz de composición, para que un
    /// test de proceso real pueda comprobar que el motor registra `inferencia_sin_respuesta` (y
    /// no envía nada) cuando el proveedor falla, sin necesidad de un proveedor real ni de tocar
    /// producción: por defecto, ausente, el proveedor nunca falla.
    pub proveedor_de_inferencia_falla: bool,
    /// Configuración de límites para el algoritmo de admisión GCRA (`hexcell_core::admision::ConfiguracionGcra`).
    pub configuracion_gcra: hexcell_core::admision::ConfiguracionGcra,
    /// Límite estricto de concurrencia de tareas en vuelo por contenedor (`crate::concurrencia`).
    pub limite_de_concurrencia: usize,
    /// Unidades de presupuesto inicial acreditadas en la primera puesta en marcha (opcional, por defecto 0).
    pub presupuesto_inicial_unidades: u64,
    /// Configuración opcional del proveedor de inferencia HTTPS real compatible con OpenAI.
    pub inferencia: Option<crate::proveedor_openai::ConfiguracionDeInferencia>,
    /// Configuración opcional del proveedor de incrustaciones HTTPS real (OpenRouter o Gemini).
    pub embeddings: Option<ConfiguracionDeEmbeddingsSegunProveedor>,
}

/// Error de configuración: nombra siempre la variable concreta y su formato esperado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeConfiguracion {
    /// La variable obligatoria no está presente en el entorno.
    VariableAusente {
        /// Nombre exacto de la variable de entorno.
        nombre: &'static str,
        /// Descripción, en español, del formato que se esperaba.
        formato_esperado: &'static str,
    },
    /// La variable está presente pero su valor no parsea al tipo esperado.
    ValorInvalido {
        /// Nombre exacto de la variable de entorno.
        nombre: &'static str,
        /// Valor recibido, tal cual, para que el mensaje sea accionable.
        valor: String,
        /// Descripción, en español, del formato que se esperaba.
        formato_esperado: &'static str,
    },
    /// La ruta de datos de la célula no existe en disco.
    RutaDeDatosInexistente {
        /// Nombre exacto de la variable de entorno que la declaró.
        nombre: &'static str,
        /// Ruta que no se encontró.
        ruta: PathBuf,
    },
}

impl fmt::Display for ErrorDeConfiguracion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VariableAusente {
                nombre,
                formato_esperado,
            } => write!(
                f,
                "falta la variable de entorno obligatoria {nombre} (formato esperado: {formato_esperado})"
            ),
            Self::ValorInvalido {
                nombre,
                valor,
                formato_esperado,
            } => write!(
                f,
                "la variable de entorno {nombre} tiene un valor inválido: «{valor}» \
                 (formato esperado: {formato_esperado})"
            ),
            Self::RutaDeDatosInexistente { nombre, ruta } => write!(
                f,
                "la ruta indicada por {nombre} no existe en disco: {ruta}",
                ruta = ruta.display()
            ),
        }
    }
}

impl std::error::Error for ErrorDeConfiguracion {}

/// Nombre de la variable de entorno con el identificador de la célula (obligatoria).
pub const HEXCELL_ID_CELULA: &str = "HEXCELL_ID_CELULA";
/// Nombre de la variable de entorno con la ruta de datos de la célula (obligatoria).
pub const HEXCELL_RUTA_DATOS: &str = "HEXCELL_RUTA_DATOS";
/// Nombre de la variable de entorno con la dirección del servidor de salud (opcional).
pub const HEXCELL_DIRECCION_SALUD: &str = "HEXCELL_DIRECCION_SALUD";
/// Nombre de la variable de entorno con la ruta del socket IPC (opcional).
pub const HEXCELL_SOCKET_IPC: &str = "HEXCELL_SOCKET_IPC";
/// Nombre de la variable de entorno con el canal configurado (opcional).
pub const HEXCELL_CANAL: &str = "HEXCELL_CANAL";
/// Nombre de la variable de entorno con la capacidad del canal de eventos (opcional).
pub const HEXCELL_CAPACIDAD_COLA: &str = "HEXCELL_CAPACIDAD_COLA";
/// Nombre de la variable de entorno con la ventana de retención de deduplicación, en segundos
/// (opcional).
pub const HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS: &str = "HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS";
/// Nombre de la variable de entorno con el límite de drenaje del apagado ordenado, en segundos
/// (opcional).
pub const HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS: &str = "HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS";
/// Nombre de la variable de entorno con la latencia artificial del proveedor de inferencia
/// simulado, en milisegundos (opcional, solo para tests).
pub const HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS: &str = "HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS";
/// Nombre de la variable de entorno con el contenido de un evento sintético de arranque para el
/// canal simulado (opcional, solo para tests).
pub const HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE: &str = "HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE";
/// Nombre de la variable de entorno que fuerza que el proveedor de inferencia simulado falle
/// siempre (opcional, solo para tests; su presencia basta, el valor no se interpreta).
pub const HEXCELL_PROVEEDOR_DE_INFERENCIA_FALLA: &str = "HEXCELL_PROVEEDOR_DE_INFERENCIA_FALLA";
/// Nombre de la variable de entorno con la tasa sostenida de admisión GCRA por segundo (opcional).
pub const HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO: &str =
    "HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO";
/// Nombre de la variable de entorno con la tolerancia a ráfaga de admisión GCRA (opcional).
pub const HEXCELL_ADMISION_TOLERANCIA_RAFAGA: &str = "HEXCELL_ADMISION_TOLERANCIA_RAFAGA";
/// Nombre de la variable de entorno con el límite estricto de concurrencia por contenedor (opcional).
pub const HEXCELL_CONCURRENCIA_LIMITE: &str = "HEXCELL_CONCURRENCIA_LIMITE";
/// Nombre de la variable de entorno con el presupuesto inicial en unidades (opcional, por defecto 0).
pub const HEXCELL_PRESUPUESTO_INICIAL_UNIDADES: &str = "HEXCELL_PRESUPUESTO_INICIAL_UNIDADES";
/// Nombre de la variable de entorno con la URL base del proveedor de inferencia OpenAI (opcional, su presencia activa el proveedor real).
pub const HEXCELL_INFERENCIA_URL_BASE: &str = "HEXCELL_INFERENCIA_URL_BASE";
/// Nombre de la variable de entorno con la clave de API del proveedor de inferencia (obligatoria si URL_BASE está presente).
pub const HEXCELL_INFERENCIA_API_KEY: &str = "HEXCELL_INFERENCIA_API_KEY";
/// Nombre de la variable de entorno con el nombre del modelo de inferencia (obligatorio si URL_BASE está presente).
pub const HEXCELL_INFERENCIA_MODELO: &str = "HEXCELL_INFERENCIA_MODELO";
/// Nombre de la variable de entorno con el tiempo de espera de inferencia en milisegundos (opcional).
pub const HEXCELL_INFERENCIA_TIMEOUT_MS: &str = "HEXCELL_INFERENCIA_TIMEOUT_MS";
/// Nombre de la variable de entorno con la cantidad de reintentos de inferencia (opcional).
pub const HEXCELL_INFERENCIA_REINTENTOS: &str = "HEXCELL_INFERENCIA_REINTENTOS";

/// Tiempo de espera de inferencia por defecto: 8000 milisegundos.
pub const TIMEOUT_INFERENCIA_POR_DEFECTO: Duration = Duration::from_millis(8000);
/// Cantidad de reintentos de inferencia por defecto: 1.
pub const REINTENTOS_INFERENCIA_POR_DEFECTO: u32 = 1;

/// Nombre de la variable de entorno con la URL base del proveedor de embeddings (opcional, su presencia activa el proveedor real).
pub const HEXCELL_EMBEDDINGS_URL_BASE: &str = "HEXCELL_EMBEDDINGS_URL_BASE";
/// Nombre de la variable de entorno con la clave de API del proveedor de embeddings (obligatoria si URL_BASE está presente).
pub const HEXCELL_EMBEDDINGS_API_KEY: &str = "HEXCELL_EMBEDDINGS_API_KEY";
/// Nombre de la variable de entorno con el nombre del modelo de embeddings (obligatorio si URL_BASE está presente).
pub const HEXCELL_EMBEDDINGS_MODELO: &str = "HEXCELL_EMBEDDINGS_MODELO";
/// Nombre de la variable de entorno con el tiempo de espera de embeddings en milisegundos (opcional).
pub const HEXCELL_EMBEDDINGS_TIMEOUT_MS: &str = "HEXCELL_EMBEDDINGS_TIMEOUT_MS";
/// Nombre de la variable de entorno con la cantidad de reintentos de embeddings (opcional).
pub const HEXCELL_EMBEDDINGS_REINTENTOS: &str = "HEXCELL_EMBEDDINGS_REINTENTOS";
/// Nombre de la variable de entorno con el tamaño máximo de lote de embeddings (opcional).
pub const HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE: &str = "HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE";
/// Nombre de la variable de entorno con el proveedor de embeddings seleccionado (opcional).
pub const HEXCELL_EMBEDDINGS_PROVEEDOR: &str = "HEXCELL_EMBEDDINGS_PROVEEDOR";

/// Tiempo de espera de embeddings por defecto: 8000 milisegundos.
pub const TIMEOUT_EMBEDDINGS_POR_DEFECTO: Duration = Duration::from_millis(8000);
/// Cantidad de reintentos de embeddings por defecto: 1.
pub const REINTENTOS_EMBEDDINGS_POR_DEFECTO: u32 = 1;
/// Tamaño de lote de embeddings por defecto: 32.
pub const TAMANO_DE_LOTE_EMBEDDINGS_POR_DEFECTO: usize = 32;

/// Dirección de salud por defecto: loopback (127.0.0.1), nunca `0.0.0.0`. Una célula sobre canal
/// propio empaquetada en un contenedor (etapa A-6) necesita sondear esta ruta desde un
/// contenedor hermano, y para eso existe `HEXCELL_DIRECCION_SALUD` como puerta explícita.
///
/// Se construye como constante a partir de `Ipv4Addr::LOCALHOST`, sin parsear ninguna cadena en
/// tiempo de arranque: así el valor por defecto no puede fallar a parsear, y este módulo no
/// necesita `expect()` para tratar un caso que en realidad nunca ocurre.
const DIRECCION_SALUD_POR_DEFECTO: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8081);
/// Canal por defecto cuando no se configura ninguno: el único que existe hoy en el árbol.
const CANAL_POR_DEFECTO: CanalSeleccionado = CanalSeleccionado::Simulado;
/// Ruta por omisión del socket IPC documentada en el protocolo.
pub const RUTA_SOCKET_IPC_POR_DEFECTO: &str = "/var/lib/hexcell/ipc/sidecar.sock";
/// Capacidad por defecto del canal `mpsc` acotado.
const CAPACIDAD_COLA_POR_DEFECTO: usize = 256;

impl Configuracion {
    /// Lee y valida la configuración completa a partir de las variables de entorno del proceso.
    ///
    /// Envoltorio delgado de producción sobre `desde_fuente`: la única razón de que siga
    /// existiendo es que la raíz de composición (`main`) no tenga que conocer el puerto ni
    /// construir un adaptador para el caso normal. Toda la lógica vive en `desde_fuente`.
    pub fn desde_entorno() -> Result<Self, ErrorDeConfiguracion> {
        Self::desde_fuente(&EntornoDelProceso)
    }

    /// Lee y valida la configuración completa a partir de la fuente inyectada.
    ///
    /// La fuente se recibe como parámetro y se consulta entera aquí dentro; no se guarda en ningún
    /// campo ni en ningún global, porque retenerla más allá de la construcción conservaría un asa
    /// viva sobre el entorno del proceso: justo el acoplamiento que este puerto elimina.
    ///
    /// Devuelve el primer error que encuentra; no acumula varios a la vez porque el proceso
    /// termina en el primero de todos modos y una lista de errores no cambiaría el resultado.
    pub fn desde_fuente(fuente: &dyn FuenteDeConfiguracion) -> Result<Self, ErrorDeConfiguracion> {
        let id_celula = leer_obligatoria(
            fuente,
            HEXCELL_ID_CELULA,
            "texto no vacío, p. ej. piloto-01",
        )?;

        let ruta_datos_str = leer_obligatoria(
            fuente,
            HEXCELL_RUTA_DATOS,
            "ruta de directorio existente en disco",
        )?;
        let ruta_datos = PathBuf::from(&ruta_datos_str);
        if !ruta_datos.is_dir() {
            return Err(ErrorDeConfiguracion::RutaDeDatosInexistente {
                nombre: HEXCELL_RUTA_DATOS,
                ruta: ruta_datos,
            });
        }

        let direccion_salud =
            match fuente.leer(HEXCELL_DIRECCION_SALUD) {
                Some(valor) => valor.parse::<SocketAddr>().map_err(|_| {
                    ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_DIRECCION_SALUD,
                        valor: valor.clone(),
                        formato_esperado: "dirección socket, p. ej. 127.0.0.1:8081",
                    }
                })?,
                None => DIRECCION_SALUD_POR_DEFECTO,
            };

        let canal = match fuente.leer(HEXCELL_CANAL) {
            Some(valor) => CanalSeleccionado::desde_str(&valor).ok_or_else(|| {
                ErrorDeConfiguracion::ValorInvalido {
                    nombre: HEXCELL_CANAL,
                    valor: valor.clone(),
                    formato_esperado: "uno de: simulado, whatsmeow",
                }
            })?,
            None => CANAL_POR_DEFECTO,
        };

        let ruta_socket_ipc = match fuente.leer(HEXCELL_SOCKET_IPC) {
            Some(valor) => PathBuf::from(valor),
            None => PathBuf::from(RUTA_SOCKET_IPC_POR_DEFECTO),
        };

        let capacidad_cola = match fuente.leer(HEXCELL_CAPACIDAD_COLA) {
            Some(valor) => {
                valor
                    .parse::<usize>()
                    .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_CAPACIDAD_COLA,
                        valor: valor.clone(),
                        formato_esperado: "entero positivo, p. ej. 256",
                    })?
            }
            None => CAPACIDAD_COLA_POR_DEFECTO,
        };

        let ventana_deduplicacion = match fuente.leer(HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS) {
            Some(valor) => {
                let segundos =
                    valor
                        .parse::<u64>()
                        .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                            nombre: HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS,
                            valor: valor.clone(),
                            formato_esperado: "entero positivo de segundos, p. ej. 1800",
                        })?;
                Duration::from_secs(segundos)
            }
            None => VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO,
        };

        let limite_de_drenaje = match fuente.leer(HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS) {
            Some(valor) => {
                let segundos =
                    valor
                        .parse::<u64>()
                        .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                            nombre: HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS,
                            valor: valor.clone(),
                            formato_esperado: "entero positivo de segundos, p. ej. 10",
                        })?;
                Duration::from_secs(segundos)
            }
            None => LIMITE_DE_DRENAJE_POR_DEFECTO,
        };

        let latencia_inferencia_simulada =
            match fuente.leer(HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS) {
                Some(valor) => {
                    let milisegundos =
                        valor
                            .parse::<u64>()
                            .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS,
                                valor: valor.clone(),
                                formato_esperado: "entero no negativo de milisegundos, p. ej. 1500",
                            })?;
                    Duration::from_millis(milisegundos)
                }
                None => Duration::ZERO,
            };

        let evento_simulado_de_arranque = fuente.leer(HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE);
        let proveedor_de_inferencia_falla =
            fuente.leer(HEXCELL_PROVEEDOR_DE_INFERENCIA_FALLA).is_some();

        let defecto_gcra = hexcell_core::admision::ConfiguracionGcra::default();
        let tasa_sostenida = match fuente.leer(HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO) {
            Some(valor) => {
                valor
                    .parse::<f64>()
                    .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO,
                        valor: valor.clone(),
                        formato_esperado:
                            "número flotante positivo de peticiones por segundo, p. ej. 0.5",
                    })?
            }
            None => defecto_gcra.tasa_sostenida_por_segundo(),
        };

        let tolerancia_rafaga = match fuente.leer(HEXCELL_ADMISION_TOLERANCIA_RAFAGA) {
            Some(valor) => {
                valor
                    .parse::<u32>()
                    .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_ADMISION_TOLERANCIA_RAFAGA,
                        valor: valor.clone(),
                        formato_esperado: "entero no negativo de eventos en ráfaga, p. ej. 3",
                    })?
            }
            None => defecto_gcra.tolerancia_rafaga(),
        };

        let configuracion_gcra = hexcell_core::admision::ConfiguracionGcra::nueva(
            tasa_sostenida,
            tolerancia_rafaga,
        )
        .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
            nombre: HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO,
            valor: tasa_sostenida.to_string(),
            formato_esperado: "número flotante positivo de peticiones por segundo, p. ej. 0.5",
        })?;

        let limite_de_concurrencia = match fuente.leer(HEXCELL_CONCURRENCIA_LIMITE) {
            Some(valor) => {
                let parsed =
                    valor
                        .parse::<usize>()
                        .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                            nombre: HEXCELL_CONCURRENCIA_LIMITE,
                            valor: valor.clone(),
                            formato_esperado: "entero estrictamente positivo, p. ej. 8",
                        })?;
                if parsed == 0 {
                    return Err(ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_CONCURRENCIA_LIMITE,
                        valor: valor.clone(),
                        formato_esperado: "entero estrictamente positivo, p. ej. 8",
                    });
                }
                parsed
            }
            None => LIMITE_DE_CONCURRENCIA_POR_DEFECTO,
        };

        let presupuesto_inicial_unidades = match fuente.leer(HEXCELL_PRESUPUESTO_INICIAL_UNIDADES) {
            Some(valor) => {
                valor
                    .parse::<u64>()
                    .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_PRESUPUESTO_INICIAL_UNIDADES,
                        valor: valor.clone(),
                        formato_esperado: "entero no negativo de unidades, p. ej. 1000",
                    })?
            }
            None => 0,
        };

        let inferencia = match fuente.leer(HEXCELL_INFERENCIA_URL_BASE) {
            Some(url_base) if !url_base.trim().is_empty() => {
                let url_base = url_base.trim().to_string();
                if let Ok(uri) = url_base.parse::<hyper::Uri>() {
                    let scheme = uri.scheme_str().unwrap_or("");
                    let host = uri.host().unwrap_or("");
                    let es_loopback = host == "127.0.0.1"
                        || host == "localhost"
                        || host == "::1"
                        || host == "[::1]";
                    if scheme != "https" && (scheme != "http" || !es_loopback) {
                        return Err(ErrorDeConfiguracion::ValorInvalido {
                            nombre: HEXCELL_INFERENCIA_URL_BASE,
                            valor: url_base,
                            formato_esperado: "URL con esquema https:// (o http:// solo para loopback)",
                        });
                    }
                } else {
                    return Err(ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_INFERENCIA_URL_BASE,
                        valor: url_base,
                        formato_esperado: "URL válida",
                    });
                }

                let api_key = leer_obligatoria(
                    fuente,
                    HEXCELL_INFERENCIA_API_KEY,
                    "cadena no vacía con la clave de API",
                )?;

                let modelo = leer_obligatoria(
                    fuente,
                    HEXCELL_INFERENCIA_MODELO,
                    "nombre del modelo, p. ej. deepseek-chat",
                )?;

                let timeout = match fuente.leer(HEXCELL_INFERENCIA_TIMEOUT_MS) {
                    Some(valor) => {
                        let ms = valor.parse::<u64>().map_err(|_| {
                            ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_INFERENCIA_TIMEOUT_MS,
                                valor: valor.clone(),
                                formato_esperado:
                                    "entero estrictamente positivo de milisegundos, p. ej. 8000",
                            }
                        })?;
                        if ms == 0 {
                            return Err(ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_INFERENCIA_TIMEOUT_MS,
                                valor: valor.clone(),
                                formato_esperado: "entero estrictamente positivo de milisegundos, p. ej. 8000",
                            });
                        }
                        Duration::from_millis(ms)
                    }
                    None => TIMEOUT_INFERENCIA_POR_DEFECTO,
                };

                let reintentos = match fuente.leer(HEXCELL_INFERENCIA_REINTENTOS) {
                    Some(valor) => {
                        let r = valor.parse::<u32>().map_err(|_| {
                            ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_INFERENCIA_REINTENTOS,
                                valor: valor.clone(),
                                formato_esperado: "entero no negativo menor o igual a 3, p. ej. 1",
                            }
                        })?;
                        if r > 3 {
                            return Err(ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_INFERENCIA_REINTENTOS,
                                valor: valor.clone(),
                                formato_esperado: "entero no negativo menor o igual a 3, p. ej. 1",
                            });
                        }
                        r
                    }
                    None => REINTENTOS_INFERENCIA_POR_DEFECTO,
                };

                let tiempo_maximo_inferencia = timeout * (1 + reintentos);
                if tiempo_maximo_inferencia >= limite_de_drenaje {
                    return Err(ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_INFERENCIA_URL_BASE,
                        valor: url_base,
                        formato_esperado: "tiempo total de inferencia (timeout * (1 + reintentos)) estrictamente menor que el límite de drenaje",
                    });
                }

                Some(crate::proveedor_openai::ConfiguracionDeInferencia {
                    url_base,
                    api_key,
                    modelo,
                    timeout,
                    reintentos,
                })
            }
            _ => None,
        };

        let embeddings = match fuente.leer(HEXCELL_EMBEDDINGS_URL_BASE) {
            Some(url_base) if !url_base.trim().is_empty() => {
                let url_base = url_base.trim().to_string();
                if let Ok(uri) = url_base.parse::<hyper::Uri>() {
                    let scheme = uri.scheme_str().unwrap_or("");
                    let host = uri.host().unwrap_or("");
                    let es_loopback = host == "127.0.0.1"
                        || host == "localhost"
                        || host == "::1"
                        || host == "[::1]";
                    if scheme != "https" && (scheme != "http" || !es_loopback) {
                        return Err(ErrorDeConfiguracion::ValorInvalido {
                            nombre: HEXCELL_EMBEDDINGS_URL_BASE,
                            valor: url_base,
                            formato_esperado: "URL con esquema https:// (o http:// solo para loopback)",
                        });
                    }
                } else {
                    return Err(ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_EMBEDDINGS_URL_BASE,
                        valor: url_base,
                        formato_esperado: "URL válida",
                    });
                }

                let api_key = leer_obligatoria(
                    fuente,
                    HEXCELL_EMBEDDINGS_API_KEY,
                    "cadena no vacía con la clave de API",
                )?;

                let modelo = leer_obligatoria(
                    fuente,
                    HEXCELL_EMBEDDINGS_MODELO,
                    "nombre del modelo, p. ej. text-embedding-3-small",
                )?;

                let timeout = match fuente.leer(HEXCELL_EMBEDDINGS_TIMEOUT_MS) {
                    Some(valor) => {
                        let ms = valor.parse::<u64>().map_err(|_| {
                            ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_TIMEOUT_MS,
                                valor: valor.clone(),
                                formato_esperado:
                                    "entero estrictamente positivo de milisegundos, p. ej. 8000",
                            }
                        })?;
                        if ms == 0 {
                            return Err(ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_TIMEOUT_MS,
                                valor: valor.clone(),
                                formato_esperado: "entero estrictamente positivo de milisegundos, p. ej. 8000",
                            });
                        }
                        Duration::from_millis(ms)
                    }
                    None => TIMEOUT_EMBEDDINGS_POR_DEFECTO,
                };

                let reintentos = match fuente.leer(HEXCELL_EMBEDDINGS_REINTENTOS) {
                    Some(valor) => {
                        let r = valor.parse::<u32>().map_err(|_| {
                            ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_REINTENTOS,
                                valor: valor.clone(),
                                formato_esperado: "entero no negativo menor o igual a 3, p. ej. 1",
                            }
                        })?;
                        if r > 3 {
                            return Err(ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_REINTENTOS,
                                valor: valor.clone(),
                                formato_esperado: "entero no negativo menor o igual a 3, p. ej. 1",
                            });
                        }
                        r
                    }
                    None => REINTENTOS_EMBEDDINGS_POR_DEFECTO,
                };

                let tamano_de_lote = match fuente.leer(HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE) {
                    Some(valor) => {
                        let tam = valor.parse::<usize>().map_err(|_| {
                            ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE,
                                valor: valor.clone(),
                                formato_esperado: "entero positivo entre 1 y 128, p. ej. 32",
                            }
                        })?;
                        if !(1..=128).contains(&tam) {
                            return Err(ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE,
                                valor: valor.clone(),
                                formato_esperado: "entero positivo entre 1 y 128, p. ej. 32",
                            });
                        }
                        tam
                    }
                    None => TAMANO_DE_LOTE_EMBEDDINGS_POR_DEFECTO,
                };

                let tiempo_maximo_embeddings =
                    timeout * (1 + reintentos) + Duration::from_millis(u64::from(reintentos) * 250);
                if tiempo_maximo_embeddings >= limite_de_drenaje {
                    return Err(ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_EMBEDDINGS_URL_BASE,
                        valor: url_base,
                        formato_esperado: "tiempo total de embeddings (timeout * (1 + reintentos) + reintentos * 250ms) estrictamente menor que el límite de drenaje",
                    });
                }

                let proveedor_str = match fuente.leer(HEXCELL_EMBEDDINGS_PROVEEDOR) {
                    Some(val) => {
                        let trimmed = val.trim();
                        if trimmed == "openrouter" || trimmed == "gemini" {
                            trimmed.to_string()
                        } else {
                            return Err(ErrorDeConfiguracion::ValorInvalido {
                                nombre: HEXCELL_EMBEDDINGS_PROVEEDOR,
                                valor: val,
                                formato_esperado: "uno de: openrouter | gemini",
                            });
                        }
                    }
                    None => "openrouter".to_string(),
                };

                match proveedor_str.as_str() {
                    "openrouter" => Some(ConfiguracionDeEmbeddingsSegunProveedor::OpenRouter(
                        crate::proveedor_embeddings::ConfiguracionDeEmbeddings {
                            url_base,
                            api_key,
                            modelo,
                            timeout,
                            reintentos,
                            tamano_de_lote,
                        },
                    )),
                    "gemini" => Some(ConfiguracionDeEmbeddingsSegunProveedor::Gemini(
                        crate::proveedor_embeddings_gemini::ConfiguracionDeEmbeddingsGemini {
                            url_base,
                            api_key,
                            modelo,
                            timeout,
                            reintentos,
                            tamano_de_lote,
                        },
                    )),
                    _ => unreachable!(),
                }
            }
            _ => None,
        };

        Ok(Self {
            id_celula,
            ruta_datos,
            direccion_salud,
            canal,
            ruta_socket_ipc,
            capacidad_cola,
            ventana_deduplicacion,
            limite_de_drenaje,
            latencia_inferencia_simulada,
            evento_simulado_de_arranque,
            proveedor_de_inferencia_falla,
            configuracion_gcra,
            limite_de_concurrencia,
            presupuesto_inicial_unidades,
            inferencia,
            embeddings,
        })
    }
}

fn leer_obligatoria(
    fuente: &dyn FuenteDeConfiguracion,
    nombre: &'static str,
    formato_esperado: &'static str,
) -> Result<String, ErrorDeConfiguracion> {
    match fuente.leer(nombre) {
        Some(valor) if !valor.trim().is_empty() => Ok(valor),
        _ => Err(ErrorDeConfiguracion::VariableAusente {
            nombre,
            formato_esperado,
        }),
    }
}

/// Que `desde_entorno` siga leyendo el entorno real del proceso no se comprueba aquí sino en
/// `crates/hexcell/tests/configuracion.rs`, lanzando el binario de verdad con un entorno de hijo
/// controlado: es la única forma de demostrarlo sin escribir el entorno de este proceso.
#[cfg(test)]
mod pruebas {
    use super::*;

    /// Fuente mínima válida: las dos variables obligatorias, con una ruta de datos que existe.
    fn fuente_valida() -> FuenteEnMemoria {
        let dir = std::env::temp_dir();
        FuenteEnMemoria::vacia()
            .con(HEXCELL_ID_CELULA, "test-celula")
            .con(HEXCELL_RUTA_DATOS, dir.to_string_lossy())
    }

    #[test]
    fn configuracion_limite_de_concurrencia_desde_la_fuente() {
        // Cada caso trabaja sobre su propia tabla en memoria: ya no hay estado de proceso que
        // serializar, así que este test no necesita ningún cerrojo ni limpieza posterior.
        let mut fuente = fuente_valida();

        // Caso por defecto: variable ausente -> LIMITE_DE_CONCURRENCIA_POR_DEFECTO (8)
        let config = Configuracion::desde_fuente(&fuente).unwrap();
        assert_eq!(
            config.limite_de_concurrencia,
            LIMITE_DE_CONCURRENCIA_POR_DEFECTO
        );

        // Valor válido
        fuente.fijar(HEXCELL_CONCURRENCIA_LIMITE, "16");
        let config = Configuracion::desde_fuente(&fuente).unwrap();
        assert_eq!(config.limite_de_concurrencia, 16);

        // Valor no numérico -> ErrorDeConfiguracion::ValorInvalido
        fuente.fijar(HEXCELL_CONCURRENCIA_LIMITE, "invalido");
        let err = Configuracion::desde_fuente(&fuente).unwrap_err();
        assert_eq!(
            err,
            ErrorDeConfiguracion::ValorInvalido {
                nombre: HEXCELL_CONCURRENCIA_LIMITE,
                valor: "invalido".to_string(),
                formato_esperado: "entero estrictamente positivo, p. ej. 8",
            }
        );

        // Valor "0" -> ErrorDeConfiguracion::ValorInvalido
        fuente.fijar(HEXCELL_CONCURRENCIA_LIMITE, "0");
        let err = Configuracion::desde_fuente(&fuente).unwrap_err();
        assert_eq!(
            err,
            ErrorDeConfiguracion::ValorInvalido {
                nombre: HEXCELL_CONCURRENCIA_LIMITE,
                valor: "0".to_string(),
                formato_esperado: "entero estrictamente positivo, p. ej. 8",
            }
        );

        // Quitar la variable devuelve el valor por omisión sin reconstruir la fuente.
        fuente.quitar(HEXCELL_CONCURRENCIA_LIMITE);
        let config = Configuracion::desde_fuente(&fuente).unwrap();
        assert_eq!(
            config.limite_de_concurrencia,
            LIMITE_DE_CONCURRENCIA_POR_DEFECTO
        );
    }
}
