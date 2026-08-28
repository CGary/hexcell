//! Adaptador de incrustaciones HTTPS compatible con la API de OpenAI (/embeddings).
//!
//! Implementa [`ProveedorDeEmbeddings`] conectando con endpoints externos (OpenRouter)
//! mediante peticiones HTTPS salientes. La construcción del conector TLS/HTTP se duplica
//! deliberadamente desde `proveedor_openai.rs` para aislar los tipos de serialización y evitar
//! relajar la validación de tokens del flujo de chat (`adr-0025`).

use std::fmt;
use std::time::Duration;

use hexcell_core::embeddings::{
    PeticionDeEmbeddings, ProveedorDeEmbeddings, RespuestaDeEmbeddings, VectorDeEmbedding,
};
use serde::{Deserialize, Serialize};

/// Configuración de conexión para el proveedor de incrustaciones OpenRouter/OpenAI.
#[derive(Clone)]
pub struct ConfiguracionDeEmbeddings {
    /// URL base del servicio, p. ej. `https://openrouter.ai/api/v1` o `http://127.0.0.1:8080`.
    pub url_base: String,
    /// Clave de autenticación de la API.
    pub api_key: String,
    /// Identificador del modelo, p. ej. `text-embedding-3-small`.
    pub modelo: String,
    /// Tiempo máximo acotado por cada intento de petición.
    pub timeout: Duration,
    /// Cantidad máxima de reintentos ante errores transitorios o 5xx.
    pub reintentos: u32,
    /// Tamaño máximo del lote de fragmentos por petición.
    pub tamano_de_lote: usize,
}

impl fmt::Debug for ConfiguracionDeEmbeddings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguracionDeEmbeddings")
            .field("url_base", &self.url_base)
            .field("api_key", &"«redactado»")
            .field("modelo", &self.modelo)
            .field("timeout", &self.timeout)
            .field("reintentos", &self.reintentos)
            .field("tamano_de_lote", &self.tamano_de_lote)
            .finish()
    }
}

/// Avería del proveedor de incrustaciones: fallos de transporte, rechazos HTTP o cuerpo malformado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeProveedorDeEmbeddings {
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

impl fmt::Display for ErrorDeProveedorDeEmbeddings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ErrorDeTransporte(err) => write!(f, "error de transporte HTTP: {err}"),
            Self::TiempoAgotado => {
                write!(
                    f,
                    "tiempo de espera agotado al invocar al proveedor de embeddings"
                )
            }
            Self::CodigoDeEstadoHttp { codigo, detalle } => {
                write!(
                    f,
                    "el proveedor de embeddings devolvió el código HTTP {codigo}: {detalle}"
                )
            }
            Self::RespuestaInvalida(motivo) => {
                write!(
                    f,
                    "respuesta inválida del proveedor de embeddings: {motivo}"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDeProveedorDeEmbeddings {}

/// Proveedor de incrustaciones HTTPS sobre el endpoint `/embeddings` compatible con OpenAI.
#[derive(Clone)]
pub struct ProveedorDeEmbeddingsOpenRouter {
    url_base: String,
    api_key: String,
    modelo: String,
    timeout: Duration,
    reintentos: u32,
    #[allow(dead_code)]
    tamano_de_lote: usize,
    cliente: hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<bytes::Bytes>,
    >,
}

impl fmt::Debug for ProveedorDeEmbeddingsOpenRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProveedorDeEmbeddingsOpenRouter")
            .field("url_base", &self.url_base)
            .field("api_key", &"«redactado»")
            .field("modelo", &self.modelo)
            .field("timeout", &self.timeout)
            .field("reintentos", &self.reintentos)
            .field("tamano_de_lote", &self.tamano_de_lote)
            .finish()
    }
}

