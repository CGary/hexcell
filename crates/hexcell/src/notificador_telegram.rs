//! Adaptador de notificación HTTPS hacia la API `sendMessage` de Telegram.
//!
//! Implementa [`hexcell_core::notificacion::SumideroDeNotificaciones`] con una petición HTTPS
//! saliente, siguiendo el mismo cliente `hyper-util` + `hyper-rustls` que
//! `crate::proveedor_openai::ProveedorOpenAi` ya construye para el proveedor de inferencia
//! (`HEX-044`, `adr-0012`): no hace falta ninguna dependencia nueva. El token del bot llega solo
//! por variable de entorno (`crate::configuracion`, precedente `HEX-064`/`HEX-065`) y nunca
//! aparece en un `Debug`, en un `Display` ni en una línea de registro.

use std::fmt;
use std::time::Duration;

use hexcell_core::notificacion::{
    ComponenteDeCelula, Notificacion, SumideroDeNotificaciones, ValorDeDato,
};
use serde::Serialize;

/// Configuración de conexión del sumidero Telegram.
#[derive(Clone)]
pub struct ConfiguracionDeTelegram {
    /// URL base de la API de Telegram, p. ej. `https://api.telegram.org`.
    pub url_base: String,
    /// Token del bot, provisto solo por variable de entorno.
    pub token: String,
    /// Identificador del chat de destino de las notificaciones.
    pub id_chat: String,
    /// Tiempo máximo acotado por la petición.
    pub timeout: Duration,
}

impl fmt::Debug for ConfiguracionDeTelegram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguracionDeTelegram")
            .field("url_base", &self.url_base)
            .field("token", &"«redactado»")
            .field("id_chat", &self.id_chat)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Avería del sumidero Telegram: averías de transporte, rechazos HTTP o cuerpo malformado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeTelegram {
    /// La petición HTTP falló por error de transporte o red.
    ErrorDeTransporte(String),
    /// La petición superó el tiempo máximo acotado sin recibir respuesta completa.
    TiempoAgotado,
    /// El servidor devolvió un código de estado HTTP no exitoso (p. ej. 401 o 500).
    CodigoDeEstadoHttp {
        /// Código de estado HTTP devuelto por el servidor.
        codigo: u16,
        /// Cuerpo o detalle textual devuelto por el servidor.
        detalle: String,
    },
    /// El cuerpo de la petición no se pudo serializar.
    CuerpoInvalido(String),
}

impl fmt::Display for ErrorDeTelegram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ErrorDeTransporte(err) => write!(f, "error de transporte HTTP: {err}"),
            Self::TiempoAgotado => write!(f, "tiempo de espera agotado al invocar a Telegram"),
            Self::CodigoDeEstadoHttp { codigo, detalle } => {
                write!(f, "Telegram devolvió el código HTTP {codigo}: {detalle}")
            }
            Self::CuerpoInvalido(motivo) => write!(f, "cuerpo de notificación inválido: {motivo}"),
        }
    }
}

impl std::error::Error for ErrorDeTelegram {}

/// Sumidero de notificaciones que envía un mensaje de Telegram por cada notificación recibida.
#[derive(Clone)]
pub struct NotificadorTelegram {
    url_base: String,
    token: String,
    id_chat: String,
    timeout: Duration,
    cliente: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<bytes::Bytes>,
    >,
}

impl fmt::Debug for NotificadorTelegram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotificadorTelegram")
            .field("url_base", &self.url_base)
            .field("token", &"«redactado»")
            .field("id_chat", &self.id_chat)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl NotificadorTelegram {
    /// Construye un nuevo sumidero Telegram a partir de su configuración.
    pub fn nuevo(configuracion: ConfiguracionDeTelegram) -> Self {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let cfg = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("configuración de versiones TLS por defecto")
        .with_root_certificates(root_store)
        .with_no_client_auth();

        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(cfg)
            .https_or_http()
            .enable_http1()
            .build();

        let cliente =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(connector);

        Self {
            url_base: configuracion.url_base.trim_end_matches('/').to_string(),
            token: configuracion.token,
            id_chat: configuracion.id_chat,
            timeout: configuracion.timeout,
            cliente,
        }
    }
}

impl SumideroDeNotificaciones for NotificadorTelegram {
    type Error = ErrorDeTelegram;

    async fn notificar(&self, notificacion: Notificacion) -> Result<(), Self::Error> {
        let texto = formatear_texto(&notificacion);

        let cuerpo_struct = PeticionSendMessage {
            chat_id: &self.id_chat,
            text: &texto,
        };
        let cuerpo_json = serde_json::to_string(&cuerpo_struct)
            .map_err(|e| ErrorDeTelegram::CuerpoInvalido(e.to_string()))?;

        let url_endpoint = format!("{}/bot{}/sendMessage", self.url_base, self.token);
        let uri: hyper::Uri = url_endpoint
            .parse()
            .map_err(|e: hyper::http::uri::InvalidUri| {
                ErrorDeTelegram::ErrorDeTransporte(e.to_string())
            })?;

        let req = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri(uri)
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(cuerpo_json)))
            .map_err(|e| ErrorDeTelegram::ErrorDeTransporte(e.to_string()))?;

        let peticion = self.cliente.request(req);
        let res = tokio::time::timeout(self.timeout, peticion)
            .await
            .map_err(|_| ErrorDeTelegram::TiempoAgotado)?
            .map_err(|e| ErrorDeTelegram::ErrorDeTransporte(e.to_string()))?;

        let estado = res.status();

        use http_body_util::BodyExt;
        let bytes_cuerpo = res
            .into_body()
            .collect()
            .await
            .map_err(|e| ErrorDeTelegram::ErrorDeTransporte(e.to_string()))?
            .to_bytes();

        if !estado.is_success() {
            let detalle = String::from_utf8_lossy(&bytes_cuerpo).to_string();
            return Err(ErrorDeTelegram::CodigoDeEstadoHttp {
                codigo: estado.as_u16(),
                detalle,
            });
        }

        Ok(())
    }
}

#[derive(Serialize)]
struct PeticionSendMessage<'a> {
    chat_id: &'a str,
    text: &'a str,
}

/// Vuelca el código y los datos de la notificación en un texto legible para el chat de Telegram.
fn formatear_texto(notificacion: &Notificacion) -> String {
    let mut texto = format!("[{}]", notificacion.codigo.como_str());
    for dato in &notificacion.datos {
        let valor = match &dato.valor {
            ValorDeDato::Texto(t) => t.clone(),
            ValorDeDato::Instante(instante) => match instante.duration_since(std::time::UNIX_EPOCH)
            {
                Ok(d) => format!("{} s desde época", d.as_secs()),
                Err(_) => "instante anterior a la época".to_string(),
            },
            ValorDeDato::Conversacion(c) => c.como_str().to_string(),
            ValorDeDato::Componente(c) => match c {
                ComponenteDeCelula::Nucleo => "núcleo".to_string(),
                ComponenteDeCelula::Sidecar => "sidecar".to_string(),
            },
        };
        texto.push_str(&format!("\n{}: {valor}", dato.clave));
    }
    texto
}
