//! Módulo para el cálculo de similitud entre vectores de características.
//!
//! Se diseña como una utilidad pura sobre porciones de memoria estándar sin
//! dependencias de infraestructura ni crates de cálculo matricial complejos,
//! respetando el límite de dependencias vacías del núcleo (adr-0002).

/// Calcula la similitud coseno entre dos vectores numéricos de punto flotante.
///
/// # Razón de diseño
/// El cálculo de la magnitud y el producto escalar se realiza internamente en `f64`
/// porque la acumulación de errores de redondeo sobre cientos de dimensiones (como
/// las 768 requeridas en esta fase) puede desviar el resultado final de los límites
/// teóricos de [-1, 1]. El resultado se acota explícitamente mediante `clamp` antes
/// de convertirse de vuelta a `f32` para absorber cualquier residuo numérico y
/// asegurar la consistencia con las expectativas matemáticas.
///
/// # Casos especiales
/// Si los vectores tienen diferente longitud, o si la magnitud (norma) de alguno de
/// ellos es cero (lo que provocaría una división por cero), la función devuelve `None`.
/// Esto evita el uso de valores sentinela (como `NaN` o `0.0` por defecto) que podrían
/// interpretarse erróneamente como similitudes válidas por el llamador.
pub fn similitud_coseno(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }

    let mut producto_escalar: f64 = 0.0;
    let mut norma_a: f64 = 0.0;
    let mut norma_b: f64 = 0.0;

    for (val_a, val_b) in a.iter().zip(b.iter()) {
        let va = *val_a as f64;
        let vb = *val_b as f64;
        producto_escalar += va * vb;
        norma_a += va * va;
        norma_b += vb * vb;
    }

    // Si alguno de los vectores no tiene magnitud, la similitud coseno no está definida.
    if norma_a <= 0.0 || norma_b <= 0.0 {
        return None;
    }

    let magnitud_a = norma_a.sqrt();
    let magnitud_b = norma_b.sqrt();

    if magnitud_a == 0.0 || magnitud_b == 0.0 {
        return None;
    }

    let similitud = producto_escalar / (magnitud_a * magnitud_b);

    // Forzamos el resultado dentro de los límites matemáticos del coseno
    // para corregir posibles imprecisiones de coma flotante.
    Some(similitud.clamp(-1.0, 1.0) as f32)
}
