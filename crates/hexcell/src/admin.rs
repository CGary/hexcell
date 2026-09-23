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
    /// Petición `POST /admin/envio/pausa` para pausar o reanudar el envío saliente.
    PausarEnvio,
    /// Petición `POST /admin/sesion/emparejamiento` para iniciar el emparejamiento del dispositivo.
    IniciarEmparejamiento,
    /// Petición `GET /admin/sesion` para consultar el estado de sesión del canal.
    ConsultarSesion,
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
        (&Method::POST, "/admin/envio/pausa") => RutaAdmin::PausarEnvio,
        (&Method::POST, "/admin/sesion/emparejamiento") => RutaAdmin::IniciarEmparejamiento,
        (&Method::GET, "/admin/sesion") => RutaAdmin::ConsultarSesion,
        (&Method::POST, "/admin/sesion/cierre") => RutaAdmin::CerrarSesion,
        _ => RutaAdmin::NoEncontrada,
    }
}

/// DTO de entrada para la pausa de envío: `{"accion": "pausar" | "reanudar"}`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct PausarEnvioEntrante {
    pub accion: String,
}

/// DTO de entrada para el emparejamiento: `{"metodo": "qr" | "codigo_de_vinculacion"}`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct EmparejamientoEntrante {
    pub metodo: String,
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

/// Plazos de producción para las operaciones de sesión, en un solo struct para que la raíz de
/// composición los pase juntos y los tests inyecten los suyos.
///
/// Los valores por omisión son los de producción; los tests inyectan los suyos propios y nunca
/// importan esta constante (cierre y emparejamiento: 30 s; pausa: 30 s, operacional inmediato).
#[derive(Clone, Copy, Debug)]
pub struct PlazosDeSesion {
    /// Plazo para esperar el acuse de cierre de sesión.
    pub cierre: Duration,
    /// Plazo para esperar el acuse de la pausa/reanudación de envío.
    pub pausa: Duration,
    /// Plazo para esperar el código de emparejamiento.
    pub emparejamiento: Duration,
}

impl PlazosDeSesion {
    /// Plazos por omisión para producción (30 s en todas las operaciones).
    pub fn por_omision() -> Self {
        Self {
            cierre: Duration::from_secs(30),
            pausa: Duration::from_secs(30),
            emparejamiento: Duration::from_secs(30),
        }
    }
}

/// Motivo que la ruta devuelve cuando el canal no vincula ningún dispositivo.
///
/// Se declara junto a la ruta, no como retorno de ningún método del trait: el sub-trait
/// `CicloDeVidaSesion` es opcional y reservado a los adaptadores que vinculan un dispositivo
/// (ratificación R5, 2026-09-22). El valor lo aporta la variante `SinSesion` que la raíz de
/// composición elige en tiempo de compilación para el canal simulado.
pub const MOTIVO_CANAL_SIN_SESION: &str = "canal_sin_sesion";

/// Motivo de fallo cuando el adaptador reporta «sin conexión activa al sidecar».
///
/// Literal fijado por el contrato HTTP (D2): `POST /admin/sesion/emparejamiento` y
/// `POST /admin/envio/pausa` lo devuelven como `motivo` cuando el canal no puede alcanzar el
/// sidecar.
pub const MOTIVO_SIN_CONEXION: &str = "sin_conexion";

/// Motivo de fallo cuando el sidecar reporta que la sesión ya está emparejada.
///
/// Literal fijado por el contrato HTTP (D2): `POST /admin/sesion/emparejamiento` lo devuelve
/// cuando el acuse del sidecar lleva el texto `ya_emparejado` (la raíz de composición traduce el
/// texto exacto del sidecar a este literal).
pub const MOTIVO_YA_EMPAREJADA: &str = "ya_emparejada";

// ---------------------------------------------------------------------------
// Tipos de valor de las rutas de sesión
// ---------------------------------------------------------------------------

/// Acción admitida por `POST /admin/envio/pausa`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccionDePausa {
    Pausar,
    Reanudar,
}

