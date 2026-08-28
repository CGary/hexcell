//! Adaptador de incrustaciones HTTPS para Google AI Studio (Gemini).
//!
//! Implementa [`ProveedorDeEmbeddings`] conectando con la API de Gemini
//! mediante peticiones HTTPS salientes. La construcción del conector TLS/HTTP se duplica
//! para aislar los tipos de serialización.

use std::fmt;
use std::time::Duration;

use hexcell_core::embeddings::{
    PeticionDeEmbeddings, ProveedorDeEmbeddings, RespuestaDeEmbeddings, VectorDeEmbedding,
};
use serde::{Deserialize, Serialize};

/// Configuración de conexión para el proveedor de incrustaciones Google AI Studio (Gemini).
#[derive(Clone)]
pub struct ConfiguracionDeEmbeddingsGemini {
    /// URL base del servicio, p. ej. `https://generativelanguage.googleapis.com` o `http://127.0.0.1:8080`.
    pub url_base: String,
    /// Clave de autenticación de la API.
    pub api_key: String,
    /// Identificador del modelo, p. ej. `text-embedding-004`.
    pub modelo: String,
    /// Tiempo máximo acotado por cada intento de petición.
    pub timeout: Duration,
    /// Cantidad máxima de reintentos ante errores transitorios o 5xx.
    pub reintentos: u32,
    /// Tamaño máximo del lote de fragmentos por petición.
    pub tamano_de_lote: usize,
}

impl fmt::Debug for ConfiguracionDeEmbeddingsGemini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguracionDeEmbeddingsGemini")
            .field("url_base", &self.url_base)
            .field("api_key", &"«redactado»")
            .field("modelo", &self.modelo)
            .field("timeout", &self.timeout)
            .field("reintentos", &self.reintentos)
            .field("tamano_de_lote", &self.tamano_de_lote)
            .finish()
    }
}

/// Avería del proveedor de incrustaciones Gemini: fallos de transporte, rechazos HTTP o cuerpo malformado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeProveedorDeEmbeddingsGemini {
    /// La petición HTTP falló por error de transporte o red.
    ErrorDeTransporte(String),
    /// La petición superó el tiempo máximo acotado sin recibir respuesta completa.
    TiempoAgotado,
    /// El servidor devolvió un código de estado HTTP no exitoso (p. ej. 429 o 500).
    CodigoDeEstadoHttp {
        /// Código de estado HTTP devuelto por el servidor.
        codigo: u16,
        /// Detalle textual devuelto por el servidor.
        detalle: String,
    },
    /// El cuerpo de la respuesta no se pudo interpretar o contiene datos inválidos.
    RespuestaInvalida(String),
}

impl fmt::Display for ErrorDeProveedorDeEmbeddingsGemini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ErrorDeTransporte(err) => write!(f, "error de transporte HTTP: {err}"),
            Self::TiempoAgotado => {
                write!(
                    f,
                    "tiempo de espera agotado al invocar al proveedor de embeddings de Gemini"
                )
            }
            Self::CodigoDeEstadoHttp { codigo, detalle } => {
                write!(
                    f,
                    "el proveedor de embeddings de Gemini devolvió el código HTTP {codigo}: {detalle}"
                )
            }
            Self::RespuestaInvalida(motivo) => {
                write!(
                    f,
                    "respuesta inválida del proveedor de embeddings de Gemini: {motivo}"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDeProveedorDeEmbeddingsGemini {}

/// Proveedor de incrustaciones HTTPS para Google AI Studio (Gemini).
#[derive(Clone)]
pub struct ProveedorDeEmbeddingsGemini {
    url_base: String,
    api_key: String,
    modelo: String,
    timeout: Duration,
    reintentos: u32,
    tamano_de_lote: usize,
    cliente: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<bytes::Bytes>,
    >,
}

impl fmt::Debug for ProveedorDeEmbeddingsGemini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProveedorDeEmbeddingsGemini")
            .field("url_base", &self.url_base)
            .field("api_key", &"«redactado»")
            .field("modelo", &self.modelo)
            .field("timeout", &self.timeout)
            .field("reintentos", &self.reintentos)
            .field("tamano_de_lote", &self.tamano_de_lote)
            .finish()
    }
}

