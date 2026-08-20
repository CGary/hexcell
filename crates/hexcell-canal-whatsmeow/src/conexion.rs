//! Capa de transporte IPC: conexión al socket Unix del sidecar.
//!
//! `Conexion` encapsula el ciclo dial → saludo → lectura con búfer acotado → cierre, siguiendo
//! la topología real donde el **sidecar escucha** y el **núcleo conecta** (sección 2 del
//! protocolo, nunca al revés).
//!
//! El lector rechaza toda línea que supere [`crate::mensajes::LIMITE_DE_LINEA`] en lugar de
//! crecer sin techo: es una propiedad de seguridad de memoria en un proceso presupuestado en
//! 80 MB (NFR-01), no una formalidad.
//!
//! Ante cualquier error de protocolo (sección 8), la conexión se cierra y se registra el tipo
//! de error y, como mucho, el nombre del campo ofensor; **nunca la línea recibida**, que podría
//! contener texto de mensaje (`adr-0019`).

use std::path::Path;

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::error::ErrorCanalWhatsmeow;
use crate::mensajes::{
    LIMITE_DE_LINEA, MensajeEntrante, Saludo, VERSION_PROTOCOLO, analizar_mensaje_entrante,
};

/// Conexión activa al socket Unix del sidecar.
///
/// Gestiona el enmarcado por salto de línea y el búfer acotado de lectura. No gestiona la
/// reconexión: esa responsabilidad es de [`crate::adaptador::AdaptadorWhatsmeow`], que recrea
/// una `Conexion` nueva en cada intento.
pub struct Conexion {
    /// Lector con búfer acotado sobre el socket.
    lector: BufReader<tokio::io::ReadHalf<UnixStream>>,
    /// Extremo de escritura del socket, compartido con el adaptador.
    escritor: Arc<tokio::sync::Mutex<Option<tokio::io::WriteHalf<UnixStream>>>>,
}

impl Conexion {
    /// Conecta al socket Unix en la ruta dada.
    ///
    /// Solo intenta una vez; el reintento con retroceso es responsabilidad del adaptador.
    pub async fn conectar(
        ruta: &Path,
        escritor_compartido: Arc<tokio::sync::Mutex<Option<tokio::io::WriteHalf<UnixStream>>>>,
    ) -> Result<Self, ErrorCanalWhatsmeow> {
        let flujo = UnixStream::connect(ruta).await?;
        let (lectura, escritura) = tokio::io::split(flujo);
        {
            let mut lock = escritor_compartido.lock().await;
            *lock = Some(escritura);
        }
        Ok(Self {
            lector: BufReader::new(lectura),
            escritor: escritor_compartido,
        })
    }

    /// Ejecuta el saludo de versión completo (sección 3): envía el saludo propio y lee el del
    /// sidecar. Si la versión no coincide, cierra la conexión y devuelve
    /// [`ErrorCanalWhatsmeow::DesajusteDeVersion`] con las dos versiones.
    pub async fn saludar(&mut self, id_celula: &str) -> Result<Saludo, ErrorCanalWhatsmeow> {
        // El núcleo envía su saludo PRIMERO, antes que cualquier otra cosa (sección 3).
        let saludo_propio = Saludo {
            version: VERSION_PROTOCOLO,
            tipo: "saludo".to_string(),
            emisor: "nucleo".to_string(),
            id_celula: id_celula.to_string(),
        };
        self.escribir_linea(&serde_json::to_string(&saludo_propio).map_err(|e| {
            ErrorCanalWhatsmeow::ErrorDeProtocolo(format!(
                "no se pudo serializar el saludo propio: {e}"
            ))
        })?)
        .await?;

        // Lee el saludo del sidecar.
        let linea = self.leer_linea().await?;
        let mensaje = analizar_mensaje_entrante(&linea).map_err(|detalle| {
            ErrorCanalWhatsmeow::ErrorDeProtocolo(format!("saludo del sidecar inválido: {detalle}"))
        })?;

        match mensaje {
            MensajeEntrante::Saludo(saludo) => {
                if saludo.version != VERSION_PROTOCOLO {
                    return Err(ErrorCanalWhatsmeow::DesajusteDeVersion {
                        propia: VERSION_PROTOCOLO,
                        remota: saludo.version,
                    });
                }
                Ok(saludo)
            }
            _ => Err(ErrorCanalWhatsmeow::ErrorDeProtocolo(
                "el primer mensaje del sidecar debe ser un saludo".to_string(),
            )),
        }
    }