/// Método de emparejamiento solicitado por `POST /admin/sesion/emparejamiento`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetodoSolicitado {
    Qr,
    CodigoDeVinculacion,
}

/// Resultado de una operación de pausa de envío.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesenlaceDePausa {
    /// El sidecar aplicó la acción.
    Aplicado,
    /// El sidecar rechazó la acción con un motivo.
    Fallido { motivo: String },
}

/// Resultado de una operación de emparejamiento.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesenlaceDeEmparejamiento {
    /// Se recibió un código válido.
    Codigo {
        metodo: String,
        valor: String,
        expira_en_ms: i64,
    },
    /// El emparejamiento falló con un motivo.
    Fallido { motivo: String },
}

// ---------------------------------------------------------------------------
// Operaciones de sesión (cuatro operaciones tipadas)
// ---------------------------------------------------------------------------

/// Tipo de caja para la operación de cierre de sesión: sin argumentos, devuelve resultado + motivo.
type CajaDeCierre =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

/// Tipo de caja para la operación de pausa de envío.
type CajaDePausaDeEnvio = Box<
    dyn Fn(AccionDePausa) -> Pin<Box<dyn Future<Output = DesenlaceDePausa> + Send>> + Send + Sync,
>;

/// Tipo de caja para la operación de emparejamiento.
type CajaDeEmparejamiento = Box<
    dyn Fn(
            MetodoSolicitado,
            Duration,
        ) -> Pin<Box<dyn Future<Output = DesenlaceDeEmparejamiento> + Send>>
        + Send
        + Sync,
>;

/// Tipo de caja para la operación de consulta de estado de sesión.
type CajaDeEstadoSesion = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = hexcell_core::canal::EstadoSesion> + Send>>
        + Send
        + Sync,
>;

/// Las cuatro operaciones de sesión, tipadas y borradas: el enum `SesionDeCanal` envuelve este
/// struct para la variante `ConSesion`. Cada campo es una caja que devuelve un futuro, todas con
/// los parámetros y tipos de resultado necesarios.
pub struct OperacionesDeSesion {
    /// Cierra la sesión: sin plazo externo (la caja lo fija internamente), devuelve motivo en error.
    pub cerrar: CajaDeCierre,
    /// Pausa o reanuda el envío, recibe la acción y el plazo, devuelve el desenlace.
    pub pausar_envio: CajaDePausaDeEnvio,
    /// Inicia el emparejamiento, recibe el método y el plazo, devuelve el desenlace.
    pub emparejar: CajaDeEmparejamiento,
    /// Consulta el estado actual de la sesión, devuelve el valor del puerto.
    pub estado: CajaDeEstadoSesion,
}

/// Enumerado que la raíz de composición entrega a las rutas de sesión.
///
/// `ConSesion` se construye únicamente en la rama de whatsmeow, con las cuatro operaciones; `SinSesion`
/// se construye en la rama del canal simulado, que no vincula ningún dispositivo. La distinción es
/// la que permite a las rutas devolver `canal_sin_sesion`, sin que las rutas mismas conozcan el canal.
///
/// No es genérico sobre el tipo del adaptador: la raíz de composición borra el tipo al construir
/// las cajas, así que `SinSesion` no necesita ningún parámetro de tipo y el enum puede usarse sin
/// anotar el adaptador subyacente.
pub enum SesionDeCanal {
    /// El canal vincula un dispositivo; las operaciones reales están disponibles.
    ConSesion(OperacionesDeSesion),
    /// El canal no vincula ningún dispositivo; no hay sesión que operar.
    SinSesion,
}

/// Registro de sesión: `OnceLock` que la raíz de composición rellena una sola vez, después de que
/// `servir_servicios_http` haya devuelto el futuro combinado.
///
/// El futuro combinado se construye **antes** de conocer el canal seleccionado, así que la sesión
/// no puede pasarse como argumento: se registra tarde, desde la rama del `match` sobre
/// `CanalSeleccionado`, y las rutas la leen a través de este `Arc`.
pub type RegistroDeSesion = Arc<OnceLock<SesionDeCanal>>;

