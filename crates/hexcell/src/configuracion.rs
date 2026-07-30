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
use crate::deduplicacion::VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO;

/// Canal seleccionado para esta célula.
///
/// Hoy solo existe una variante porque el único adaptador que existe en el árbol es el simulado
/// (`hexcell-canal-simulado`); el adaptador de canal propio ya está cerrado en la etapa A-3 y se
/// añadirá aquí como una variante más cuando esta tarea lo integre, sin tocar el resto del enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanalSeleccionado {
    /// Adaptador en memoria con semántica restrictiva de Cloud API (`hexcell-canal-simulado`).
    Simulado,
}

impl CanalSeleccionado {
    fn desde_str(valor: &str) -> Option<Self> {
        match valor {
            "simulado" => Some(Self::Simulado),
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
    /// Por defecto, `LIMITE_DE_DRENAJE_POR_DEFECTO`: diez segundos, frente al plazo de gracia
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
                    formato_esperado: "uno de: simulado",
                }
            })?,
            Err(_) => CANAL_POR_DEFECTO,
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

        Ok(Self {
            id_celula,
            ruta_datos,
            direccion_salud,
            canal,
            capacidad_cola,
            ventana_deduplicacion,
            limite_de_drenaje,
            latencia_inferencia_simulada,
            evento_simulado_de_arranque,
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
