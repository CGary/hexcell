//! Servidor HTTP interno de administración: `POST /admin/ingesta` y `GET /admin/ingesta`.
//!
//! Expone una interfaz interna, accesible únicamente desde la red local o loopback, para desencadenar
//! la ingesta de conocimiento en segundo plano en la base en sombra (`knowledge_staging.db`) y
//! permitir a una CLI de administración consultar el estado del trabajo mediante sondeos (polling).
//!
//! Se ejecuta sobre su propio puerto (`HEXCELL_DIRECCION_ADMIN`), independiente del servidor de salud
//! (`HEXCELL_DIRECCION_SALUD`), para permitir separar la exposición de ambas superficies en etapas
//! futuras de empaquetado y seguridad.
//!
//! Ninguna clave de cerrojo cruza un `.await`: el estado del proceso de ingesta se gestiona en memoria
//! de forma síncrona en una única sección crítica (`std::sync::Mutex`), garantizando que como máximo
//! un trabajo de ingesta corra a la vez por célula.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::DocumentoDeIngesta;

use crate::embeddings::{ProveedorDeEmbeddingsDeCelula, ServicioDeEmbeddings};
use crate::ingesta::{DesenlaceDeIngesta, ResumenDeIngesta, ejecutar_ingesta};
use crate::salud::{EstadoDeSalud, servir_salud};

/// Tamaño por omisión del límite del cuerpo de las peticiones administrativas (1 MiB).
pub const LIMITE_DE_CUERPO_ADMIN_POR_DEFECTO: usize = 1024 * 1024;

/// Configuración de fragmentación por omisión para la ingesta desencadenada desde el endpoint administrativo.
pub const CONFIGURACION_DE_FRAGMENTACION_DE_INGESTA: ConfiguracionDeFragmentacion =
    ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 500,
        solapamiento: 50,
    };

/// Prefijo del motivo registrado cuando la tarea de ingesta muere sin devolver un resultado propio.
///
/// Un operador necesita distinguir un fallo *de la ingesta* (que `ejecutar_ingesta` describe con su
/// propio error de dominio) de la muerte anormal del hilo que la corría: el primero es un problema
/// del documento o del proveedor de incrustaciones; el segundo es un defecto del programa que hay
/// que reportar. Por eso el motivo lleva marca propia en vez de mimetizarse con un error de ingesta.
pub const MOTIVO_DE_TERMINACION_ANORMAL: &str =
    "terminación anormal de la tarea de ingesta (pánico)";

/// Texto de la sonda semántica utilizada para validar la calidad del modelo de incrustación.
pub const TEXTO_DE_LA_SONDA_POR_DEFECTO: &str = "sonda de prueba de conocimiento";

/// Umbral mínimo de aceptación de similitud para la sonda semántica.
pub const UMBRAL_DE_ACEPTACION_POR_DEFECTO: f32 = 0.5;

/// Cuerpo de respuesta HTTP devuelto por el servidor administrativo.
pub type CuerpoDeAdmin = Full<Bytes>;

/// Estado del proceso de ingesta administrativa en memoria.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaseDeIngesta {
    /// No hay ningún proceso de ingesta activo ni previo registrado en esta sesión del proceso.
    Inactiva,
    /// Se está ejecutando una ingesta en segundo plano.
    EnCurso,
    /// El último proceso de ingesta concluyó con éxito reportando un resumen contable.
    Finalizada { resumen: ResumenDeIngesta },
    /// El último proceso de ingesta falló con un motivo estructural.
    Fallida { motivo: String },
}

/// Servicio de aplicación que gestiona el estado en proceso de la ingesta de conocimiento.
///
/// Posee el único estado de trabajo de ingesta (compare-and-set atómico vía `Mutex`), garantizando
/// que como máximo un trabajo de ingesta se ejecute a la vez por célula.
pub struct EstadoDeAdmin {
    fase: Mutex<FaseDeIngesta>,
}

