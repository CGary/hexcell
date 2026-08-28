//! Estimación de costes basada en la longitud del contenido del evento entrante.
//!
//! El coste estimado es una función pura y determinista basada en el conteo de caracteres
//! Unicode (`chars().count()`), acotada por un suelo mínimo de [`UNIDADES_MINIMAS_POR_LLAMADA`].

/// Unidades opacas de presupuesto. Sin ningún valor monetario, moneda, precio ni tarifa.
pub type UnidadesDePresupuesto = u64;

/// Número de caracteres Unicode por cada unidad estimada de presupuesto.
pub const CARACTERES_POR_UNIDAD_ESTIMADA: u64 = 4;

/// Suelo mínimo de unidades presupuestarias por llamada a la inferencia.
pub const UNIDADES_MINIMAS_POR_LLAMADA: UnidadesDePresupuesto = 1;

/// Calcula el coste estimado de una petición de inferencia a partir de la longitud del contenido.
///
/// La estimación se calcula dividiendo la cantidad de caracteres Unicode entre
/// [`CARACTERES_POR_UNIDAD_ESTIMADA`] y aplicando [`UNIDADES_MINIMAS_POR_LLAMADA`] como suelo mínimo.
pub fn estimar_coste(prompt: &str) -> UnidadesDePresupuesto {
    let num_caracteres = prompt.chars().count() as u64;
    let estimacion = num_caracteres / CARACTERES_POR_UNIDAD_ESTIMADA;
    estimacion.max(UNIDADES_MINIMAS_POR_LLAMADA)
}

/// Calcula el coste estimado de un lote de fragmentos de texto para una petición de incrustaciones.
///
/// Suma la cantidad total de caracteres Unicode de todos los textos del lote, divide entre
/// [`CARACTERES_POR_UNIDAD_ESTIMADA`] y aplica [`UNIDADES_MINIMAS_POR_LLAMADA`] como suelo único
/// para la llamada completa, evitando sobre-reservar en lotes con múltiples fragmentos cortos.
pub fn estimar_coste_de_lote(textos: &[String]) -> UnidadesDePresupuesto {
    let total_caracteres: u64 = textos.iter().map(|t| t.chars().count() as u64).sum();
    let estimacion = total_caracteres / CARACTERES_POR_UNIDAD_ESTIMADA;
    estimacion.max(UNIDADES_MINIMAS_POR_LLAMADA)
}
