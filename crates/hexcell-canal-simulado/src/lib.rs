//! Adaptador `ChannelAdapter` simulado en memoria.
//!
//! Este crate **imita la semántica restrictiva de la Meta Cloud API, no la de whatsmeow**: ventana
//! de servicio de 24 horas, `PlantillaRequerida` fuera de ella y los cuatro rechazos de FR-12
//! disparables a voluntad. Existe para que el motor de mensajería de `crates/hexcell` y sus tests
//! tengan un adaptador determinista sin depender de un canal real; la primera implementación real
//! es whatsmeow, ya cerrada en la etapa A-3.
//!
//! Vive fuera de `hexcell-core` a propósito: una implementación de `ChannelAdapter` no es un tipo
//! del dominio, es infraestructura, y `hexcell-core` se mantiene libre de cualquier dependencia de
//! infraestructura (sin tokio, sin async runtime, sin HTTP) como criterio de aceptación de esta
//! misma tarea.
//!
//! Que este adaptador **imponga** la ventana de 24 horas no dice nada sobre lo que debe hacer el
//! adaptador del canal propio: la distinción que gobierna FR-12 es que **el TIPO admite el
//! resultado restrictivo; la POLÍTICA de cada adaptador decide si lo produce**. Este adaptador
//! decide producirlo porque su función es imitar el caso más restrictivo para que el motor y los
//! futuros tests de contrato (etapa A-2, HEX-005) lo ejerciten; el canal propio no tiene por qué
//! imponer nada parecido.

pub mod adaptador;
pub mod reloj;

pub use adaptador::{AdaptadorSimulado, ErrorDelAdaptadorSimulado};
pub use reloj::{Reloj, RelojDePrueba, RelojDelSistema};
