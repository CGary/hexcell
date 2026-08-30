//! Pruebas unitarias de la función de similitud coseno.
//!
//! Estos escenarios validan el comportamiento matemático puro del cálculo
//! de similitud entre perfiles de características vectoriales, asegurando que
//! se manejen correctamente los vectores idénticos, ortogonales, opuestos
//! y los casos de error (longitudes distintas y magnitudes nulas).

use hexcell_core::similitud::similitud_coseno;

/// Tolerancia permitida para comparaciones de punto flotante debido a
/// la imprecisión inherente al acumular valores f32 representados en f64.
const EPSILON: f32 = 1e-6;

#[test]
fn verificar_vectores_identicos() {
    let a = vec![1.0, 2.0, 3.0];
    let b = vec![1.0, 2.0, 3.0];
    let resultado = similitud_coseno(&a, &b).expect("Debe calcular similitud");
    assert!(
        (resultado - 1.0).abs() < EPSILON,
        "Vectores identicos deben retornar 1.0"
    );
}

#[test]
fn verificar_vectores_ortogonales() {
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    let resultado = similitud_coseno(&a, &b).expect("Debe calcular similitud");
    assert!(
        resultado.abs() < EPSILON,
        "Vectores ortogonales deben retornar 0.0"
    );
}

#[test]
fn verificar_vectores_opuestos() {
    let a = vec![1.0, -1.0];
    let b = vec![-1.0, 1.0];
    let resultado = similitud_coseno(&a, &b).expect("Debe calcular similitud");
    assert!(
        (resultado - (-1.0)).abs() < EPSILON,
        "Vectores opuestos deben retornar -1.0"
    );
}

#[test]
fn verificar_longitudes_distintas_retorna_none() {
    let a = vec![1.0, 2.0];
    let b = vec![1.0, 2.0, 3.0];
    let resultado = similitud_coseno(&a, &b);
    assert!(
        resultado.is_none(),
        "Diferentes longitudes deben retornar None"
    );
}

#[test]
fn verificar_magnitud_nula_retorna_none() {
    let a = vec![0.0, 0.0];
    let b = vec![1.0, 2.0];

    let resultado_a = similitud_coseno(&a, &b);
    assert!(
        resultado_a.is_none(),
        "Vector origen nulo debe retornar None"
    );

    let resultado_b = similitud_coseno(&b, &a);
    assert!(
        resultado_b.is_none(),
        "Vector destino nulo debe retornar None"
    );
}

#[test]
fn verificar_componente_nan_retorna_none() {
    let a = vec![1.0, f32::NAN, 3.0];
    let b = vec![1.0, 2.0, 3.0];

    let resultado_a = similitud_coseno(&a, &b);
    assert!(
        resultado_a.is_none(),
        "Un NaN en el primer vector nunca debe producir una puntuacion numerica"
    );

    let resultado_b = similitud_coseno(&b, &a);
    assert!(
        resultado_b.is_none(),
        "Un NaN en el segundo vector nunca debe producir una puntuacion numerica"
    );
}

#[test]
fn verificar_componente_infinito_retorna_none() {
    let a = vec![f32::INFINITY, 2.0, 3.0];
    let b = vec![1.0, 2.0, 3.0];

    let resultado_a = similitud_coseno(&a, &b);
    assert!(
        resultado_a.is_none(),
        "Un infinito en el primer vector nunca debe producir una puntuacion numerica"
    );

    let c = vec![f32::NEG_INFINITY, 2.0, 3.0];
    let resultado_c = similitud_coseno(&b, &c);
    assert!(
        resultado_c.is_none(),
        "Un infinito negativo en el segundo vector nunca debe producir una puntuacion numerica"
    );
}

#[test]
fn verificar_mezcla_de_nan_infinito_y_valores_finitos_retorna_none() {
    let mezclado = vec![1.0, f32::NAN, f32::INFINITY, 4.0];
    let finito = vec![1.0, 2.0, 3.0, 4.0];

    let resultado = similitud_coseno(&mezclado, &finito);
    assert!(
        resultado.is_none(),
        "Una mezcla de componentes corruptos junto a valores finitos nunca debe escapar como puntuacion"
    );
}