impl SesionDeCanal {
    /// Construye un `SesionDeCanal::ConSesion` a partir de un valor que implementa
    /// `CicloDeVidaSesion`, borrando el tipo.
    ///
    /// Las operaciones `cerrar` y `estado` se delegan en el valor; `pausar_envio` y `emparejar`
    /// devuelven [`DesenlaceDePausa::Fallido`] / [`DesenlaceDeEmparejamiento::Fallido`] con
    /// `sin_conexion`, ya que el trait `CicloDeVidaSesion` no expone esas operaciones. La raíz de
    /// composición real (`construir_sesion_de_canal` en `main.rs`, con el asa del adaptador del canal) la usa
    /// para las cuatro operaciones; este constructor es para los tests de cierre de sesión.
    pub fn con_sesion<C>(valor: C) -> Self
    where
        C: CicloDeVidaSesion + Send + Sync + Clone + 'static,
        C::Error: std::fmt::Display,
    {
        let valor_cerrar = Arc::new(valor.clone());
        let valor_estado = Arc::new(valor);

        SesionDeCanal::ConSesion(OperacionesDeSesion {
            cerrar: Box::new(move || {
                let v = Arc::clone(&valor_cerrar);
                Box::pin(async move {
                    CicloDeVidaSesion::cerrar_sesion(&*v)
                        .await
                        .map_err(|e| e.to_string())
                })
            }),
            pausar_envio: Box::new(move |_accion| {
                Box::pin(async move {
                    DesenlaceDePausa::Fallido {
                        motivo: MOTIVO_SIN_CONEXION.to_string(),
                    }
                })
            }),
            emparejar: Box::new(move |_metodo, _plazo| {
                Box::pin(async move {
                    DesenlaceDeEmparejamiento::Fallido {
                        motivo: MOTIVO_SIN_CONEXION.to_string(),
                    }
                })
            }),
            estado: Box::new(move || {
                let v = Arc::clone(&valor_estado);
                Box::pin(async move { CicloDeVidaSesion::estado_sesion(&*v) })
            }),
        })
    }

    /// Registra la sesión en el `OnceLock` dado.
    ///
    /// Devuelve `Err(self)` si el registro ya estaba ocupado, para que la raíz de composición
    /// pueda decidir qué hacer; en la práctica, un segundo registro sería un defecto del código
    /// y no un escenario recuperable.
    pub fn registrar(self, registro: &RegistroDeSesion) -> Result<(), Self> {
        registro.set(self)
    }
}

