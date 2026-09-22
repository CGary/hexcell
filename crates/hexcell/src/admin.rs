//! Servidor HTTP interno de administración: `POST /admin/ingesta`, `GET /admin/ingesta` y
//! `POST /admin/sesion/cierre`.
//!
//! Expone una interfaz interna, accesible únicamente desde la red local o loopback, para desencadenar
//! la ingesta de conocimiento en segundo plano en la base en sombra (`knowledge_staging.db`),
//! permitir a una CLI de administración consultar el estado del trabajo mediante sondeos (polling),
//! y ordenar el cierre de sesión del canal de la célula.
//!
//! Se ejecuta sobre su propio puerto (`HEXCELL_DIRECCION_ADMIN`), independiente del servidor de salud
//! (`HEXCELL_DIRECCION_SALUD`), para permitir separar la exposición de ambas superficies en etapas
//! futuras de empaquetado y seguridad.
//!
//! Ninguna clave de cerrojo cruza un `.await`: el estado del proceso de ingesta se gestiona en memoria
//! de forma síncrona en una única sección crítica (`std::sync::Mutex`), garantizando que como máximo
//! un trabajo de ingesta corra a la vez por célula.

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use hexcell_core::canal::CicloDeVidaSesion;
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
/// y de pruebas. Se conserva de todos modos por una razón que basta sola: evita que el hueco
/// reaparezca en silencio si algún día el perfil de release vuelve a desenrollar.
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
    /// Petición `POST /admin/sesion/cierre` para ordenar el cierre de sesión del canal.
    CerrarSesion,
    /// Ruta o método no reconocido.
    NoEncontrada,
}