impl ProveedorDeEmbeddingsGemini {
    /// Devuelve el tamaño de lote configurado para este proveedor.
    /// Diseñado el 28 de agosto de 2026 para permitir la segmentación del lote en ingestas.
    pub fn tamano_de_lote(&self) -> usize {
        self.tamano_de_lote
    }

    /// Construye un nuevo adaptador de Gemini a partir de su configuración.
    pub fn nuevo(configuracion: ConfiguracionDeEmbeddingsGemini) -> Self {
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
            tamano_de_lote: configuracion.tamano_de_lote,
            cliente,
        }
    }

    /// Ejecuta un intento individual de petición HTTP POST hacia el servidor de Gemini.
    async fn ejecutar_un_intento(
        &self,
        peticion: &PeticionDeEmbeddings,
    ) -> Result<RespuestaDeEmbeddings, ErrorDeProveedorDeEmbeddingsGemini> {
        let requests: Vec<PeticionEmbeddingItemGemini> = peticion
            .textos
            .iter()
            .map(|t| PeticionEmbeddingItemGemini {
                model: if self.modelo.starts_with("models/") {
                    self.modelo.clone()
                } else {
                    format!("models/{}", self.modelo)
                },
                content: ContenidoGemini {
                    parts: vec![ParteGemini { text: t }],
                },
            })
            .collect();

        let body_struct = PeticionEmbeddingsGemini { requests };

        let body_json = serde_json::to_string(&body_struct)
            .map_err(|e| ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(e.to_string()))?;

        // Construir la URL del endpoint para batchEmbedContents
        let url_endpoint = format!(
            "{}/v1beta/models/{}:batchEmbedContents",
            self.url_base, self.modelo
        );
        let uri: hyper::Uri = url_endpoint
            .parse()
            .map_err(|e: hyper::http::uri::InvalidUri| {
                ErrorDeProveedorDeEmbeddingsGemini::ErrorDeTransporte(e.to_string())
            })?;

        // Enviar la clave de API exclusivamente en la cabecera x-goog-api-key
        let req = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri(uri)
            .header("x-goog-api-key", &self.api_key)
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(http_body_util::Full::new(bytes::Bytes::from(body_json)))
            .map_err(|e| ErrorDeProveedorDeEmbeddingsGemini::ErrorDeTransporte(e.to_string()))?;

        let res =
            self.cliente.request(req).await.map_err(|e| {
                ErrorDeProveedorDeEmbeddingsGemini::ErrorDeTransporte(e.to_string())
            })?;

        let estado = res.status();

        use http_body_util::BodyExt;
        let bytes_cuerpo = res
            .into_body()
            .collect()
            .await
            .map_err(|e| ErrorDeProveedorDeEmbeddingsGemini::ErrorDeTransporte(e.to_string()))?
            .to_bytes();

        if !estado.is_success() {
            let detalle = String::from_utf8_lossy(&bytes_cuerpo).to_string();
            return Err(ErrorDeProveedorDeEmbeddingsGemini::CodigoDeEstadoHttp {
                codigo: estado.as_u16(),
                detalle,
            });
        }

        let dto: RespuestaEmbeddingsGemini =
            serde_json::from_slice(&bytes_cuerpo).map_err(|e| {
                ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(format!(
                    "JSON malformado: {e}"
                ))
            })?;

        let embeddings = dto.embeddings.ok_or_else(|| {
            ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(
                "falta el campo embeddings".to_string(),
            )
        })?;

        if embeddings.len() != peticion.textos.len() {
            return Err(ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(
                "longitud de embeddings incorrecta".to_string(),
            ));
        }

        let mut vectores: Vec<Option<VectorDeEmbedding>> =
            Vec::with_capacity(peticion.textos.len());

        for item in embeddings {
            let values = item.values.ok_or_else(|| {
                ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(
                    "valores de embedding ausentes".to_string(),
                )
            })?;

            if values.is_empty() {
                return Err(ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(
                    "vector de embedding vacío".to_string(),
                ));
            }

            vectores.push(Some(VectorDeEmbedding::nuevo(values)));
        }

        let unidades_consumidas = match dto.usage_metadata {
            Some(uso) => uso.prompt_token_count.unwrap_or(0),
            None => 0,
        };

        Ok(RespuestaDeEmbeddings {
            vectores,
            unidades_consumidas,
        })
    }
}