/// Servicio de aplicación puro para el cierre de sesión, bajo prueba directa.
///
/// Devuelve el estado HTTP y el cuerpo JSON que la ruta debe emitir, sin tocar el transporte:
/// los tests lo invocan con un `registro` y un `plazo` inyectados, sin pasar por ningún servidor.
///
/// El contrato de alambre del cierre de sesión no cambia con la generalización del registro
/// (HEX-082): SinSesion → 200 completado + motivo canal_sin_sesion; ConSesion ok → 200;
/// ConSesion fallido → 502 con el motivo real; plazo agotado → 504; sin registrar → 502.
pub async fn atender_cierre_de_sesion(
    registro: &RegistroDeSesion,
    plazo: Duration,
) -> (StatusCode, serde_json::Value) {
    let sesion = match registro.get() {
        Some(s) => s,
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

    match sesion {
        SesionDeCanal::SinSesion => (
            StatusCode::OK,
            serde_json::json!({
                "resultado": "completado",
                "motivo": MOTIVO_CANAL_SIN_SESION
            }),
        ),
        SesionDeCanal::ConSesion(operaciones) => {
            let futuro = (operaciones.cerrar)();
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

/// Servicio de aplicación puro para la pausa/reanudación de envío, bajo prueba directa.
///
/// `accion` ya fue validada por la ruta (Pausar o Reanudar); `plazo` lo inyecta la ruta.
/// SinSesion → 200 resultado canal_sin_sesion; ConSesion → ejecuta la operación y traduce su
/// desenlace (Aplicado → 200 resultado aplicado + accion; Fallido → 200 resultado fallido + accion
/// + motivo); sin registrar → 502 fallido (fallar cerrado).
pub async fn atender_pausa_de_envio(
    registro: &RegistroDeSesion,
    accion: AccionDePausa,
    plazo: Duration,
) -> (StatusCode, serde_json::Value) {
    let sesion = match registro.get() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "resultado": "fallido",
                    "motivo": "pausa de envío no registrada en la composición"
                }),
            );
        }
    };

    match sesion {
        SesionDeCanal::SinSesion => (
            StatusCode::OK,
            serde_json::json!({ "resultado": MOTIVO_CANAL_SIN_SESION }),
        ),
        SesionDeCanal::ConSesion(operaciones) => {
            let futuro = (operaciones.pausar_envio)(accion);
            match tokio::time::timeout(plazo, futuro).await {
                Ok(DesenlaceDePausa::Aplicado) => {
                    let accion_str = match accion {
                        AccionDePausa::Pausar => "pausar",
                        AccionDePausa::Reanudar => "reanudar",
                    };
                    (
                        StatusCode::OK,
                        serde_json::json!({
                            "resultado": "aplicado",
                            "accion": accion_str,
                        }),
                    )
                }
                Ok(DesenlaceDePausa::Fallido { motivo }) => {
                    let accion_str = match accion {
                        AccionDePausa::Pausar => "pausar",
                        AccionDePausa::Reanudar => "reanudar",
                    };
                    (
                        StatusCode::OK,
                        serde_json::json!({
                            "resultado": "fallido",
                            "accion": accion_str,
                            "motivo": motivo,
                        }),
                    )
                }
                Err(_agotado) => {
                    let accion_str = match accion {
                        AccionDePausa::Pausar => "pausar",
                        AccionDePausa::Reanudar => "reanudar",
                    };
                    (
                        StatusCode::OK,
                        serde_json::json!({
                            "resultado": "fallido",
                            "accion": accion_str,
                            "motivo": "no se recibió acuse de pausa de envío dentro del plazo"
                        }),
                    )
                }
            }
        }
    }
}

/// Servicio de aplicación puro para el emparejamiento, bajo prueba directa.
///
/// `metodo` ya fue validada por la ruta (Qr o CodigoDeVinculacion); `plazo` lo inyecta la ruta.
/// SinSesion → 200 resultado canal_sin_sesion; ConSesion → ejecuta la operación y traduce su
/// desenlace (Codigo → 200 resultado codigo + metodo + valor + expira_en_ms; Fallido → 200
/// resultado fallido + motivo); sin registrar → 502 fallido (fallar cerrado). Si el plazo se
/// agota, la operación interna resuelve con Fallido{plazo}.
pub async fn atender_emparejamiento(
    registro: &RegistroDeSesion,
    metodo: MetodoSolicitado,
    plazo: Duration,
) -> (StatusCode, serde_json::Value) {
    let sesion = match registro.get() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "resultado": "fallido",
                    "motivo": "emparejamiento no registrado en la composición"
                }),
            );
        }
    };

    match sesion {
        SesionDeCanal::SinSesion => (
            StatusCode::OK,
            serde_json::json!({ "resultado": MOTIVO_CANAL_SIN_SESION }),
        ),
        SesionDeCanal::ConSesion(operaciones) => {
            let futuro = (operaciones.emparejar)(metodo, plazo);
            match tokio::time::timeout(plazo, futuro).await {
                Ok(DesenlaceDeEmparejamiento::Codigo {
                    metodo,
                    valor,
                    expira_en_ms,
                }) => (
                    StatusCode::OK,
                    serde_json::json!({
                        "resultado": "codigo",
                        "metodo": metodo,
                        "valor": valor,
                        "expira_en_ms": expira_en_ms,
                    }),
                ),
                Ok(DesenlaceDeEmparejamiento::Fallido { motivo }) => (
                    StatusCode::OK,
                    serde_json::json!({
                        "resultado": "fallido",
                        "motivo": motivo,
                    }),
                ),
                Err(_agotado) => (
                    StatusCode::OK,
                    serde_json::json!({
                        "resultado": "fallido",
                        "motivo": "no se recibió código de emparejamiento dentro del plazo"
                    }),
                ),
            }
        }
    }
}

