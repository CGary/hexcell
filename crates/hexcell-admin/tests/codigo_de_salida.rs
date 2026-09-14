//! Pruebas del contrato tipado de códigos de salida `CodigoDeSalida`.
//!
//! Viven en `tests/`, es decir, en un crate externo que solo ve la API pública de
//! `hexcell-admin`. La ubicación es deliberada, siguiendo el precedente de
//! `tests/estado_de_celula.rs`: como `CodigoDeSalida` se declara cerrado (sin
//! `#[non_exhaustive]`), una consumidora externa lo sigue viendo exhaustivo y puede recorrerlo sin
//! un brazo por defecto. Esa es la propiedad fuerte que aquí se fija por escrito.
//!
//! Ningún `match` de este archivo tiene un brazo comodín ni un patrón de resto: añadir o quitar
//! una variante de `CodigoDeSalida` debe romper la compilación de estas pruebas, no solo la del
//! crate de producción.

use std::collections::HashSet;
use std::process::ExitCode;

use hexcell_admin::codigo_de_salida::CodigoDeSalida;

/// Copia local, declarada fuera del crate de producción, de las cuatro variantes esperadas.
const TODOS_LOS_CODIGOS: [CodigoDeSalida; 4] = [
    CodigoDeSalida::Exito,
    CodigoDeSalida::Fallo,
    CodigoDeSalida::UsoIncorrecto,
    CodigoDeSalida::NoImplementadoTodavia,
];

/// Etiqueta local por variante, con una coincidencia exhaustiva y sin brazo por defecto: si se
/// añade una quinta variante a `CodigoDeSalida` sin tocar este archivo, esta función deja de
/// compilar antes de que corra ninguna aserción.
fn etiqueta(codigo: CodigoDeSalida) -> &'static str {
    match codigo {
        CodigoDeSalida::Exito => "éxito",
        CodigoDeSalida::Fallo => "fallo",
        CodigoDeSalida::UsoIncorrecto => "uso incorrecto",
        CodigoDeSalida::NoImplementadoTodavia => "no implementado todavía",
    }
}

#[test]
fn exito_vale_cero() {
    assert_eq!(CodigoDeSalida::Exito.codigo(), 0);
}

#[test]
fn fallo_vale_uno() {
    assert_eq!(CodigoDeSalida::Fallo.codigo(), 1);
}

#[test]
fn uso_incorrecto_vale_dos() {
    assert_eq!(CodigoDeSalida::UsoIncorrecto.codigo(), 2);
}

#[test]
fn no_implementado_todavia_vale_tres() {
    assert_eq!(CodigoDeSalida::NoImplementadoTodavia.codigo(), 3);
}

#[test]
fn los_cuatro_valores_numericos_no_se_solapan() {
    let valores: HashSet<u8> = TODOS_LOS_CODIGOS.iter().map(|c| c.codigo()).collect();
    assert_eq!(
        valores.len(),
        TODOS_LOS_CODIGOS.len(),
        "dos variantes comparten el mismo valor numérico"
    );
}

#[test]
fn cada_variante_local_tiene_una_etiqueta_distinta() {
    let mut etiquetas = HashSet::new();
    for codigo in TODOS_LOS_CODIGOS {
        assert!(
            etiquetas.insert(etiqueta(codigo)),
            "dos códigos comparten etiqueta: {codigo:?}"
        );
    }
    assert_eq!(etiquetas.len(), TODOS_LOS_CODIGOS.len());
}

#[test]
fn exito_convierte_al_exit_code_de_exito_estandar() {
    let convertido: ExitCode = ExitCode::from(CodigoDeSalida::Exito);
    assert_eq!(convertido, ExitCode::SUCCESS);
}

#[test]
fn cada_variante_convierte_a_traves_de_exit_code_from_u8() {
    for codigo in TODOS_LOS_CODIGOS {
        let convertido: ExitCode = ExitCode::from(codigo);
        let esperado = ExitCode::from(codigo.codigo());
        assert_eq!(
            convertido, esperado,
            "la conversión de {codigo:?} no coincide con ExitCode::from(u8)"
        );
    }
}

#[test]
fn no_implementado_todavia_es_distinto_de_exito_y_de_fallo() {
    assert_ne!(
        CodigoDeSalida::NoImplementadoTodavia.codigo(),
        CodigoDeSalida::Exito.codigo()
    );
    assert_ne!(
        CodigoDeSalida::NoImplementadoTodavia.codigo(),
        CodigoDeSalida::Fallo.codigo()
    );
}

#[test]
fn uso_incorrecto_es_distinto_de_fallo_y_de_exito() {
    assert_ne!(
        CodigoDeSalida::UsoIncorrecto.codigo(),
        CodigoDeSalida::Fallo.codigo()
    );
    assert_ne!(
        CodigoDeSalida::UsoIncorrecto.codigo(),
        CodigoDeSalida::Exito.codigo()
    );
}
