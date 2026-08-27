//! Adaptador de inferencia HTTPS compatible con la API de OpenAI (chat-completions).
//!
//! Implementa [`ProveedorDeInferencia`] conectando con endpoints externos (OpenRouter,
//! Google AI Studio, DeepSeek V4-Flash) mediante peticiones HTTPS salientes. Toda la
//! parametrización (URL base, clave de API, modelo, tiempo de espera y reintentos) se
//! gobierna por configuración sin ramificaciones en código.

use std::fmt;
use std::time::Duration;

use hexcell_core::inferencia::{
    PeticionDeInferencia, ProveedorDeInferencia, RespuestaDeInferencia,
};
use serde::{Deserialize, Serialize};

/// Configuración de conexión para el proveedor de inferencia OpenAI.
#[derive(Clone)]
pub struct ConfiguracionDeInferencia {
    /// URL base del servicio, p. ej. `https://openrouter.ai/api/v1` o `http://127.0.0.1:8080`.
    pub url_base: String,
    /// Clave de autenticación de la API.
    pub api_key: String,
    /// Identificador del modelo, p. ej. `deepseek/deepseek-chat`.
    pub modelo: String,
    /// Tiempo máximo acotado por cada intento de petición.
    pub timeout: Duration,
    /// Cantidad máxima de reintentos ante errores transitorios o 5xx.
    pub reintentos: u32,
}

impl fmt::Debug for ConfiguracionDeInferencia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguracionDeInferencia")
            .field("url_base", &self.url_base)
            .field("api_key", &"«redactado»")
            .field("modelo", &self.modelo)
            .field("timeout", &self.timeout)
            .field("reintentos", &self.reintentos)
            .finish()
    }
}

/// Avería del proveedor OpenAI: averías de transporte, rechazos HTTP o cuerpo malformado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeProveedorOpenAi {
    /// La petición HTTP falló por error de transporte o red.
    ErrorDeTransporte(String),
    /// La petición superó el tiempo máximo acotado sin recibir respuesta completa.
    TiempoAgotado,
    /// El servidor devolvió un código de estado HTTP no exitoso (p. ej. 429 o 500).
    CodigoDeEstadoHttp {
        /// Código de estado HTTP devuelto por el servidor.
        codigo: u16,
        /// Cuerpo o detalle textual devuelto por el servidor.
        detalle: String,
    },
    /// El cuerpo de la respuesta no se pudo interpretar o no contiene metadatos de uso válidos.
    RespuestaInvalida(String),
}

impl fmt::Display for ErrorDeProveedorOpenAi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ErrorDeTransporte(err) => write!(f, "error de transporte HTTP: {err}"),
            Self::TiempoAgotado => write!(f, "tiempo de espera agotado al invocar al proveedor"),
            Self::CodigoDeEstadoHttp { codigo, detalle } => {
                write!(
                    f,
                    "el proveedor devolvió el código HTTP {codigo}: {detalle}"
                )
            }
            Self::RespuestaInvalida(motivo) => {
                write!(f, "respuesta inválida del proveedor: {motivo}")
            }
        }
    }
}

impl std::error::Error for ErrorDeProveedorOpenAi {}

/// Proveedor de inferencia HTTPS sobre la API chat-completions compatible con OpenAI.
#[derive(Clone)]
pub struct ProveedorOpenAi {
    url_base: String,
    api_key: String,
    modelo: String,
    timeout: Duration,
    reintentos: u32,
    cliente: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<bytes::Bytes>,
    >,
}

impl fmt::Debug for ProveedorOpenAi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProveedorOpenAi")
            .field("url_base", &self.url_base)
            .field("api_key", &"«redactado»")
            .field("modelo", &self.modelo)
            .field("timeout", &self.timeout)
            .field("reintentos", &self.reintentos)
            .finish()
    }
}

impl ProveedorOpenAi {
    /// Construye un nuevo proveedor OpenAI a partir de su configuración.
    pub fn nuevo(configuracion: ConfiguracionDeInferencia) -> Self {
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

        let url_base = configuracion.url_base.trim_end_matches('/').to_string();

        Self {
            url_base,
            api_key: configuracion.api_key,
            modelo: configuracion.modelo,
            timeout: configuracion.timeout,
            reintentos: configuracion.reintentos,
            cliente,
        }
    }