/// Enruta puramente una petición a partir del método HTTP y la ruta de la URI.
pub fn enrutar_admin(metodo: &Method, ruta: &str) -> RutaAdmin {
    match (metodo, ruta) {
        (&Method::POST, "/admin/ingesta") => RutaAdmin::DispararIngesta,
        (&Method::GET, "/admin/ingesta") => RutaAdmin::ConsultarEstado,
        (&Method::POST, "/admin/sesion/cierre") => RutaAdmin::CerrarSesion,
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

/// Plazo por omisión para esperar el acuse de cierre de sesión desde la ruta HTTP.
///
/// Es el valor de producción; los tests inyectan el suyo propio y nunca importan esta constante.
pub const PLAZO_DE_CIERRE_DE_SESION: Duration = Duration::from_secs(30);

/// Motivo que la ruta devuelve cuando el canal no vincula ningún dispositivo.
///
/// Se declara junto a la ruta, no como retorno de ningún método del trait: el sub-trait
/// `CicloDeVidaSesion` es opcional y reservado a los adaptadores que vinculan un dispositivo
/// (ratificación R5, 2026-09-22). El valor lo aporta la variante `SinSesion` que la raíz de
/// composición elige en tiempo de compilación para el canal simulado.
pub const MOTIVO_CANAL_SIN_SESION: &str = "canal_sin_sesion";

/// Tipo de la caja que envuelve la operación de cierre de sesión.
///
/// Se extrae como alias porque el tipo completo es demasiado complejo para clippy
/// (`type_complexity`) y porque se repite en la definición de `CierreDeSesion`.
type CajaDeCierre =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

/// Enumerado que la raíz de composición entrega a la ruta.
///
/// `ConSesion` se construye únicamente en la rama de whatsmeow, con una caja que devuelve el
/// resultado de `cerrar_sesion`; `SinSesion` se construye en la rama del canal simulado, que no
/// vincula ningún dispositivo. La distinción es la que permite a la ruta devolver 200 con motivo
/// `canal_sin_sesion` en un caso y 200 sin motivo en el otro, sin que la ruta misma conozca el
/// canal.
///
/// No es genérico sobre el tipo del adaptador: la raíz de composición borra el tipo al construir
/// la caja, así que `SinSesion` no necesita ningún parámetro de tipo y el enum puede usarse sin
/// anotar el adaptador subyacente.
pub enum CierreDeSesion {
    /// El canal vincula un dispositivo; cerrar la sesión requiere la operación real.
    ConSesion(CajaDeCierre),
    /// El canal no vincula ningún dispositivo; no hay sesión que cerrar.
    SinSesion,
}

/// Versión registrada de `CierreDeSesion`, almacenada en un `OnceLock`.
///
/// Es el mismo tipo que `CierreDeSesion`; el alias existe para distinguir el rol: el
/// `RegistroDeCierreDeSesion` guarda un `CerradorRegistrado`, no un `CierreDeSesion` fresco.
pub type CerradorRegistrado = CierreDeSesion;

/// Registro de cierre de sesión: `OnceLock` que la raíz de composición rellena una sola vez,
/// después de que `servir_servicios_http` haya devuelto el futuro combinado.
///
/// El futuro combinado se construye **antes** de conocer el canal seleccionado, así que el
/// cerrador no puede pasarse como argumento: se registra tarde, desde la rama del `match` sobre
/// `CanalSeleccionado`, y la ruta lo lee a través de este `Arc`.
pub type RegistroDeCierreDeSesion = Arc<OnceLock<CerradorRegistrado>>;

impl CierreDeSesion {
    /// Construye un `CierreDeSesion::ConSesion` a partir de un adaptador que implementa
    /// `CicloDeVidaSesion`, borrando el tipo.
    pub fn con_sesion<C>(adaptador: C) -> Self
    where
        C: CicloDeVidaSesion + Send + Sync + 'static,
        C::Error: std::fmt::Display,
    {
        let adaptador = Arc::new(adaptador);
        CierreDeSesion::ConSesion(Box::new(move || {
            let adaptador = Arc::clone(&adaptador);
            Box::pin(async move { adaptador.cerrar_sesion().await.map_err(|e| e.to_string()) })
                as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        }))
    }

    /// Registra el cierre en el `OnceLock` dado.
    ///
    /// Devuelve `Err(self)` si el registro ya estaba ocupado, para que la raíz de composición
    /// pueda decidir qué hacer; en la práctica, un segundo registro sería un defecto del código
    /// y no un escenario recuperable.
    pub fn registrar(self, registro: &RegistroDeCierreDeSesion) -> Result<(), Self> {
        registro.set(self)
    }
}

/// Servicio de aplicación puro para el cierre de sesión, bajo prueba directa.
///
/// Devuelve el estado HTTP y el cuerpo JSON que la ruta debe emitir, sin tocar el transporte:
/// los tests lo invocan con un `registro` y un `plazo` inyectados, sin pasar por ningún servidor.
pub async fn atender_cierre_de_sesion(
    registro: &RegistroDeCierreDeSesion,
    plazo: Duration,
) -> (StatusCode, serde_json::Value) {
    let cerrador = match registro.get() {
        Some(c) => c,
        // Sin registro: fallar cerrado. Un 200 aquí destruiría el volumen con la sesión aún
        // vinculada; un 502 obliga al operador a investigar antes de reintentar.
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "resultado": "fallido",
                    "motivo": "cierre de sesión no registrado en la composición"
                }),
            );
        }
    };

    match cerrador {
        CierreDeSesion::SinSesion => (
            StatusCode::OK,
            serde_json::json!({
                "resultado": "completado",
                "motivo": MOTIVO_CANAL_SIN_SESION
            }),
        ),
        CierreDeSesion::ConSesion(f) => {
            let futuro = f();
            match tokio::time::timeout(plazo, futuro).await {
                Ok(Ok(())) => (
                    StatusCode::OK,
                    serde_json::json!({ "resultado": "completado" }),
                ),
                Ok(Err(motivo)) => (
                    StatusCode::BAD_GATEWAY,
                    serde_json::json!({
                        "resultado": "fallido",
                        "motivo": motivo
                    }),
                ),
                Err(_agotado) => (
                    StatusCode::GATEWAY_TIMEOUT,
                    serde_json::json!({
                        "resultado": "ausente",
                        "motivo": "no se recibió acuse de cierre de sesión dentro del plazo"
                    }),
                ),
            }
        }
    }
}

