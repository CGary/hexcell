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

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::apagado::LIMITE_DE_DRENAJE_POR_DEFECTO;
use crate::concurrencia::LIMITE_DE_CONCURRENCIA_POR_DEFECTO;
use crate::deduplicacion::VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO;

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
    /// Devuelve el primer error que encuentra; no acumula varios a la vez porque el proceso
    /// termina en el primero de todos modos y una lista de errores no cambiaría el resultado.
    pub fn desde_entorno() -> Result<Self, ErrorDeConfiguracion> {
        let id_celula = leer_obligatoria(HEXCELL_ID_CELULA, "texto no vacío, p. ej. piloto-01")?;

        let ruta_datos_str =
            leer_obligatoria(HEXCELL_RUTA_DATOS, "ruta de directorio existente en disco")?;
        let ruta_datos = PathBuf::from(&ruta_datos_str);
        if !ruta_datos.is_dir() {
            return Err(ErrorDeConfiguracion::RutaDeDatosInexistente {
                nombre: HEXCELL_RUTA_DATOS,
                ruta: ruta_datos,
            });
        }

        let direccion_salud =
            match std::env::var(HEXCELL_DIRECCION_SALUD) {
                Ok(valor) => valor.parse::<SocketAddr>().map_err(|_| {
                    ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_DIRECCION_SALUD,
                        valor: valor.clone(),
                        formato_esperado: "dirección socket, p. ej. 127.0.0.1:8081",
                    }
                })?,
                Err(_) => DIRECCION_SALUD_POR_DEFECTO,
            };

        let canal = match std::env::var(HEXCELL_CANAL) {
            Ok(valor) => CanalSeleccionado::desde_str(&valor).ok_or_else(|| {
                ErrorDeConfiguracion::ValorInvalido {
                    nombre: HEXCELL_CANAL,
                    valor: valor.clone(),
                    formato_esperado: "uno de: simulado, whatsmeow",
                }
            })?,
            Err(_) => CANAL_POR_DEFECTO,
        };

        let ruta_socket_ipc = match std::env::var(HEXCELL_SOCKET_IPC) {
            Ok(valor) => PathBuf::from(valor),
            Err(_) => PathBuf::from(RUTA_SOCKET_IPC_POR_DEFECTO),
        };

        let capacidad_cola = match std::env::var(HEXCELL_CAPACIDAD_COLA) {
            Ok(valor) => {
                valor
                    .parse::<usize>()
                    .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_CAPACIDAD_COLA,
                        valor: valor.clone(),
                        formato_esperado: "entero positivo, p. ej. 256",
                    })?
            }
            Err(_) => CAPACIDAD_COLA_POR_DEFECTO,
        };

        let ventana_deduplicacion = match std::env::var(HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS) {
            Ok(valor) => {
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
            Err(_) => VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO,
        };

        let limite_de_drenaje = match std::env::var(HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS) {
            Ok(valor) => {
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
            Err(_) => LIMITE_DE_DRENAJE_POR_DEFECTO,
        };

        let latencia_inferencia_simulada =
            match std::env::var(HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS) {
                Ok(valor) => {
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
                Err(_) => Duration::ZERO,
            };

        let evento_simulado_de_arranque = std::env::var(HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE).ok();
        let proveedor_de_inferencia_falla =
            std::env::var(HEXCELL_PROVEEDOR_DE_INFERENCIA_FALLA).is_ok();

        let defecto_gcra = hexcell_core::admision::ConfiguracionGcra::default();
        let tasa_sostenida = match std::env::var(HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO) {
            Ok(valor) => {
                valor
                    .parse::<f64>()
                    .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                        nombre: HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO,
                        valor: valor.clone(),
                        formato_esperado:
                            "número flotante positivo de peticiones por segundo, p. ej. 0.5",
                    })?
            }
            Err(_) => defecto_gcra.tasa_sostenida_por_segundo(),
        };

        let tolerancia_rafaga = match std::env::var(HEXCELL_ADMISION_TOLERANCIA_RAFAGA) {
            Ok(valor) => valor
                .parse::<u32>()
                .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                    nombre: HEXCELL_ADMISION_TOLERANCIA_RAFAGA,
                    valor: valor.clone(),
                    formato_esperado: "entero no negativo de eventos en ráfaga, p. ej. 3",
                })?,
            Err(_) => defecto_gcra.tolerancia_rafaga(),
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

        let limite_de_concurrencia = match std::env::var(HEXCELL_CONCURRENCIA_LIMITE) {
            Ok(valor) => {
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
            Err(_) => LIMITE_DE_CONCURRENCIA_POR_DEFECTO,
        };

        let presupuesto_inicial_unidades = match std::env::var(HEXCELL_PRESUPUESTO_INICIAL_UNIDADES)
        {
            Ok(valor) => valor
                .parse::<u64>()
                .map_err(|_| ErrorDeConfiguracion::ValorInvalido {
                    nombre: HEXCELL_PRESUPUESTO_INICIAL_UNIDADES,
                    valor: valor.clone(),
                    formato_esperado: "entero no negativo de unidades, p. ej. 1000",
                })?,
            Err(_) => 0,
        };

        let inferencia = match std::env::var(HEXCELL_INFERENCIA_URL_BASE) {
            Ok(url_base) if !url_base.trim().is_empty() => {
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
                    HEXCELL_INFERENCIA_API_KEY,
                    "cadena no vacía con la clave de API",
                )?;

                let modelo = leer_obligatoria(
                    HEXCELL_INFERENCIA_MODELO,
                    "nombre del modelo, p. ej. deepseek-chat",
                )?;

                let timeout = match std::env::var(HEXCELL_INFERENCIA_TIMEOUT_MS) {
                    Ok(valor) => {
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
                    Err(_) => TIMEOUT_INFERENCIA_POR_DEFECTO,
                };

                let reintentos = match std::env::var(HEXCELL_INFERENCIA_REINTENTOS) {
                    Ok(valor) => {
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
                    Err(_) => REINTENTOS_INFERENCIA_POR_DEFECTO,
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
        })
    }
}

fn leer_obligatoria(
    nombre: &'static str,
    formato_esperado: &'static str,
) -> Result<String, ErrorDeConfiguracion> {
    match std::env::var(nombre) {
        Ok(valor) if !valor.trim().is_empty() => Ok(valor),
        _ => Err(ErrorDeConfiguracion::VariableAusente {
            nombre,
            formato_esperado,
        }),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::sync::Mutex;

    static BLOQUEO_ENTORNO: Mutex<()> = Mutex::new(());

    #[test]
    fn configuracion_limite_de_concurrencia_desde_entorno() {
        let _guard = BLOQUEO_ENTORNO.lock().unwrap();

        let dir = std::env::temp_dir();
        unsafe {
            std::env::set_var(HEXCELL_ID_CELULA, "test-celula");
            std::env::set_var(HEXCELL_RUTA_DATOS, &dir);
            std::env::remove_var(HEXCELL_CONCURRENCIA_LIMITE);
        }

        // Caso por defecto: variable ausente -> LIMITE_DE_CONCURRENCIA_POR_DEFECTO (8)
        let config = Configuracion::desde_entorno().unwrap();
        assert_eq!(
            config.limite_de_concurrencia,
            LIMITE_DE_CONCURRENCIA_POR_DEFECTO
        );

        // Valor válido
        unsafe {
            std::env::set_var(HEXCELL_CONCURRENCIA_LIMITE, "16");
        }
        let config = Configuracion::desde_entorno().unwrap();
        assert_eq!(config.limite_de_concurrencia, 16);

        // Valor no numérico -> ErrorDeConfiguracion::ValorInvalido
        unsafe {
            std::env::set_var(HEXCELL_CONCURRENCIA_LIMITE, "invalido");
        }
        let err = Configuracion::desde_entorno().unwrap_err();
        assert_eq!(
            err,
            ErrorDeConfiguracion::ValorInvalido {
                nombre: HEXCELL_CONCURRENCIA_LIMITE,
                valor: "invalido".to_string(),
                formato_esperado: "entero estrictamente positivo, p. ej. 8",
            }
        );

        // Valor "0" -> ErrorDeConfiguracion::ValorInvalido
        unsafe {
            std::env::set_var(HEXCELL_CONCURRENCIA_LIMITE, "0");
        }
        let err = Configuracion::desde_entorno().unwrap_err();
        assert_eq!(
            err,
            ErrorDeConfiguracion::ValorInvalido {
                nombre: HEXCELL_CONCURRENCIA_LIMITE,
                valor: "0".to_string(),
                formato_esperado: "entero estrictamente positivo, p. ej. 8",
            }
        );

        // Limpiar entorno
        unsafe {
            std::env::remove_var(HEXCELL_ID_CELULA);
            std::env::remove_var(HEXCELL_RUTA_DATOS);
            std::env::remove_var(HEXCELL_CONCURRENCIA_LIMITE);
        }
    }
}
