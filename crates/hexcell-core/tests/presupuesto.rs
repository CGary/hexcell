//! Tests del estimador de costes determinista en hexcell-core (AC-3).

use hexcell_core::presupuesto::{
    CARACTERES_POR_UNIDAD_ESTIMADA, UNIDADES_MINIMAS_POR_LLAMADA, estimar_coste,
    estimar_coste_de_lote,
};

#[test]
fn estimacion_es_determinista_para_prompts_de_misma_longitud_de_caracteres() {
    let ascii = "abcd"; // 4 caracteres, 4 bytes
    let no_ascii = "ábéñ"; // 4 caracteres, 7 bytes

    let coste_ascii = estimar_coste(ascii);
    let coste_no_ascii = estimar_coste(no_ascii);

    assert_eq!(
        coste_ascii, coste_no_ascii,
        "prompts con igual cantidad de caracteres deben tener la misma estimación"
    );
    assert_eq!(coste_ascii, 1);
}

#[test]
fn estimacion_esta_acotada_por_el_suelo_minimo() {
    assert_eq!(
        estimar_coste(""),
        UNIDADES_MINIMAS_POR_LLAMADA,
        "un prompt vacío debe devolver al menos las unidades mínimas"
    );
    assert_eq!(
        estimar_coste("a"),
        UNIDADES_MINIMAS_POR_LLAMADA,
        "un prompt de 1 caracter debe devolver al menos las unidades mínimas"
    );
}

#[test]
fn estimacion_es_monotona_con_la_longitud() {
    let base = "a".repeat(CARACTERES_POR_UNIDAD_ESTIMADA as usize * 2);
    let mayor = "a".repeat(CARACTERES_POR_UNIDAD_ESTIMADA as usize * 4);

    assert_eq!(estimar_coste(&base), 2);
    assert_eq!(estimar_coste(&mayor), 4);
    assert!(estimar_coste(&mayor) > estimar_coste(&base));
}

#[test]
fn estimacion_de_lote_aplica_suelo_una_sola_vez_sobre_la_suma_total() {
    let lote_vacio: Vec<String> = Vec::new();
    assert_eq!(
        estimar_coste_de_lote(&lote_vacio),
        UNIDADES_MINIMAS_POR_LLAMADA,
        "un lote vacío aplica el suelo mínimo de una unidad"
    );

    // Varios fragmentos muy cortos: 4 fragmentos de 1 caracter = 4 caracteres
    let fragmentos_cortos = vec![
        "a".to_string(),
        "b".to_string(),
        "c".to_string(),
        "d".to_string(),
    ];
    // Individualmente cada uno estimaría 1 unidad (total 4 unidades con suelo per-text)
    // Pero en lote es (1+1+1+1)/4 = 1 unidad (suelo per-call)
    assert_eq!(
        estimar_coste_de_lote(&fragmentos_cortos),
        1,
        "el lote debe sumar caracteres y aplicar suelo una sola vez"
    );

    let lote_largo = vec![
        "a".repeat(CARACTERES_POR_UNIDAD_ESTIMADA as usize * 3),
        "b".repeat(CARACTERES_POR_UNIDAD_ESTIMADA as usize * 5),
    ];
    // (12 + 20) / 4 = 8 unidades
    assert_eq!(estimar_coste_de_lote(&lote_largo), 8);
}
