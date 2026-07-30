//! Servidor HTTP interno de salud: `GET /health/live` y `GET /health/ready`.
//!
//! No es una ruta de cara al público: la sondea la CLI de administración sobre la interfaz
//! interna que resuelve `crate::configuracion::Configuracion::direccion_salud` (loopback por
//! defecto).
//!
//! Las dos rutas responden preguntas distintas y **no** deben confundirse:
//!
//! * `GET /health/live` responde 200 en cuanto el proceso vive. No consulta los pools ni el estado
//!   del canal, y no puede responder un error: si atara la vivacidad a la persistencia, un
//!   supervisor reiniciaría en bucle una célula cuyo disco está temporalmente ocupado, que es la
//!   reacción exactamente contraria a la útil.
//! * `GET /health/ready` responde si la célula puede **atender un mensaje ahora mismo**: la
//!   conjunción de las dos vitalidades de `crate::preparacion` y del estado de sesión del canal.
//!   Devuelve 503 nombrando el componente que falló.
//!
//! Ningún guardián de cerrojo cruza un `.await`: la sonda de vitalidad es síncrona de principio a
//! fin y suelta las conexiones antes de que el futuro que la envuelve ceda el control.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use hexcell_storage::GestorDePools;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::preparacion::{Preparacion, SesionDelCanal, evaluar_preparacion};

/// Cuerpo de respuesta de este servidor: texto fijo, sin streaming.
type CuerpoDeSalud = Full<Bytes>;

/// Lo que el servidor de salud necesita para responder: los pools y el estado de sesión.
///
/// Se agrupan en un tipo propio para que `servir_salud` reciba **una** cosa y para que la raíz de
/// composición sea el único sitio donde se decide de dónde sale el estado de sesión.
pub struct EstadoDeSalud {
    pools: Arc<GestorDePools>,
    sesion: SesionDelCanal,
}

impl EstadoDeSalud {
    /// Agrupa los pools ya abiertos con el estado de sesión del canal.
    pub fn nuevo(pools: Arc<GestorDePools>, sesion: SesionDelCanal) -> Self {
        Self { pools, sesion }
    }

    /// Evalúa la preparación consultando las dos sondas de vitalidad y el estado de sesión.
    ///
    /// Síncrona a propósito: las dos consultas se hacen y se cierran aquí, así que ningún
    /// guardián de cerrojo puede sobrevivir hasta el siguiente punto de espera del servidor.
    pub fn preparacion(&self) -> Preparacion {
        evaluar_preparacion(
            self.pools.sesiones().vitalidad(),
            self.pools.conocimiento().vitalidad(),
            &self.sesion,
        )
    }
}

/// Construye una respuesta sin pasar por el constructor falible del builder.
///
/// `Response::builder()` devuelve un `Result` que obligaría a un `expect()` para un caso que no
/// puede ocurrir, y `[profile.release]` fija `panic = "abort"`: un pánico en producción no dejaría
/// ningún mensaje utilizable. Esta forma no puede fallar.
fn respuesta(codigo: StatusCode, cuerpo: Bytes) -> Response<CuerpoDeSalud> {
    let mut respuesta = Response::new(Full::new(cuerpo));
    *respuesta.status_mut() = codigo;
    respuesta
}

/// Atiende una petición ya recibida, sin tocar la red: función pura respecto del transporte, para
/// poder probar el enrutado y la preparación sin vincular ningún puerto.
pub fn atender_peticion_de_salud(
    peticion: &Request<Incoming>,
    estado: &EstadoDeSalud,
) -> Response<CuerpoDeSalud> {
    match (peticion.method(), peticion.uri().path()) {
        (&Method::GET, "/health/live") => respuesta(StatusCode::OK, Bytes::from_static(b"viva")),
        (&Method::GET, "/health/ready") => match estado.preparacion() {
            Preparacion::Lista => respuesta(StatusCode::OK, Bytes::from_static(b"lista")),
            Preparacion::NoLista { componente, motivo } => respuesta(
                StatusCode::SERVICE_UNAVAILABLE,
                Bytes::from(format!("no lista: {componente}: {motivo}")),
            ),
        },
        _ => respuesta(StatusCode::NOT_FOUND, Bytes::new()),
    }
}

/// Vincula el listener de salud y sirve conexiones indefinidamente.
///
/// Devuelve la dirección **realmente** vinculada (útil cuando `direccion` llega con el puerto en
/// `0`, para que quien llama pueda leer el puerto real elegido por el sistema operativo, como
/// hacen los tests de este binario) junto con el futuro que sirve el servidor.
pub async fn servir_salud(
    direccion: SocketAddr,
    estado: Arc<EstadoDeSalud>,
) -> std::io::Result<(SocketAddr, impl Future<Output = ()>)> {
    let listener = TcpListener::bind(direccion).await?;
    let direccion_real = listener.local_addr()?;

    let futuro = async move {
        loop {
            let (flujo, _) = match listener.accept().await {
                Ok(aceptado) => aceptado,
                Err(_) => continue,
            };
            let io = TokioIo::new(flujo);
            let estado_de_la_conexion = Arc::clone(&estado);

            tokio::task::spawn(async move {
                let atendido = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |peticion: Request<Incoming>| {
                            let estado = Arc::clone(&estado_de_la_conexion);
                            async move {
                                Ok::<_, Infallible>(atender_peticion_de_salud(&peticion, &estado))
                            }
                        }),
                    )
                    .await;
                if let Err(error) = atendido {
                    eprintln!("salud: error sirviendo una conexión: {error}");
                }
            });
        }
    };

    Ok((direccion_real, futuro))
}
