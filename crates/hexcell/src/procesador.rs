//! Procesador de mensajes: punto de extensión del motor, sin ninguna regla de producto.
//!
//! El motor de mensajería (`crate::motor`) despacha cada evento entrante a una implementación de
//! [`ProcesadorDeMensajes`] y envía lo que esta devuelva. Esta tarea solo aporta
//! [`ProcesadorDeEco`], que devuelve el mismo contenido recibido: no hay lógica de negocio que
//! implementar todavía, y adelantarla aquí sería escribir producto antes de que exista el
//! requisito que lo justifique. El procesador real llega en etapas posteriores del plan.
//!
//! El método es deliberadamente síncrono. La interfaz del proveedor de inferencia es una tarea
//! posterior del plan (etapa A-2, más adelante), y declarar hoy `-> impl Future<Output = ...>` sin
//! tener todavía nada asíncrono que ejecutar sería generalidad especulativa: se añadirá cuando el
//! procesador real la necesite, no antes.

use hexcell_core::canal::{EventoEntrante, MensajeSaliente};

/// Puerto del procesador de mensajes, local a este binario.
///
/// No es un trait del dominio (`hexcell-core`), porque cómo se decide una respuesta es una
/// política de la célula, no un tipo canónico de FR-12.
pub trait ProcesadorDeMensajes {
    /// Decide qué responder, si algo, ante un evento entrante ya normalizado por el adaptador.
    ///
    /// Devolver `None` significa que este evento no genera respuesta; el motor simplemente no
    /// llama a `send` en ese caso.
    fn procesar(&self, evento: &EventoEntrante) -> Option<MensajeSaliente>;
}

/// Procesador mínimo de eco: repite el contenido del evento entrante como respuesta libre.
///
/// Es el único procesador de esta tarea. No decide nada sobre el negocio: ni interpreta el
/// contenido, ni consulta ningún catálogo, ni invoca ningún proveedor externo. Sirve para que el
/// motor y su prueba de extremo a extremo local tengan algo determinista que despachar.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcesadorDeEco;

impl ProcesadorDeMensajes for ProcesadorDeEco {
    fn procesar(&self, evento: &EventoEntrante) -> Option<MensajeSaliente> {
        Some(MensajeSaliente::RespuestaLibre(evento.contenido.clone()))
    }
}