    /// Ejecuta un intento individual de petición HTTP POST hacia el servidor.
    async fn ejecutar_un_intento(
        &self,
        peticion: &PeticionDeInferencia,
    ) -> Result<RespuestaDeInferencia, ErrorDeProveedorOpenAi> {
        let body_struct = PeticionChatOpenAi {
            model: &self.modelo,
            messages: vec![MensajeChatOpenAi {
                role: "user",
                content: &peticion.contenido,
            }],
        };

        let body_json = serde_json::to_string(&body_struct)
            .map_err(|e| ErrorDeProveedorOpenAi::RespuestaInvalida(e.to_string()))?;

        let url_endpoint = format!("{}/chat/completions", self.url_base);
        let uri: hyper::Uri = url_endpoint
            .parse()
            .map_err(|e: hyper::http::uri::InvalidUri| {
                ErrorDeProveedorOpenAi::ErrorDeTransporte(e.to_string())
            })?;

        let req = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri(uri)
            .header(
                hyper::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key),
            )
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(body_json)))
            .map_err(|e| ErrorDeProveedorOpenAi::ErrorDeTransporte(e.to_string()))?;

        let res = self
            .cliente
            .request(req)
            .await
            .map_err(|e| ErrorDeProveedorOpenAi::ErrorDeTransporte(e.to_string()))?;

        let estado = res.status();

        use http_body_util::BodyExt;
        let bytes_cuerpo = res
            .into_body()
            .collect()
            .await
            .map_err(|e| ErrorDeProveedorOpenAi::ErrorDeTransporte(e.to_string()))?
            .to_bytes();

        if !estado.is_success() {
            let detalle = String::from_utf8_lossy(&bytes_cuerpo).to_string();
            return Err(ErrorDeProveedorOpenAi::CodigoDeEstadoHttp {
                codigo: estado.as_u16(),
                detalle,
            });
        }

        let dto: RespuestaChatOpenAi = serde_json::from_slice(&bytes_cuerpo).map_err(|e| {
            ErrorDeProveedorOpenAi::RespuestaInvalida(format!("JSON malformado: {e}"))
        })?;

        let choices = dto.choices.ok_or_else(|| {
            ErrorDeProveedorOpenAi::RespuestaInvalida("falta el campo choices".to_string())
        })?;

        if choices.is_empty() {
            return Err(ErrorDeProveedorOpenAi::RespuestaInvalida(
                "el arreglo choices está vacío".to_string(),
            ));
        }

        let contenido = choices[0]
            .message
            .as_ref()
            .and_then(|m| m.content.clone())
            .ok_or_else(|| {
                ErrorDeProveedorOpenAi::RespuestaInvalida(
                    "falta choices[0].message.content".to_string(),
                )
            })?;

        let usage = dto.usage.ok_or_else(|| {
            ErrorDeProveedorOpenAi::RespuestaInvalida("falta el campo usage".to_string())
        })?;

        let prompt_tokens = usage.prompt_tokens.ok_or_else(|| {
            ErrorDeProveedorOpenAi::RespuestaInvalida("falta usage.prompt_tokens".to_string())
        })?;

        let completion_tokens = usage.completion_tokens.ok_or_else(|| {
            ErrorDeProveedorOpenAi::RespuestaInvalida("falta usage.completion_tokens".to_string())
        })?;

        let unidades_consumidas = prompt_tokens.saturating_add(completion_tokens);

        Ok(RespuestaDeInferencia {
            contenido,
            unidades_consumidas,
        })
    }
}

impl ProveedorDeInferencia for ProveedorOpenAi {
    type Error = ErrorDeProveedorOpenAi;

    async fn generar(
        &self,
        peticion: PeticionDeInferencia,
    ) -> Result<RespuestaDeInferencia, Self::Error> {
        let total_intentos = 1 + self.reintentos;
        let mut ultimo_error = None;

        for intento in 0..total_intentos {
            if intento > 0 {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }

            let resultado =
                tokio::time::timeout(self.timeout, self.ejecutar_un_intento(&peticion)).await;

            match resultado {
                Ok(Ok(respuesta)) => return Ok(respuesta),
                Ok(Err(err)) => match &err {
                    ErrorDeProveedorOpenAi::CodigoDeEstadoHttp { codigo, .. } => {
                        if *codigo == 429 || (*codigo >= 400 && *codigo < 500) {
                            return Err(err);
                        }
                        ultimo_error = Some(err);
                    }
                    ErrorDeProveedorOpenAi::RespuestaInvalida(_) => {
                        return Err(err);
                    }
                    _ => {
                        ultimo_error = Some(err);
                    }
                },
                Err(_) => {
                    ultimo_error = Some(ErrorDeProveedorOpenAi::TiempoAgotado);
                }
            }
        }

        Err(ultimo_error.unwrap_or(ErrorDeProveedorOpenAi::TiempoAgotado))
    }
}

#[derive(Serialize)]
struct PeticionChatOpenAi<'a> {
    model: &'a str,
    messages: Vec<MensajeChatOpenAi<'a>>,
}

#[derive(Serialize)]
struct MensajeChatOpenAi<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct RespuestaChatOpenAi {
    choices: Option<Vec<OpcionChatOpenAi>>,
    usage: Option<UsoTokensOpenAi>,
}

#[derive(Deserialize)]
struct OpcionChatOpenAi {
    message: Option<ContenidoMensajeOpenAi>,
}

#[derive(Deserialize)]
struct ContenidoMensajeOpenAi {
    content: Option<String>,
}

#[derive(Deserialize)]
struct UsoTokensOpenAi {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

#[cfg(test)]
mod pruebas_redaccion {
    use super::*;

    #[test]
    fn clave_de_api_no_aparece_en_debug_ni_en_errores() {
        let clave_sentinela = "CLAVE_SECRET_UNICA_12345";
        let config = ConfiguracionDeInferencia {
            url_base: "http://127.0.0.1:8080".to_string(),
            api_key: clave_sentinela.to_string(),
            modelo: "modelo-test".to_string(),
            timeout: Duration::from_secs(1),
            reintentos: 1,
        };

        let debug_config = format!("{config:?}");
        assert!(!debug_config.contains(clave_sentinela));
        assert!(debug_config.contains("«redactado»"));

        let proveedor = ProveedorOpenAi::nuevo(config);
        let debug_proveedor = format!("{proveedor:?}");
        assert!(!debug_proveedor.contains(clave_sentinela));
        assert!(debug_proveedor.contains("«redactado»"));

        let error_transporte = ErrorDeProveedorOpenAi::ErrorDeTransporte("fallo".to_string());
        assert!(!format!("{error_transporte:?}").contains(clave_sentinela));
        assert!(!format!("{error_transporte}").contains(clave_sentinela));

        let error_http = ErrorDeProveedorOpenAi::CodigoDeEstadoHttp {
            codigo: 401,
            detalle: "No autorizado".to_string(),
        };
        assert!(!format!("{error_http:?}").contains(clave_sentinela));
        assert!(!format!("{error_http}").contains(clave_sentinela));
    }
}