impl ProveedorDeEmbeddings for ProveedorDeEmbeddingsGemini {
    type Error = ErrorDeProveedorDeEmbeddingsGemini;

    async fn incrustar_lote(
        &self,
        peticion: PeticionDeEmbeddings,
    ) -> Result<RespuestaDeEmbeddings, Self::Error> {
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
                    ErrorDeProveedorDeEmbeddingsGemini::CodigoDeEstadoHttp { codigo, .. } => {
                        if *codigo == 429 || (*codigo >= 400 && *codigo < 500) {
                            return Err(err.clone());
                        }
                        ultimo_error = Some(err.clone());
                    }
                    ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(_) => {
                        return Err(err.clone());
                    }
                    _ => {
                        ultimo_error = Some(err.clone());
                    }
                },
                Err(_) => {
                    ultimo_error = Some(ErrorDeProveedorDeEmbeddingsGemini::TiempoAgotado);
                }
            }
        }

        Err(ultimo_error.unwrap_or(ErrorDeProveedorDeEmbeddingsGemini::TiempoAgotado))
    }
}

#[derive(Serialize)]
struct PeticionEmbeddingsGemini<'a> {
    requests: Vec<PeticionEmbeddingItemGemini<'a>>,
}

#[derive(Serialize)]
struct PeticionEmbeddingItemGemini<'a> {
    model: String,
    content: ContenidoGemini<'a>,
}

#[derive(Serialize)]
struct ContenidoGemini<'a> {
    parts: Vec<ParteGemini<'a>>,
}

#[derive(Serialize)]
struct ParteGemini<'a> {
    text: &'a str,
}

#[derive(Deserialize)]
struct RespuestaEmbeddingsGemini {
    embeddings: Option<Vec<EmbeddingItemGemini>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsoDeEmbeddingsGemini>,
}

#[derive(Deserialize)]
struct EmbeddingItemGemini {
    values: Option<Vec<f32>>,
}

#[derive(Deserialize)]
struct UsoDeEmbeddingsGemini {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
}

#[cfg(test)]
mod pruebas_redaccion {
    use super::*;

    #[test]
    fn clave_de_api_no_aparece_en_debug_ni_en_errores() {
        let clave_sentinela = "CLAVE_SECRET_UNICA_EMBED_GEMINI_12345";
        let config = ConfiguracionDeEmbeddingsGemini {
            url_base: "http://127.0.0.1:8080".to_string(),
            api_key: clave_sentinela.to_string(),
            modelo: "modelo-embeddings-test".to_string(),
            timeout: Duration::from_secs(1),
            reintentos: 1,
            tamano_de_lote: 32,
        };

        let debug_config = format!("{config:?}");
        assert!(!debug_config.contains(clave_sentinela));
        assert!(debug_config.contains("«redactado»"));

        let proveedor = ProveedorDeEmbeddingsGemini::nuevo(config);
        let debug_proveedor = format!("{proveedor:?}");
        assert!(!debug_proveedor.contains(clave_sentinela));
        assert!(debug_proveedor.contains("«redactado»"));

        let error_transporte =
            ErrorDeProveedorDeEmbeddingsGemini::ErrorDeTransporte("fallo de red".to_string());
        assert!(!format!("{error_transporte:?}").contains(clave_sentinela));
        assert!(!format!("{error_transporte}").contains(clave_sentinela));

        let error_http = ErrorDeProveedorDeEmbeddingsGemini::CodigoDeEstadoHttp {
            codigo: 401,
            detalle: "No autorizado".to_string(),
        };
        assert!(!format!("{error_http:?}").contains(clave_sentinela));
        assert!(!format!("{error_http}").contains(clave_sentinela));
    }
}
