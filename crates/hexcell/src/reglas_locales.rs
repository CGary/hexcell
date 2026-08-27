//! Reglas locales para el modo degradado del procesador de inferencia.
//!
//! Este módulo contiene el mecanismo mínimo de reglas locales (FR-10, etapa A-4, tarea 10)
//! para responder cuando el presupuesto se ha agotado. Cabe destacar que este mecanismo
//! genera una respuesta provisional y no constituye un catálogo de respuestas comerciales,
//! cuya definición queda pendiente de una decisión de producto.

use hexcell_core::inferencia::RespuestaDeInferencia;

/// Texto de la respuesta degradada provisional de inferencia.
pub const TEXTO_DE_RESPUESTA_DEGRADADA: &str = "[modo degradado] Sin saldo de inferencia disponible en este momento. Texto provisional del mecanismo, pendiente de decisión de producto.";

/// Genera una respuesta local para el modo degradado con consumo de presupuesto cero.
///
/// Ignora cualquier entrada y devuelve de forma determinista la respuesta provisional
/// marcada como modo degradado, con cero unidades consumidas.
pub fn responder_localmente() -> RespuestaDeInferencia {
    RespuestaDeInferencia {
        contenido: TEXTO_DE_RESPUESTA_DEGRADADA.to_string(),
        unidades_consumidas: 0,
    }
}