    /// Lee la siguiente línea del socket, respetando el límite de 131 072 bytes.
    ///
    /// **No usa `read_line` desnudo**, que crece sin límite: lee byte a byte dentro de un búfer
    /// acotado y rechaza cualquier línea que lo supere.
    pub async fn leer_linea(&mut self) -> Result<String, ErrorCanalWhatsmeow> {
        let mut bufer = Vec::with_capacity(4096);

        loop {
            let byte = match self.lector.read_u8().await {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(ErrorCanalWhatsmeow::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "el sidecar cerró la conexión",
                    )));
                }
                Err(e) => return Err(ErrorCanalWhatsmeow::Io(e)),
            };

            if byte == b'\n' {
                break;
            }

            bufer.push(byte);

            // El límite incluye el salto de línea final, así que el contenido puede tener
            // hasta LIMITE_DE_LINEA - 1 bytes antes del '\n'.
            if bufer.len() >= LIMITE_DE_LINEA {
                return Err(ErrorCanalWhatsmeow::LineaDemasiadoLarga);
            }
        }

        String::from_utf8(bufer).map_err(|_| {
            ErrorCanalWhatsmeow::ErrorDeProtocolo("la línea no es UTF-8 válido".to_string())
        })
    }

    /// Lee y analiza el siguiente mensaje entrante del sidecar.
    pub async fn leer_mensaje(&mut self) -> Result<MensajeEntrante, ErrorCanalWhatsmeow> {
        let linea = self.leer_linea().await?;
        analizar_mensaje_entrante(&linea).map_err(ErrorCanalWhatsmeow::ErrorDeProtocolo)
    }

    /// Escribe una línea de texto al socket, terminada en `\n`.
    pub async fn escribir_linea(&mut self, linea: &str) -> Result<(), ErrorCanalWhatsmeow> {
        let mut guardia = self.escritor.lock().await;
        if let Some(escritor) = guardia.as_mut() {
            escritor.write_all(linea.as_bytes()).await?;
            escritor.write_all(b"\n").await?;
            escritor.flush().await?;
            Ok(())
        } else {
            Err(ErrorCanalWhatsmeow::SinConexion)
        }
    }

    /// Envía una confirmación de entrega para el evento con el identificador de deduplicación
    /// dado.
    ///
    /// La confirmación lleva el **identificador de deduplicación** del evento, nunca un número
    /// de secuencia por conexión (sección 4 del protocolo, prohibición explícita).
    pub async fn confirmar(&mut self, id_deduplicacion: &str) -> Result<(), ErrorCanalWhatsmeow> {
        let confirmacion = crate::mensajes::Confirmacion {
            version: VERSION_PROTOCOLO,
            tipo: "confirmacion".to_string(),
            id_deduplicacion: id_deduplicacion.to_string(),
        };
        let linea = serde_json::to_string(&confirmacion).map_err(|e| {
            ErrorCanalWhatsmeow::ErrorDeProtocolo(format!(
                "no se pudo serializar la confirmación: {e}"
            ))
        })?;
        self.escribir_linea(&linea).await
    }
}

/// Envía un mensaje saliente a través del extremo de escritura compartido.
pub async fn enviar_saliente(
    escritor_compartido: &tokio::sync::Mutex<Option<tokio::io::WriteHalf<UnixStream>>>,
    mensaje: &crate::mensajes::MensajeSalienteIpc,
) -> Result<(), ErrorCanalWhatsmeow> {
    let linea = serde_json::to_string(mensaje).map_err(|e| {
        ErrorCanalWhatsmeow::ErrorDeProtocolo(format!(
            "no se pudo serializar mensaje_saliente: {e}"
        ))
    })?;
    let mut guardia = escritor_compartido.lock().await;
    if let Some(escritor) = guardia.as_mut() {
        escritor.write_all(linea.as_bytes()).await?;
        escritor.write_all(b"\n").await?;
        escritor.flush().await?;
        Ok(())
    } else {
        Err(ErrorCanalWhatsmeow::SinConexion)
    }
}

/// Envía una orden de respaldo del sqlstore a través del extremo de escritura compartido.
pub async fn enviar_orden_respaldo_sqlstore(
    escritor_compartido: &tokio::sync::Mutex<Option<tokio::io::WriteHalf<UnixStream>>>,
    orden: &crate::mensajes::OrdenRespaldoSqlstore,
) -> Result<(), ErrorCanalWhatsmeow> {
    let linea = serde_json::to_string(orden).map_err(|e| {
        ErrorCanalWhatsmeow::ErrorDeProtocolo(format!(
            "no se pudo serializar orden_respaldo_sqlstore: {e}"
        ))
    })?;
    let mut guardia = escritor_compartido.lock().await;
    if let Some(escritor) = guardia.as_mut() {
        escritor.write_all(linea.as_bytes()).await?;
        escritor.write_all(b"\n").await?;
        escritor.flush().await?;
        Ok(())
    } else {
        Err(ErrorCanalWhatsmeow::SinConexion)
    }
}

/// Envía una orden de respaldo del almacén de identidad a través del extremo de escritura compartido.
pub async fn enviar_orden_respaldo_identidad(
    escritor_compartido: &tokio::sync::Mutex<Option<tokio::io::WriteHalf<UnixStream>>>,
    orden: &crate::mensajes::OrdenRespaldoIdentidad,
) -> Result<(), ErrorCanalWhatsmeow> {
    let linea = serde_json::to_string(orden).map_err(|e| {
        ErrorCanalWhatsmeow::ErrorDeProtocolo(format!(
            "no se pudo serializar orden_respaldo_identidad: {e}"
        ))
    })?;
    let mut guardia = escritor_compartido.lock().await;
    if let Some(escritor) = guardia.as_mut() {
        escritor.write_all(linea.as_bytes()).await?;
        escritor.write_all(b"\n").await?;
        escritor.flush().await?;
        Ok(())
    } else {
        Err(ErrorCanalWhatsmeow::SinConexion)
    }
}

/// Envía una orden de emparejar a través del extremo de escritura compartido.
pub async fn enviar_orden_emparejar(
    escritor_compartido: &tokio::sync::Mutex<Option<tokio::io::WriteHalf<UnixStream>>>,
    orden: &crate::mensajes::OrdenEmparejar,
) -> Result<(), ErrorCanalWhatsmeow> {
    let linea = serde_json::to_string(orden).map_err(|e| {
        ErrorCanalWhatsmeow::ErrorDeProtocolo(format!("no se pudo serializar orden_emparejar: {e}"))
    })?;
    let mut guardia = escritor_compartido.lock().await;
    if let Some(escritor) = guardia.as_mut() {
        escritor.write_all(linea.as_bytes()).await?;
        escritor.write_all(b"\n").await?;
        escritor.flush().await?;
        Ok(())
    } else {
        Err(ErrorCanalWhatsmeow::SinConexion)
    }
}