impl Default for EstadoDeAdmin {
    fn default() -> Self {
        Self {
            fase: Mutex::new(FaseDeIngesta::Inactiva),
        }
    }
}

impl EstadoDeAdmin {
    /// Construye un nuevo contenedor de estado administrativo.
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Compara y conmuta el estado a `EnCurso` en una única sección crítica.
    ///
    /// Devuelve `true` si el trabajo fue iniciado con éxito, o `false` si ya había una ingesta
    /// `EnCurso` (lo que da origen a la decisión 409 Conflict).
    pub fn intentar_iniciar(&self) -> bool {
        let mut guard = self.fase.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*guard, FaseDeIngesta::EnCurso) {
            false
        } else {
            *guard = FaseDeIngesta::EnCurso;
            true
        }
    }

    /// Transición terminal desde `EnCurso` hacia `Finalizada` o `Fallida`.
    pub fn registrar_desenlace(&self, resultado: Result<ResumenDeIngesta, String>) {
        let mut guard = self.fase.lock().unwrap_or_else(|e| e.into_inner());
        *guard = match resultado {
            Ok(resumen) => FaseDeIngesta::Finalizada { resumen },
            Err(motivo) => FaseDeIngesta::Fallida { motivo },
        };
    }

    /// Devuelve una instantánea clonada de la fase actual.
    pub fn fase_actual(&self) -> FaseDeIngesta {
        let guard = self.fase.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }
}

/// Vigila la tarea de ingesta y garantiza que su muerte anormal deje una fase **terminal**.
///
/// El diseño de un solo trabajo por célula falla cerrado: mientras la fase siga en `EnCurso`, todo
/// POST posterior recibe 409 y el GET describe un trabajo que ya no existe. Si la tarea muere sin
/// registrar su desenlace, ese cierre es permanente hasta reiniciar el proceso. Awaitar el
/// `JoinHandle` desde una tarea aparte cubre ese hueco sin `catch_unwind`, porque `tokio` ya
/// entrega la muerte anormal como `Err(JoinError)`.
///
/// **El alcance depende del perfil de compilación, y conviene no exagerarlo.** El perfil de
/// release de este workspace fija `panic = "abort"` (raíz `Cargo.toml`), así que allí un pánico
/// mata el proceso entero en el sitio: no hay desenrollado, `tokio` nunca produce `Err(JoinError)`
/// por pánico y la fase clavada no llega a existir, porque tampoco existe el proceso. La rama de
/// pánico de este vigilante es alcanzable bajo `panic = "unwind"`, que es el perfil de desarrollo
/// y de pruebas. Se conserva de todos modos por dos razones: cubre el aborto de la tarea que no
/// viene del apagado, y evita que el hueco reaparezca en silencio si algún día el perfil de
/// release vuelve a desenrollar.
///
/// La cancelación se ignora a propósito. La única forma en que esta tarea se cancela es que el
/// proceso esté bajando y el runtime suelte sus tareas; registrar «fallida» ahí sería inventar un
/// fallo que no ocurrió. Además, al bajar el proceso el propio vigilante se suelta con la tarea
/// vigilada, así que el apagado sigue sin esperar a nadie.
pub async fn supervisar_ingesta(
    estado: Arc<EstadoDeAdmin>,
    tarea: tokio::task::JoinHandle<Result<ResumenDeIngesta, String>>,
) {
    match tarea.await {
        Ok(resultado) => estado.registrar_desenlace(resultado),
        Err(error) if error.is_cancelled() => {}
        Err(error) => {
            estado.registrar_desenlace(Err(format!("{MOTIVO_DE_TERMINACION_ANORMAL}: {error}")));
        }
    }
}

/// Rutas soportadas por el endpoint de administración.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RutaAdmin {
    /// Petición `POST /admin/ingesta` para iniciar ingesta en segundo plano.
    DispararIngesta,
    /// Petición `GET /admin/ingesta` para consultar la fase actual del trabajo.
    ConsultarEstado,
    /// Ruta o método no reconocido.
    NoEncontrada,
}

