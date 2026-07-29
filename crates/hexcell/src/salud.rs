//! Servidor HTTP interno de salud: `GET /health/live` y `GET /health/ready`.
//!
//! No es una ruta de cara al público: la sondea la CLI de administración sobre la interfaz
//! interna que resuelve `crate::configuracion::Configuracion::direccion_salud` (loopback por
//! defecto). `GET /health/live` responde 200 en cuanto el proceso vive, sin depender de ningún
//! pool ni del estado del canal. `GET /health/ready` existe como ruta y responde con un cuerpo
//! fijo que documenta que su lógica real —comprobar los pools SQLite y el estado de sesión del
//! puerto de canal— llega en la etapa HEX-006 (tarea 7 del plan); esta respuesta no comprueba
//! nada todavía y no debe fingir que lo hace.

use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

/// Cuerpo de respuesta de este servidor: texto fijo, sin streaming.
type CuerpoDeSalud = Full<Bytes>;

/// Atiende una petición ya recibida, sin tocar la red: función pura para poder probar el
/// enrutado de las dos rutas sin bindear ningún puerto.
pub fn atender_peticion_de_salud(peticion: &Request<Incoming>) -> Response<CuerpoDeSalud> {
    match (peticion.method(), peticion.uri().path()) {
        (&Method::GET, "/health/live") => Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from_static(b"viva")))
            .expect("una respuesta 200 con cuerpo fijo siempre es válida"),
        (&hyper::Method::GET, "/health/ready") => Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from_static(
                b"esqueleto: la comprobacion real de los pools SQLite y del estado de \
                  sesion del puerto de canal llega en HEX-006 (tarea 7 del plan); esta \
                  respuesta no comprueba nada todavia",
            )))
            .expect("una respuesta 200 con cuerpo fijo siempre es válida"),
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::new()))
            .expect("una respuesta 404 sin cuerpo siempre es válida"),
    }
}

/// Vincula el listener de salud y sirve conexiones indefinidamente.
///
/// Devuelve la dirección **realmente** vinculada (útil cuando `direccion` llega con el puerto en
/// `0`, para que quien llama pueda leer el puerto real elegido por el sistema operativo, como
/// hacen los tests de este binario) junto con el futuro que sirve el servidor.
pub async fn servir_salud(
    direccion: SocketAddr,
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

            tokio::task::spawn(async move {
                let atendido = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(|peticion: Request<Incoming>| async move {
                            Ok::<_, Infallible>(atender_peticion_de_salud(&peticion))
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
