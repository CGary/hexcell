//! Tests del estimador de costes determinista en hexcell-core (AC-3).

use hexcell_core::presupuesto::{
    CARACTERES_POR_UNIDAD_ESTIMADA, UNIDADES_MINIMAS_POR_LLAMADA, estimar_coste,
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