impl ProveedorDeEmbeddingsOpenRouter {
    /// Construye un nuevo adaptador OpenRouter a partir de su configuración.
    pub fn nuevo(configuracion: ConfiguracionDeEmbeddings) -> Self {
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

    /// Ejecuta un intento individual de petición HTTP POST hacia el servidor de embeddings.
    async fn ejecutar_un_intento(
        &self,
        peticion: &PeticionDeEmbeddings,
    ) -> Result<RespuestaDeEmbeddings, ErrorDeProveedorDeEmbeddings> {
        let body_struct = PeticionEmbeddingsOpenAi {
            model: &self.modelo,
            input: &peticion.textos,
            encoding_format: "float",
        };

        let body_json = serde_json::to_string(&body_struct)
            .map_err(|e| ErrorDeProveedorDeEmbeddings::RespuestaInvalida(e.to_string()))?;

        let url_endpoint = format!("{}/embeddings", self.url_base);
        let uri: hyper::Uri = url_endpoint
            .parse()
            .map_err(|e: hyper::http::uri::InvalidUri| {
                ErrorDeProveedorDeEmbeddings::ErrorDeTransporte(e.to_string())
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
            .map_err(|e| ErrorDeProveedorDeEmbeddings::ErrorDeTransporte(e.to_string()))?;

        let res = self
            .cliente
            .request(req)
            .await
            .map_err(|e| ErrorDeProveedorDeEmbeddings::ErrorDeTransporte(e.to_string()))?;

        let estado = res.status();

        use http_body_util::BodyExt;
        let bytes_cuerpo = res
            .into_body()
            .collect()
            .await
            .map_err(|e| ErrorDeProveedorDeEmbeddings::ErrorDeTransporte(e.to_string()))?
            .to_bytes();

        if !estado.is_success() {
            let detalle = String::from_utf8_lossy(&bytes_cuerpo).to_string();
            return Err(ErrorDeProveedorDeEmbeddings::CodigoDeEstadoHttp {
                codigo: estado.as_u16(),
                detalle,
            });
        }

        let dto: RespuestaEmbeddingsOpenAi =
            serde_json::from_slice(&bytes_cuerpo).map_err(|e| {
                ErrorDeProveedorDeEmbeddings::RespuestaInvalida(format!("JSON malformado: {e}"))
            })?;

        let data = dto.data.ok_or_else(|| {
            ErrorDeProveedorDeEmbeddings::RespuestaInvalida("falta el campo data".to_string())
        })?;

        let mut vectores: Vec<Option<VectorDeEmbedding>> = vec![None; peticion.textos.len()];

        for item in data {
            let idx = item.index.ok_or_else(|| {
                ErrorDeProveedorDeEmbeddings::RespuestaInvalida(
                    "falta el campo index en un elemento de data".to_string(),
                )
            })?;

            if idx >= peticion.textos.len() {
                return Err(ErrorDeProveedorDeEmbeddings::RespuestaInvalida(format!(
                    "índice {idx} fuera de rango para petición de longitud {}",
                    peticion.textos.len()
                )));
            }

            if vectores[idx].is_some() {
                return Err(ErrorDeProveedorDeEmbeddings::RespuestaInvalida(format!(
                    "índice {idx} duplicado en la respuesta"
                )));
            }

            let embedding = item.embedding.ok_or_else(|| {
                ErrorDeProveedorDeEmbeddings::RespuestaInvalida(
                    "falta el arreglo embedding en un elemento de data".to_string(),
                )
            })?;

            if embedding.is_empty() {
                return Err(ErrorDeProveedorDeEmbeddings::RespuestaInvalida(
                    "el vector de embedding tiene longitud cero".to_string(),
                ));
            }

            vectores[idx] = Some(VectorDeEmbedding::nuevo(embedding));
        }

        let unidades_consumidas = match dto.usage {
            Some(uso) => match uso.prompt_tokens {
                Some(prompt) => prompt.saturating_add(uso.completion_tokens.unwrap_or(0)),
                None => 0,
            },
            None => 0,
        };

        Ok(RespuestaDeEmbeddings {
            vectores,
            unidades_consumidas,
        })
    }
}

impl ProveedorDeEmbeddings for ProveedorDeEmbeddingsOpenRouter {
    type Error = ErrorDeProveedorDeEmbeddings;

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
                    ErrorDeProveedorDeEmbeddings::CodigoDeEstadoHttp { codigo, .. } => {
                        if *codigo == 429 || (*codigo >= 400 && *codigo < 500) {
                            return Err(err);
                        }
                        ultimo_error = Some(err);
                    }
                    ErrorDeProveedorDeEmbeddings::RespuestaInvalida(_) => {
                        return Err(err);
                    }
                    _ => {
                        ultimo_error = Some(err);
                    }
                },
                Err(_) => {
                    ultimo_error = Some(ErrorDeProveedorDeEmbeddings::TiempoAgotado);
                }
            }
        }

        Err(ultimo_error.unwrap_or(ErrorDeProveedorDeEmbeddings::TiempoAgotado))
    }
}

#[derive(Serialize)]
struct PeticionEmbeddingsOpenAi<'a> {
    model: &'a str,
    input: &'a [String],
    encoding_format: &'a str,
}

#[derive(Deserialize)]
struct RespuestaEmbeddingsOpenAi {
    data: Option<Vec<DatoEmbeddingOpenAi>>,
    usage: Option<UsoTokensEmbeddingsOpenAi>,
}

#[derive(Deserialize)]
struct DatoEmbeddingOpenAi {
    index: Option<usize>,
    embedding: Option<Vec<f32>>,
}

#[derive(Deserialize)]
struct UsoTokensEmbeddingsOpenAi {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

#[cfg(test)]
mod pruebas_redaccion {
    use super::*;

    #[test]
    fn clave_de_api_no_aparece_en_debug_ni_en_errores() {
        let clave_sentinela = "CLAVE_SECRET_UNICA_EMBED_12345";
        let config = ConfiguracionDeEmbeddings {
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

        let proveedor = ProveedorDeEmbeddingsOpenRouter::nuevo(config);
        let debug_proveedor = format!("{proveedor:?}");
        assert!(!debug_proveedor.contains(clave_sentinela));
        assert!(debug_proveedor.contains("«redactado»"));

        let error_transporte =
            ErrorDeProveedorDeEmbeddings::ErrorDeTransporte("fallo de red".to_string());
        assert!(!format!("{error_transporte:?}").contains(clave_sentinela));
        assert!(!format!("{error_transporte}").contains(clave_sentinela));

        let error_http = ErrorDeProveedorDeEmbeddings::CodigoDeEstadoHttp {
            codigo: 401,
            detalle: "No autorizado".to_string(),
        };
        assert!(!format!("{error_http:?}").contains(clave_sentinela));
        assert!(!format!("{error_http}").contains(clave_sentinela));
    }
}