/// Servicio de aplicación puro para la consulta de estado de sesión, bajo prueba directa.
///
/// SinSesion → 200 estado canal_sin_sesion; ConSesion → consulta la operación `estado` y traduce
/// el valor del puerto a los cuatro literales; sin registrar → 502 fallido (fallar cerrado).
pub async fn atender_consulta_de_sesion(
    registro: &RegistroDeSesion,
    plazo: Duration,
) -> (StatusCode, serde_json::Value) {
    let _ = plazo; // La consulta es síncrona; el plazo se mantiene en la firma por uniformidad.
    let sesion = match registro.get() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_GATEWAY,
                serde_json::json!({
                    "resultado": "fallido",
                    "motivo": "consulta de sesión no registrada en la composición"
                }),
            );
        }
    };

    match sesion {
        SesionDeCanal::SinSesion => (
            StatusCode::OK,
            serde_json::json!({ "estado": MOTIVO_CANAL_SIN_SESION }),
        ),
        SesionDeCanal::ConSesion(operaciones) => {
            let futuro = (operaciones.estado)();
            let estado = futuro.await;
            let estado_str = match estado {
                hexcell_core::canal::EstadoSesion::Activa => "activa",
                hexcell_core::canal::EstadoSesion::Reconectando => "reconectando",
                hexcell_core::canal::EstadoSesion::Desvinculada => "desvinculada",
                hexcell_core::canal::EstadoSesion::Pausada => "pausada",
            };
            (StatusCode::OK, serde_json::json!({ "estado": estado_str }))
        }
    }
}

