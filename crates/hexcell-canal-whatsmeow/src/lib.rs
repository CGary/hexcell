//! Adaptador `ChannelAdapter` de whatsmeow: cliente IPC sobre socket Unix.
//!
//! Este crate implementa el lado Rust del protocolo IPC versión 4
//! (`docs/protocolo-ipc-nucleo-sidecar.md`, documento 1.3) como cliente que conecta al sidecar
//! Go de whatsmeow. El sidecar escucha, el núcleo conecta (sección 2, nunca al revés).
//!
//! # Módulos
//!
//! - [`mensajes`]: objetos de valor del protocolo (serde structs, `deny_unknown_fields`).
//! - [`conexion`]: capa de transporte (dial, saludo, lectura acotada, confirmación).
//! - [`reconexion`]: retroceso exponencial determinista con techo, inyectable.
//! - [`adaptador`]: servicio de aplicación (`AdaptadorWhatsmeow`).
//! - [`error`]: averías de transporte.
//!
//! # Decisión de análisis
//!
//! Se usa `serde` + `serde_json` para analizar las líneas JSON del protocolo. La reconciliación
//! con `adr-0019` —que rechazó un serializador para **emitir** líneas de registro— vive en
//! `docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md`: esta tarea **analiza** entrada adversarial
//! en una frontera de confianza donde `contenido` transporta texto hostil arbitrario, y hacerlo
//! sin biblioteca es estrictamente más difícil y propenso a errores.

pub mod adaptador;
pub mod conexion;
pub mod error;
pub mod mensajes;
pub mod reconexion;

pub use adaptador::AdaptadorWhatsmeow;
pub use error::ErrorCanalWhatsmeow;
pub use mensajes::VERSION_PROTOCOLO;
pub use reconexion::Retroceso;
