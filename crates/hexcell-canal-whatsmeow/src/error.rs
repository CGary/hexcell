//! Averías de transporte del adaptador IPC de whatsmeow.
//!
//! Cada variante nombra un fallo del **transporte**, no un desenlace del dominio: los cuatro
//! rechazos de FR-12 viajan dentro de [`hexcell_core::canal::ResultadoEnvio`] y no aquí.

use std::fmt;

/// Avería del transporte IPC del adaptador de whatsmeow.
///
/// No es un resultado del dominio: lo que FR-12 enumera (ventana cerrada, plantilla requerida,
/// límite de tasa, destinatario inválido) viaja dentro de [`hexcell_core::canal::ResultadoEnvio`].
/// Esto son problemas del socket, del protocolo o de la conexión.
#[derive(Debug)]
pub enum ErrorCanalWhatsmeow {
    /// Error de entrada/salida del socket Unix.
    Io(std::io::Error),
    /// La versión del protocolo del sidecar no coincide con la del núcleo.
    DesajusteDeVersion {
        /// Versión que esperaba el núcleo.
        propia: i64,
        /// Versión que envió el sidecar.
        remota: i64,
    },
    /// La línea recibida viola una regla del protocolo: tipo desconocido, campo ausente, campo
    /// desconocido, valor que no es cadena ni entero, valor anidado o JSON inválido.
    ///
    /// El detalle nombra el **tipo de error**, no la línea recibida, que podría contener texto
    /// de mensaje (`adr-0019`).
    ErrorDeProtocolo(String),
    /// La línea recibida supera el límite de 131 072 bytes de la sección 1 del protocolo.
    LineaDemasiadoLarga,
    /// Se intentó enviar sin una conexión activa al sidecar.
    SinConexion,
    /// Se intentó enviar una plantilla, pero este transporte solo admite respuesta libre.
    PlantillaNoRepresentable,
    /// Se intentó enviar una respuesta a una conversación sin marca temporal de origen
    /// conocida: el adaptador nunca vio pasar un evento entrante de esa conversación por su
    /// bucle de lectura (por ejemplo, justo tras un reinicio del núcleo, ya que el mapa de
    /// marcas es memoria de proceso y se pierde con él). Se rechaza en vez de inventar una
    /// marca: un valor centinela de 0 (época Unix) se leería en el sidecar como "ya expirado"
    /// y descartaría el mensaje sin ningún intento real de envío, silenciosamente.
    OrigenDesconocido,
    /// No se recibió el acuse del respaldo del sqlstore dentro del plazo previsto, o la
    /// conexión terminó antes de recibir respuesta.
    RespaldoSinAcuse,
    /// No se recibió el acuse del emparejamiento dentro del plazo previsto, o la
    /// conexión terminó antes de recibir respuesta.
    EmparejamientoSinAcuse,
}

impl fmt::Display for ErrorCanalWhatsmeow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "error de E/S del socket IPC: {error}"),
            Self::DesajusteDeVersion { propia, remota } => write!(
                f,
                "desajuste de versión del protocolo IPC: propia={propia}, remota={remota}"
            ),
            Self::ErrorDeProtocolo(detalle) => {
                write!(f, "error de protocolo IPC: {detalle}")
            }
            Self::LineaDemasiadoLarga => write!(
                f,
                "la línea recibida supera el límite de 131072 bytes del protocolo IPC"
            ),
            Self::SinConexion => write!(f, "sin conexión activa al sidecar IPC"),
            Self::PlantillaNoRepresentable => write!(f, "el canal IPC no admite plantillas"),
            Self::OrigenDesconocido => write!(
                f,
                "sin marca temporal de origen conocida para esta conversación"
            ),
            Self::RespaldoSinAcuse => write!(
                f,
                "no se recibió acuse de respaldo del sqlstore dentro del plazo"
            ),
            Self::EmparejamientoSinAcuse => {
                write!(f, "no se recibió acuse de emparejamiento dentro del plazo")
            }
        }
    }
}

impl std::error::Error for ErrorCanalWhatsmeow {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ErrorCanalWhatsmeow {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