/// Construye una respuesta HTTP JSON genérica (estado y cuerpo dados).
fn respuesta_json(estado: StatusCode, cuerpo: serde_json::Value) -> Response<CuerpoDeAdmin> {
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
    registro_sesion: &RegistroDeSesion,
    plazos: &PlazosDeSesion,
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
        // POST /admin/envio/pausa: pausa o reanudación del envío saliente.
        //
        // NO lleva autenticación: la frontera de seguridad es la red interna de la célula,
        // exactamente igual que /admin/ingesta. El cuerpo se analiza como JSON sin importar el
        // Content-Type (busybox wget --post-data envía x-www-form-urlencoded). Una acción inválida,
        // ausente o un cuerpo que no es JSON se resuelve con 400 ANTES de invocar ninguna operación.
        RutaAdmin::PausarEnvio => {
            let bytes = match acumular_cuerpo_acotado(peticion, limite_cuerpo_bytes).await {
                Ok(b) => b,
                Err(codigo) => return respuesta_texto(codigo, "cuerpo demasiado grande"),
            };

            let pausa: PausarEnvioEntrante = match serde_json::from_slice(&bytes) {
                Ok(p) => p,
                Err(_) => {
                    return respuesta_json(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "resultado": "fallido", "motivo": "cuerpo JSON inválido" }),
                    );
                }
            };

            let accion = match pausa.accion.as_str() {
                "pausar" => AccionDePausa::Pausar,
                "reanudar" => AccionDePausa::Reanudar,
                _ => {
                    return respuesta_json(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "resultado": "fallido", "motivo": "acción inválida" }),
                    );
                }
            };

            let (estado_http, cuerpo) =
                atender_pausa_de_envio(registro_sesion, accion, plazos.pausa).await;
            respuesta_json(estado_http, cuerpo)
        }
        // POST /admin/sesion/emparejamiento: inicio del emparejamiento del dispositivo.
        //
        // NO lleva autenticación: la frontera de seguridad es la red interna de la célula.
        // El cuerpo se analiza como JSON sin importar el Content-Type. Un método inválido o ausente
        // se resuelve con 400 ANTES de invocar ninguna operación. El plazo de espera del código es
        // `PlazosDeSesion.emparejamiento` (30 s); si se agota, la operación resuelve con
        // Fallido{plazo} y la ruta contesta 200 fallido.
        RutaAdmin::IniciarEmparejamiento => {
            let bytes = match acumular_cuerpo_acotado(peticion, limite_cuerpo_bytes).await {
                Ok(b) => b,
                Err(codigo) => return respuesta_texto(codigo, "cuerpo demasiado grande"),
            };

            let empa: EmparejamientoEntrante = match serde_json::from_slice(&bytes) {
                Ok(e) => e,
                Err(_) => {
                    return respuesta_json(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "resultado": "fallido", "motivo": "cuerpo JSON inválido" }),
                    );
                }
            };

            let metodo = match empa.metodo.as_str() {
                "qr" => MetodoSolicitado::Qr,
                "codigo_de_vinculacion" => MetodoSolicitado::CodigoDeVinculacion,
                _ => {
                    return respuesta_json(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "resultado": "fallido", "motivo": "método inválido" }),
                    );
                }
            };

            let (estado_http, cuerpo) =
                atender_emparejamiento(registro_sesion, metodo, plazos.emparejamiento).await;
            respuesta_json(estado_http, cuerpo)
        }
        // GET /admin/sesion: consulta del estado de sesión del canal.
        //
        // NO lleva autenticación: la frontera de seguridad es la red interna de la célula. No lleva
        // cuerpo; la respuesta es uno de los cuatro literales del puerto o canal_sin_sesion.
        RutaAdmin::ConsultarSesion => {
            let (estado_http, cuerpo) =
                atender_consulta_de_sesion(registro_sesion, plazos.cierre).await;
            respuesta_json(estado_http, cuerpo)
        }
        // POST /admin/sesion/cierre: cierre de sesión del canal.
        //
        // NO lleva autenticación: la frontera de seguridad es la red interna de la célula,
        // exactamente igual que /admin/ingesta. El listener administrativo por omisión escucha
        // en loopback (crates/hexcell/src/configuracion.rs) y la plantilla de despliegue lo
        // abre a 0.0.0.0 únicamente dentro de la red de célula (deploy/cell.compose.yml),
        // que es la frontera declarada. El contrato de alambre del cierre no cambia (HEX-082):
        // SinSesion → 200 completado + motivo canal_sin_sesion; ConSesion ok → 200; ConSesion
        // fallido → 502 con el motivo real; plazo → 504; sin registrar → 502.
        RutaAdmin::CerrarSesion => {
            let (estado_http, cuerpo) =
                atender_cierre_de_sesion(registro_sesion, plazos.cierre).await;
            respuesta_json(estado_http, cuerpo)
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
    registro_sesion: RegistroDeSesion,
    plazos: PlazosDeSesion,
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
            let registro_sesion_conexion = Arc::clone(&registro_sesion);

            tokio::task::spawn(async move {
                let atendido = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |peticion: Request<Incoming>| {
                            let estado = Arc::clone(&estado_conexion);
                            let servicio = Arc::clone(&servicio_conexion);
                            let ruta = ruta_conexion.clone();
                            let debe_apagar_fn = debe_apagar_conexion.clone();
                            let registro = Arc::clone(&registro_sesion_conexion);
                            let plazos = plazos;
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
                                        &plazos,
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
/// # Registro de sesión tardío
///
/// El futuro combinado se construye **antes** de conocer el canal seleccionado, así que el
/// registro no puede pasarse como argumento directo: la raíz de composición lo registra tarde,
/// desde la rama del `match` sobre `CanalSeleccionado`, y las rutas lo leen a través del
/// `RegistroDeSesion` que se pasa aquí.
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
    registro_sesion: RegistroDeSesion,
    plazos: PlazosDeSesion,
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
        registro_sesion,
        plazos,
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
