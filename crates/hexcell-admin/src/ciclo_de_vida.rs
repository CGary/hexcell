//! Operaciones Docker del ciclo de vida de una célula.

use std::fmt;

use crate::docker::{ClienteDocker, ErrorDeClienteDocker, OpcionesDeContenedor};

/// Intervalo entre intentos de la sonda, en milisegundos.
pub const CADENCIA_DE_SONDEO_MS: u64 = 100;
/// Tiempo máximo que se concede a la sonda.
pub const LIMITE_DE_SONDEO_S: u64 = 60;
/// Imagen mínima que contiene el intérprete y `wget`.
pub const IMAGEN_DE_SONDA_POR_OMISION: &str = "alpine:3";

/// Nombres Docker derivados de la identidad de la célula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NombresDeCelula {
    /// Nombre del contenedor del núcleo.
    pub nucleo: String,
    /// Nombre del contenedor del sidecar.
    pub sidecar: String,
}

impl NombresDeCelula {
    /// Construye los nombres fijados por la plantilla de célula.
    pub fn nueva(id: &str) -> Self {
        Self {
            nucleo: format!("{id}-nucleo"),
            sidecar: format!("{id}-sidecar"),
        }
    }
}

/// Configuración opcional de la sonda de disponibilidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosDeSondeo {
    /// Imagen que hospedará la sonda hermana.
    pub imagen: String,
    /// Límite de espera expresado en segundos.
    pub limite_segundos: u64,
}

impl Default for DatosDeSondeo {
    fn default() -> Self {
        Self {
            imagen: std::env::var("HEXCELL_IMAGEN_SONDA")
                .unwrap_or_else(|_| IMAGEN_DE_SONDA_POR_OMISION.to_string()),
            limite_segundos: LIMITE_DE_SONDEO_S,
        }
    }
}

/// Fallo de una operación del ciclo de vida.
#[derive(Debug)]
pub enum ErrorDeCicloDeVida {
    /// Fallo devuelto por Docker.
    Docker(ErrorDeClienteDocker),
    /// La inspección no contiene la configuración necesaria.
    Configuracion(String),
    /// La sonda no confirmó disponibilidad a tiempo.
    TiempoDeSondeoAgotado { limite_segundos: u64 },
    /// La imagen auxiliar no está disponible en el demonio.
    ImagenDeSondaNoEncontrada { imagen: String },
}

impl fmt::Display for ErrorDeCicloDeVida {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Docker(error) => write!(f, "fallo de Docker: {error}"),
            Self::Configuracion(motivo) => {
                write!(f, "configuración de la célula inválida: {motivo}")
            }
            Self::TiempoDeSondeoAgotado { limite_segundos } => write!(
                f,
                "la célula no alcanzó /health/ready: se agotó el límite de {limite_segundos} segundos"
            ),
            Self::ImagenDeSondaNoEncontrada { imagen } => write!(
                f,
                "la imagen de sonda «{imagen}» no existe en Docker; hay que traerla antes de reanudar"
            ),
        }
    }
}

impl std::error::Error for ErrorDeCicloDeVida {}

impl From<ErrorDeClienteDocker> for ErrorDeCicloDeVida {
    fn from(error: ErrorDeClienteDocker) -> Self {
        Self::Docker(error)
    }
}

/// Detiene primero el sidecar y después el núcleo.
pub fn pausar(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
) -> Result<(), ErrorDeCicloDeVida> {
    cliente.detener_contenedor(&nombres.sidecar)?;
    cliente.detener_contenedor(&nombres.nucleo)?;
    Ok(())
}

/// Arranca la célula y espera la disponibilidad mediante un contenedor hermano.
pub fn reanudar(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeSondeo,
) -> Result<(), ErrorDeCicloDeVida> {
    cliente.iniciar_contenedor(&nombres.nucleo)?;
    cliente.iniciar_contenedor(&nombres.sidecar)?;

    let inspeccion = cliente.inspeccionar_contenedor(&nombres.nucleo)?;
    let red = inspeccion
        .pointer("/NetworkSettings/Networks")
        .and_then(serde_json::Value::as_object)
        .and_then(|redes| redes.keys().next())
        .cloned()
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion("el núcleo no declara una red".to_string())
        })?;
    let direccion = inspeccion
        .pointer("/Config/Env")
        .and_then(serde_json::Value::as_array)
        .and_then(|variables| {
            variables.iter().find_map(|variable| {
                variable
                    .as_str()?
                    .strip_prefix("HEXCELL_DIRECCION_SALUD=")
                    .map(str::to_string)
            })
        })
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion(
                "falta HEXCELL_DIRECCION_SALUD en el núcleo".to_string(),
            )
        })?;
    let puerto = direccion
        .rsplit_once(':')
        .map(|(_, puerto)| puerto)
        .filter(|puerto| !puerto.is_empty())
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion(
                "HEXCELL_DIRECCION_SALUD no contiene un puerto".to_string(),
            )
        })?;
    let url = format!("http://{}:{}/health/ready", nombres.nucleo, puerto);
    let opciones = OpcionesDeContenedor {
        red,
        cmd: guion_de_sonda_con_limite(&url, datos.limite_segundos),
    };
    let sonda = match cliente.crear_e_iniciar_contenedor_con_opciones(&datos.imagen, opciones) {
        Ok(resultado) => match resultado {
            crate::docker::ResultadoDeArranque::Iniciado { id_contenedor }
            | crate::docker::ResultadoDeArranque::YaEnEjecucion { id_contenedor } => id_contenedor,
        },
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada {
                imagen: datos.imagen.clone(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let espera = cliente.esperar_contenedor(&sonda);
    let limpieza = cliente.eliminar_contenedor(&sonda);
    let codigo = match (espera, limpieza) {
        (Ok(codigo), Ok(())) => codigo,
        (Err(error), Ok(())) => return Err(error.into()),
        (Ok(_), Err(error)) => return Err(error.into()),
        (Err(error), Err(_)) => return Err(error.into()),
    };
    if codigo == 0 {
        Ok(())
    } else {
        Err(ErrorDeCicloDeVida::TiempoDeSondeoAgotado {
            limite_segundos: datos.limite_segundos,
        })
    }
}

/// Produce el comando que ejecuta la sonda dentro de la red de la célula.
pub fn guion_de_sonda(url: &str) -> Vec<String> {
    guion_de_sonda_con_limite(url, LIMITE_DE_SONDEO_S)
}

fn guion_de_sonda_con_limite(url: &str, limite_segundos: u64) -> Vec<String> {
    let intentos = limite_segundos.saturating_mul(1000 / CADENCIA_DE_SONDEO_MS);
    let guion = format!(
        "i=0; while [ \"$i\" -lt {intentos} ]; do if wget -q -O /dev/null \"{url}\"; then exit 0; fi; i=$((i+1)); sleep 0.1; done; exit 1"
    );
    vec!["/bin/sh".to_string(), "-c".to_string(), guion]
}