/// Construye la respuesta HTTP del cierre de sesión a partir del resultado de
/// `atender_cierre_de_sesion`.
fn respuesta_de_cierre_de_sesion(
    estado: StatusCode,
    cuerpo: serde_json::Value,
) -> Response<CuerpoDeAdmin> {
    let mut respuesta = Response::new(Full::new(Bytes::from(cuerpo.to_string())));
    *respuesta.status_mut() = estado;
    respuesta.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
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
#[allow(clippy::too_many_arguments)]
pub async fn atender_peticion_de_admin<F>(
    peticion: Request<Incoming>,
    estado: &Arc<EstadoDeAdmin>,
    servicio_embeddings: &Arc<ServicioDeEmbeddings<ProveedorDeEmbeddingsDeCelula>>,
    ruta_datos: &Path,
    limite_cuerpo_bytes: usize,
    debe_apagar: F,
    registro_cierre: &RegistroDeCierreDeSesion,
    plazo_cierre: Duration,
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
        // POST /admin/sesion/cierre: cierre de sesión del canal.
        //
        // NO lleva autenticación: la frontera de seguridad es la red interna de la célula,
        // exactamente igual que /admin/ingesta. El listener administrativo por omisión escucha
        // en loopback (crates/hexcell/src/configuracion.rs) y la plantilla de despliegue lo
        // abre a 0.0.0.0 únicamente dentro de la red de célula (deploy/cell.compose.yml),
        // que es la frontera declarada.
        RutaAdmin::CerrarSesion => {
            let (estado_http, cuerpo) =
                atender_cierre_de_sesion(registro_cierre, plazo_cierre).await;
            respuesta_de_cierre_de_sesion(estado_http, cuerpo)
        }
        RutaAdmin::NoEncontrada => respuesta_texto(StatusCode::NOT_FOUND, ""),
    }
}

/// Vincula el listener administrativo y sirve peticiones HTTP.
#[allow(clippy::too_many_arguments)]
pub async fn servir_admin<F>(
    direccion: SocketAddr,
    limite_cuerpo_bytes: usize,
    estado: Arc<EstadoDeAdmin>,
    servicio_embeddings: Arc<ServicioDeEmbeddings<ProveedorDeEmbeddingsDeCelula>>,
    ruta_datos: PathBuf,
    debe_apagar: F,
    registro_cierre: RegistroDeCierreDeSesion,
    plazo_cierre: Duration,
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
            let registro_cierre_conexion = Arc::clone(&registro_cierre);

            tokio::task::spawn(async move {
                let atendido = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |peticion: Request<Incoming>| {
                            let estado = Arc::clone(&estado_conexion);
                            let servicio = Arc::clone(&servicio_conexion);
                            let ruta = ruta_conexion.clone();
                            let debe_apagar_fn = debe_apagar_conexion.clone();
                            let registro = Arc::clone(&registro_cierre_conexion);
                            async move {
                                Ok::<_, Infallible>(
                                    atender_peticion_de_admin(
                                        peticion,
                                        &estado,
                                        &servicio,
                                        &ruta,
                                        limite_cuerpo_bytes,
                                        debe_apagar_fn,
                                        &registro,
                                        plazo_cierre,
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
///
/// # Registro de cierre de sesión tardío
///
/// El futuro combinado se construye **antes** de conocer el canal seleccionado, así que el
/// cerrador no puede pasarse como argumento directo: la raíz de composición lo registra tarde,
/// desde la rama del `match` sobre `CanalSeleccionado`, y la ruta lo lee a través del
/// `RegistroDeCierreDeSesion` que se pasa aquí.
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
    registro_cierre: RegistroDeCierreDeSesion,
    plazo_cierre: Duration,
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
        registro_cierre,
        plazo_cierre,
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