/// Enruta puramente una petición a partir del método HTTP y la ruta de la URI.
pub fn enrutar_admin(metodo: &Method, ruta: &str) -> RutaAdmin {
    match (metodo, ruta) {
        (&Method::POST, "/admin/ingesta") => RutaAdmin::DispararIngesta,
        (&Method::GET, "/admin/ingesta") => RutaAdmin::ConsultarEstado,
        _ => RutaAdmin::NoEncontrada,
    }
}

/// DTO de entrada para deserializar el cuerpo JSON del POST de ingesta.
///
/// Aísla el modelo de persistencia `DocumentoDeIngesta` de decoraciones de transporte.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct DocumentoEntrante {
    pub referencia_externa: String,
    pub titulo: String,
    pub contenido: String,
    pub actualizado_ms: Option<i64>,
}

impl DocumentoEntrante {
    /// Convierte el DTO de transporte a la entidad de persistencia.
    pub fn en_documento_de_ingesta(self) -> DocumentoDeIngesta {
        let actualizado_ms = self
            .actualizado_ms
            .unwrap_or_else(|| hexcell_storage::a_milisegundos(std::time::SystemTime::now()));
        DocumentoDeIngesta {
            referencia_externa: self.referencia_externa,
            titulo: self.titulo,
            contenido: self.contenido,
            actualizado_ms,
        }
    }
}

/// Construye una respuesta HTTP con el cuerpo JSON representativo de la fase.
pub fn respuesta_de_fase(fase: &FaseDeIngesta) -> Response<CuerpoDeAdmin> {
    let documento = match fase {
        FaseDeIngesta::Inactiva => serde_json::json!({ "estado": "inactiva" }),
        FaseDeIngesta::EnCurso => serde_json::json!({ "estado": "en_curso" }),
        FaseDeIngesta::Finalizada { resumen } => {
            let desenlace_reportado = match resumen.desenlace {
                DesenlaceDeIngesta::Completa => "completa",
                DesenlaceDeIngesta::Parcial => "parcial",
                DesenlaceDeIngesta::DetenidaPorApagado => "detenida_por_apagado",
                DesenlaceDeIngesta::SinIncrustaciones => "sin_incrustaciones",
            };
            serde_json::json!({
                "estado": "finalizada",
                "resumen": {
                    "fragmentos_solicitados": resumen.fragmentos_solicitados,
                    "fragmentos_escritos": resumen.fragmentos_escritos,
                    "lotes_emitidos": resumen.lotes_emitidos,
                    "dimension_observada": resumen.dimension_observada,
                    "dimension_de_la_sonda": resumen.dimension_de_la_sonda,
                    "desenlace": desenlace_reportado,
                }
            })
        }
        FaseDeIngesta::Fallida { motivo } => serde_json::json!({
            "estado": "fallida",
            "motivo": motivo,
        }),
    };

    let mut respuesta = Response::new(Full::new(Bytes::from(documento.to_string())));
    *respuesta.status_mut() = StatusCode::OK;
    respuesta.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    respuesta
}

fn respuesta_texto(codigo: StatusCode, mensaje: &'static str) -> Response<CuerpoDeAdmin> {
    let mut respuesta = Response::new(Full::new(Bytes::from_static(mensaje.as_bytes())));
    *respuesta.status_mut() = codigo;
    respuesta
}

