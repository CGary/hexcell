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
///
/// Un componente corrupto (`NaN` o infinito) en cualquiera de los dos vectores también
/// devuelve `None`: sin esta comprobación explícita, `NaN` atraviesa silenciosamente cada
/// comparación de esta función (toda comparación con `NaN` es falsa) y `clamp` no lo
/// corrige, porque `clamp` sobre un `NaN` devuelve el mismo `NaN`. Dejar pasar ese valor
/// sería exactamente el sentinela indetectable que el párrafo anterior promete evitar.
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

    // Un componente NaN o infinito en la entrada arrastra su corrupción hasta aquí:
    // la acumulación con un NaN produce un NaN, y con un infinito produce un infinito
    // o un NaN (infinito menos infinito). Cortamos aquí porque ninguna comparación
    // posterior con `<=` o `==` puede detectar un NaN: toda comparación con NaN es falsa.
    if !producto_escalar.is_finite() || !norma_a.is_finite() || !norma_b.is_finite() {
        return None;
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

    // Segunda barrera tras la división: aunque las normas fuesen finitas, el cociente
    // podría dejar de serlo (por ejemplo, si una magnitud fuese un valor extremo cercano
    // al límite superior de f64). `clamp` no repara un NaN, así que lo rechazamos antes
    // de acotar el resultado a los límites matemáticos del coseno.
    if !similitud.is_finite() {
        return None;
    }

    // Forzamos el resultado dentro de los límites matemáticos del coseno
    // para corregir posibles imprecisiones de coma flotante.
    Some(similitud.clamp(-1.0, 1.0) as f32)
}
