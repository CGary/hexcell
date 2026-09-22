//! Cliente del socket Unix del motor Docker.
//!
//! Módulo interno de `hexcell-admin` que habla la API del motor Docker por su socket Unix usando un
//! cliente HTTP/1.1 síncrono escrito a mano sobre [`std::os::unix::net::UnixStream`]: sin bollard,
//! sin hyper y sin tokio. Expone arranque de contenedor (crear + iniciar, con o sin red y `Cmd`
//! explícitos), arranque de un contenedor ya creado, parada con margen de gracia de 30 segundos
//! (`t=30`, nunca un bucle de espera y matar en el cliente) y parada sin plazo explícito, espera
//! del código de salida, inspección, eliminación de contenedor y eliminación de volumen, cada una
//! con un error tipado.
//!
//! # Límite de alcance
//!
//! Aquí no viven el analizador de argumentos de la CLI, el formato de salida, los códigos de
//! retorno, el modo de simulación ni el modelo de estado de la célula (tarea 10 de la etapa A-6),
//! ni la orquestación de `cell pause`/`cell unpause` con su sondeo de disponibilidad (tarea 11), ni
//! la obtención de registros, la construcción o la descarga de imágenes. Todo eso es alcance de
//! tareas posteriores que se construirán sobre este módulo. El listado de contenedores vive en
//! `inventario`, no en `cliente`, porque `cliente` no lo exponía y esta tarea no lo modifica.

mod cliente;
mod error;
mod inventario;
mod transporte;

pub use cliente::{ClienteDocker, OpcionesDeContenedor, ResultadoDeArranque};
pub use error::ErrorDeClienteDocker;
pub use inventario::{InventarioDocker, ResumenDeContenedor};
pub use transporte::{ConexionDocker, RespuestaHttp};