/// Acumula el cuerpo de la petición en flujo respetando el límite de bytes configurado.
///
/// Son dos guardas porque un cliente puede llegar por dos caminos distintos: si declara su
/// longitud, se le rechaza **antes** de reservar o leer un solo byte; si no la declara —cuerpo
/// troceado— no hay nada que comprobar por adelantado y la única defensa posible es acotar la
/// lectura mientras ocurre. Quitar cualquiera de las dos deja abierto uno de los dos caminos.
pub async fn acumular_cuerpo_acotado(
    peticion: Request<Incoming>,
    limite_bytes: usize,
) -> Result<Bytes, StatusCode> {
    if peticion
        .body()
        .size_hint()
        .upper()
        .is_some_and(|upper| upper > limite_bytes as u64)
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let acotado = http_body_util::Limited::new(peticion.into_body(), limite_bytes);
    match acotado.collect().await {
        Ok(recolectado) => Ok(recolectado.to_bytes()),
        Err(_) => Err(StatusCode::PAYLOAD_TOO_LARGE),
    }
}

/// Procesa una petición HTTP entrante sobre la interfaz de administración.
pub async fn atender_peticion_de_admin<F>(
    peticion: Request<Incoming>,
    estado: &Arc<EstadoDeAdmin>,
    servicio_embeddings: &Arc<ServicioDeEmbeddings<ProveedorDeEmbeddingsDeCelula>>,
    ruta_datos: &Path,
    limite_cuerpo_bytes: usize,
    debe_apagar: F,
) -> Response<CuerpoDeAdmin>
where
    F: Fn() -> bool + Send + Sync + Clone + 'static,
{
    let ruta = enrutar_admin(peticion.method(), peticion.uri().path());
    match ruta {
        RutaAdmin::ConsultarEstado => respuesta_de_fase(&estado.fase_actual()),
        RutaAdmin::DispararIngesta => {
            let bytes = match acumular_cuerpo_acotado(peticion, limite_cuerpo_bytes).await {
                Ok(b) => b,
                Err(codigo) => return respuesta_texto(codigo, "cuerpo demasiado grande"),
            };

            let documento_entrante: DocumentoEntrante = match serde_json::from_slice(&bytes) {
                Ok(doc) => doc,
                Err(_) => return respuesta_texto(StatusCode::BAD_REQUEST, "cuerpo JSON inválido"),
            };

            if !estado.intentar_iniciar() {
                return respuesta_texto(
                    StatusCode::CONFLICT,
                    "ya hay un trabajo de ingesta en curso",
                );
            }

            let documento = documento_entrante.en_documento_de_ingesta();
            let estado_de_la_tarea = Arc::clone(estado);
            let servicio_de_la_tarea = Arc::clone(servicio_embeddings);
            let ruta_datos_de_la_tarea = ruta_datos.to_path_buf();
            let debe_apagar_de_la_tarea = debe_apagar.clone();

            let tarea = tokio::task::spawn(async move {
                ejecutar_ingesta(
                    documento,
                    CONFIGURACION_DE_FRAGMENTACION_DE_INGESTA,
                    &servicio_de_la_tarea,
                    &ruta_datos_de_la_tarea,
                    TEXTO_DE_LA_SONDA_POR_DEFECTO,
                    UMBRAL_DE_ACEPTACION_POR_DEFECTO,
                    debe_apagar_de_la_tarea,
                )
                .await
                .map_err(|e| e.to_string())
            });

            tokio::task::spawn(supervisar_ingesta(estado_de_la_tarea, tarea));

            let mut resp = respuesta_de_fase(&FaseDeIngesta::EnCurso);
            *resp.status_mut() = StatusCode::ACCEPTED;
            resp
        }
        RutaAdmin::NoEncontrada => respuesta_texto(StatusCode::NOT_FOUND, ""),
    }
}

