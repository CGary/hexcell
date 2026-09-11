//! Error único del cliente del socket Unix de Docker.
//!
//! Un solo enumerado para todo el módulo `docker`, y no un tipo por operación: quien consume el
//! cliente —la CLI de administración— reacciona ante un fallo del demonio de forma parecida sea
//! cual sea la operación, y multiplicar los tipos solo multiplicaría las conversiones sin cambiar
//! ninguna decisión.
//!
//! Cada variante nombra un modo de fallo **distinto**, nunca una cadena cruda ni un pánico: el
//! demonio inalcanzable, el permiso denegado, la respuesta que no se puede interpretar, el tiempo
//! de espera agotado, el recurso inexistente (404), el conflicto de estado (409) y el error del
//! propio demonio (5xx o cualquier otro código no previsto). Ningún camino de este módulo termina
//! en `panic`: `[profile.release]` fija `panic = "abort"` y un pánico en producción no deja ningún
//! mensaje utilizable.

use std::fmt;
use std::io;

/// Fallo del cliente del demonio de Docker.
#[derive(Debug)]
pub enum ErrorDeClienteDocker {
    /// El demonio no responde: el socket no existe en disco o la conexión fue rechazada.
    DemonioInalcanzable,
    /// El socket rechazó la conexión por permisos (`EACCES`): el proceso no puede hablar con él.
    PermisoDenegado,
    /// La respuesta del demonio no se pudo interpretar como HTTP/1.1 válido.
    RespuestaMalformada {
        /// Motivo legible, en español, de por qué la respuesta no se pudo interpretar.
        motivo: String,
    },
    /// La operación excedió el tiempo de espera acotado del cliente.
    TiempoDeEsperaAgotado,
    /// El recurso solicitado no existe en el demonio (404).
    NoEncontrado,
    /// El estado actual del recurso impide la operación (409): por ejemplo, eliminar un contenedor
    /// en ejecución sin forzarlo.
    Conflicto,
    /// El demonio respondió con un código de error (5xx u otro no previsto por el cliente).
    ErrorDelDaemon {
        /// Código de estado HTTP tal y como lo devolvió el demonio.
        estado: u16,
        /// Cuerpo de la respuesta del demonio, ya como texto (puede ser vacío).
        cuerpo: String,
    },
    /// Fallo de entrada/salida del socket que no se corresponde con ningún caso anterior.
    Io(io::Error),
}

impl fmt::Display for ErrorDeClienteDocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DemonioInalcanzable => {
                write!(f, "el demonio de Docker no responde por su socket Unix")
            }
            Self::PermisoDenegado => write!(
                f,
                "sin permiso para conectar con el socket Unix del demonio de Docker"
            ),
            Self::RespuestaMalformada { motivo } => {
                write!(f, "respuesta malformada del demonio de Docker: {motivo}")
            }
            Self::TiempoDeEsperaAgotado => {
                write!(f, "la operación agotó el tiempo de espera acotado")
            }
            Self::NoEncontrado => write!(f, "el recurso no existe en el demonio de Docker"),
            Self::Conflicto => write!(
                f,
                "el estado actual del recurso impide la operación en el demonio de Docker"
            ),
            Self::ErrorDelDaemon { estado, cuerpo } => {
                if cuerpo.is_empty() {
                    write!(f, "el demonio de Docker respondió con el estado {estado}")
                } else {
                    write!(
                        f,
                        "el demonio de Docker respondió con el estado {estado}: {cuerpo}"
                    )
                }
            }
            Self::Io(error) => write!(f, "error de E/S del socket del demonio de Docker: {error}"),
        }
    }
}

impl std::error::Error for ErrorDeClienteDocker {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