/// Vincula el listener administrativo y sirve peticiones HTTP.
pub async fn servir_admin<F>(
    direccion: SocketAddr,
    limite_cuerpo_bytes: usize,
    estado: Arc<EstadoDeAdmin>,
    servicio_embeddings: Arc<ServicioDeEmbeddings<ProveedorDeEmbeddingsDeCelula>>,
    ruta_datos: PathBuf,
    debe_apagar: F,
) -> std::io::Result<(SocketAddr, impl Future<Output = ()>)>
where
    F: Fn() -> bool + Send + Sync + Clone + 'static,
{
    let listener = TcpListener::bind(direccion).await?;
    let direccion_real = listener.local_addr()?;

    let futuro = async move {
        loop {
            let (flujo, _) = match listener.accept().await {
                Ok(aceptado) => aceptado,
                Err(_) => continue,
            };
            let io = TokioIo::new(flujo);
            let estado_conexion = Arc::clone(&estado);
            let servicio_conexion = Arc::clone(&servicio_embeddings);
            let ruta_conexion = ruta_datos.clone();
            let debe_apagar_conexion = debe_apagar.clone();

            tokio::task::spawn(async move {
                let atendido = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |peticion: Request<Incoming>| {
                            let estado = Arc::clone(&estado_conexion);
                            let servicio = Arc::clone(&servicio_conexion);
                            let ruta = ruta_conexion.clone();
                            let debe_apagar_fn = debe_apagar_conexion.clone();
                            async move {
                                Ok::<_, Infallible>(
                                    atender_peticion_de_admin(
                                        peticion,
                                        &estado,
                                        &servicio,
                                        &ruta,
                                        limite_cuerpo_bytes,
                                        debe_apagar_fn,
                                    )
                                    .await,
                                )
                            }
                        }),
                    )
                    .await;
                if let Err(error) = atendido {
                    eprintln!("admin: error sirviendo una conexión: {error}");
                }
            });
        }
    };

    Ok((direccion_real, futuro))
}

/// Vincula de forma unificada ambos servidores HTTP (salud y administración).
///
/// Se construyen juntos para que ninguna rama de `CanalSeleccionado` pueda quedarse sin uno de
/// los dos: una lista de futuros enumerada a mano en cada `tokio::select!` se puede omitir a
/// medias sin que nada falle, y el endpoint desaparecería en silencio de un canal.
///
/// El conteo de argumentos se admite a propósito: son las dependencias que cada listener ya
/// exigía por separado, y agruparlas en una estructura sería un cambio de diseño de la raíz de
/// composición, no de esta función.
#[allow(clippy::too_many_arguments)]
pub async fn servir_servicios_http<F>(
    direccion_salud: SocketAddr,
    estado_salud: Arc<EstadoDeSalud>,
    direccion_admin: SocketAddr,
    limite_cuerpo_admin_bytes: usize,
    estado_admin: Arc<EstadoDeAdmin>,
    servicio_embeddings: Arc<ServicioDeEmbeddings<ProveedorDeEmbeddingsDeCelula>>,
    ruta_datos: PathBuf,
    debe_apagar: F,
) -> std::io::Result<((SocketAddr, SocketAddr), impl Future<Output = ()>)>
where
    F: Fn() -> bool + Send + Sync + Clone + 'static,
{
    // Vincular los dos listeners tras un único `?` haría indistinguibles sus fallos, y ambos
    // pueden chocar con un puerto ajeno: quien opera necesita saber cuál de los dos se quedó sin
    // dirección y en qué dirección, que es justo lo que un `io::Error` desnudo no dice.
    let (dir_salud_real, servidor_salud) = servir_salud(direccion_salud, estado_salud)
        .await
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("servidor de salud en {direccion_salud}: {error}"),
            )
        })?;
    let (dir_admin_real, servidor_admin) = servir_admin(
        direccion_admin,
        limite_cuerpo_admin_bytes,
        estado_admin,
        servicio_embeddings,
        ruta_datos,
        debe_apagar,
    )
    .await
    .map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("servidor de administración en {direccion_admin}: {error}"),
        )
    })?;

    let futuro_combinado = async move {
        tokio::select! {
            () = servidor_salud => {}
            () = servidor_admin => {}
        }
    };

    Ok(((dir_salud_real, dir_admin_real), futuro_combinado))
}
