//! Pruebas del agregado de estado de célula del plano de control.
//!
//! Viven en `tests/`, es decir, en un crate externo que solo ve la API pública de
//! `hexcell-admin`. La ubicación es deliberada, siguiendo el precedente de
//! `crates/hexcell-core/tests/exhaustividad_resultado_envio.rs`: como `EstadoDeCelula` y
//! `TransicionInvalida` se declaran cerrados, una consumidora externa los sigue viendo
//! exhaustivos y puede recorrerlos sin un brazo por defecto. Esa es la propiedad fuerte que aquí
//! se fija por escrito.
//!
//! Ningún `match` de este archivo tiene un brazo comodín ni un patrón de resto: añadir o quitar
//! una variante de `EstadoDeCelula` debe romper la compilación de estas pruebas, no solo la del
//! crate de producción.

use std::collections::HashSet;

use hexcell_admin::estado_de_celula::{CicloDeVidaDeCelula, EstadoDeCelula, TransicionInvalida};

/// Copia local, declarada fuera del crate de producción, de los cinco estados esperados.
const TODOS_LOS_ESTADOS: [EstadoDeCelula; 5] = [
    EstadoDeCelula::Aprovisionada,
    EstadoDeCelula::EnEjecucion,
    EstadoDeCelula::Suspendida,
    EstadoDeCelula::Reemparejando,
    EstadoDeCelula::Retirada,
];

/// Etiqueta local por estado, con una coincidencia exhaustiva y sin brazo por defecto: si se
/// añade una sexta variante a `EstadoDeCelula` sin tocar este archivo, esta función deja de
/// compilar antes de que corra ninguna aserción.
fn etiqueta(estado: EstadoDeCelula) -> &'static str {
    match estado {
        EstadoDeCelula::Aprovisionada => "aprovisionada",
        EstadoDeCelula::EnEjecucion => "en ejecución",
        EstadoDeCelula::Suspendida => "suspendida",
        EstadoDeCelula::Reemparejando => "reemparejando",
        EstadoDeCelula::Retirada => "retirada",
    }
}

/// Los once pares ordenados legales, fijados por el plano de la etapa A-6 y repetidos aquí como
/// tabla independiente de verificación (no derivada de la función de producción).
fn pares_legales() -> Vec<(EstadoDeCelula, EstadoDeCelula)> {
    vec![
        (EstadoDeCelula::Aprovisionada, EstadoDeCelula::EnEjecucion),
        (EstadoDeCelula::Aprovisionada, EstadoDeCelula::Retirada),
        (EstadoDeCelula::EnEjecucion, EstadoDeCelula::Suspendida),
        (EstadoDeCelula::EnEjecucion, EstadoDeCelula::Reemparejando),
        (EstadoDeCelula::EnEjecucion, EstadoDeCelula::Retirada),
        (EstadoDeCelula::Suspendida, EstadoDeCelula::EnEjecucion),
        (EstadoDeCelula::Suspendida, EstadoDeCelula::Reemparejando),
        (EstadoDeCelula::Suspendida, EstadoDeCelula::Retirada),
        (EstadoDeCelula::Reemparejando, EstadoDeCelula::EnEjecucion),
        (EstadoDeCelula::Reemparejando, EstadoDeCelula::Suspendida),
        (EstadoDeCelula::Reemparejando, EstadoDeCelula::Retirada),
    ]
}

#[test]
fn el_estado_de_celula_tiene_exactamente_cinco_variantes() {
    assert_eq!(TODOS_LOS_ESTADOS.len(), 5);
    assert_eq!(EstadoDeCelula::TODOS.len(), 5);
}

#[test]
fn cada_variante_local_tiene_una_etiqueta_distinta() {
    let mut etiquetas = HashSet::new();
    for estado in TODOS_LOS_ESTADOS {
        assert!(
            etiquetas.insert(etiqueta(estado)),
            "dos estados comparten etiqueta: {estado:?}"
        );
    }
    assert_eq!(etiquetas.len(), TODOS_LOS_ESTADOS.len());
}

#[test]
fn todos_los_estados_del_agregado_estan_en_la_constante_publica() {
    let conjunto_local: HashSet<EstadoDeCelula> = TODOS_LOS_ESTADOS.into_iter().collect();
    let conjunto_publico: HashSet<EstadoDeCelula> = EstadoDeCelula::TODOS.into_iter().collect();
    assert_eq!(conjunto_local, conjunto_publico);
    assert_eq!(conjunto_publico.len(), 5, "TODOS no puede tener duplicados");
}

#[test]
fn todo_estado_que_aparece_en_una_lista_de_transiciones_es_miembro_de_todos() {
    let conjunto_publico: HashSet<EstadoDeCelula> = EstadoDeCelula::TODOS.into_iter().collect();
    for origen in EstadoDeCelula::TODOS {
        for destino in origen.transiciones_permitidas() {
            assert!(
                conjunto_publico.contains(destino),
                "{destino:?} aparece como destino desde {origen:?} pero no está en TODOS"
            );
        }
    }
}

#[test]
fn los_veinticinco_pares_ordenados_se_resuelven_segun_la_tabla_independiente() {
    let legales: HashSet<(EstadoDeCelula, EstadoDeCelula)> = pares_legales().into_iter().collect();
    assert_eq!(
        legales.len(),
        11,
        "la tabla de referencia debe tener once pares legales"
    );

    let mut vistos_legales = 0usize;
    let mut vistos_ilegales = 0usize;

    for origen in EstadoDeCelula::TODOS {
        for destino in EstadoDeCelula::TODOS {
            let deberia_ser_legal = legales.contains(&(origen, destino));
            let resultado = origen.transitar(destino);
            if deberia_ser_legal {
                vistos_legales += 1;
                assert_eq!(
                    resultado,
                    Ok(destino),
                    "se esperaba que {origen:?} -> {destino:?} fuera aceptado"
                );
                assert!(origen.permite(destino));
            } else {
                vistos_ilegales += 1;
                assert!(
                    resultado.is_err(),
                    "se esperaba que {origen:?} -> {destino:?} fuera rechazado"
                );
                assert!(!origen.permite(destino));
            }
        }
    }

    assert_eq!(vistos_legales, 11);
    assert_eq!(vistos_ilegales, 25 - 11);
}

#[test]
fn las_cinco_transiciones_identicas_estan_entre_las_rechazadas() {
    for estado in EstadoDeCelula::TODOS {
        assert!(
            !estado.permite(estado),
            "la transición de {estado:?} a sí mismo debe estar ausente de la tabla"
        );
    }
}

#[test]
fn el_rechazo_desde_un_origen_no_terminal_es_par_no_permitido_con_el_par_exacto() {
    let resultado = EstadoDeCelula::Aprovisionada.transitar(EstadoDeCelula::Suspendida);
    assert_eq!(
        resultado,
        Err(TransicionInvalida::ParNoPermitido {
            desde: EstadoDeCelula::Aprovisionada,
            hacia: EstadoDeCelula::Suspendida,
        })
    );
    let error = resultado.unwrap_err();
    assert_eq!(error.desde(), EstadoDeCelula::Aprovisionada);
    assert_eq!(error.hacia(), EstadoDeCelula::Suspendida);
}

#[test]
fn el_rechazo_desde_retirada_es_origen_terminal_con_el_par_exacto() {
    let resultado = EstadoDeCelula::Retirada.transitar(EstadoDeCelula::EnEjecucion);
    assert_eq!(
        resultado,
        Err(TransicionInvalida::OrigenTerminal {
            desde: EstadoDeCelula::Retirada,
            hacia: EstadoDeCelula::EnEjecucion,
        })
    );
    let error = resultado.unwrap_err();
    assert_eq!(error.desde(), EstadoDeCelula::Retirada);
    assert_eq!(error.hacia(), EstadoDeCelula::EnEjecucion);
}

#[test]
fn retirada_es_el_unico_estado_terminal() {
    for estado in EstadoDeCelula::TODOS {
        let esperado_terminal = estado == EstadoDeCelula::Retirada;
        assert_eq!(estado.es_terminal(), esperado_terminal, "{estado:?}");
    }
    assert!(
        EstadoDeCelula::Retirada
            .transiciones_permitidas()
            .is_empty()
    );
}

#[test]
fn una_celula_nueva_empieza_aprovisionada() {
    let celula = CicloDeVidaDeCelula::nueva();
    assert_eq!(celula.estado(), EstadoDeCelula::Aprovisionada);
    assert!(!celula.es_terminal());
}

#[test]
fn aplicar_con_destino_legal_avanza_el_estado() {
    let mut celula = CicloDeVidaDeCelula::nueva();
    let resultado = celula.aplicar(EstadoDeCelula::EnEjecucion);
    assert_eq!(resultado, Ok(()));
    assert_eq!(celula.estado(), EstadoDeCelula::EnEjecucion);
}

#[test]
fn aplicar_con_destino_ilegal_deja_el_estado_sin_cambios() {
    let mut celula = CicloDeVidaDeCelula::nueva();
    let resultado = celula.aplicar(EstadoDeCelula::Suspendida);
    assert!(resultado.is_err());
    assert_eq!(
        celula.estado(),
        EstadoDeCelula::Aprovisionada,
        "un aplicar rechazado no puede haber escrito el estado antes de validar"
    );
}

#[test]
fn aplicar_hasta_retirada_deja_la_celula_terminal_y_sin_mas_transiciones() {
    let mut celula = CicloDeVidaDeCelula::nueva();
    celula.aplicar(EstadoDeCelula::EnEjecucion).unwrap();
    celula.aplicar(EstadoDeCelula::Retirada).unwrap();
    assert!(celula.es_terminal());
    assert!(celula.aplicar(EstadoDeCelula::EnEjecucion).is_err());
    assert_eq!(celula.estado(), EstadoDeCelula::Retirada);
}

/// Guarda de exploración del código fuente de producción: busca tokens que las invariantes de
/// la tarea prohíben. Es la única prueba mecánica de las invariantes de ausencia de comodín,
/// ausencia de `serde` y ausencia de camino de pánico, que `clippy` no exige por sí solo.
#[test]
fn el_modulo_de_produccion_no_contiene_tokens_prohibidos() {
    let fuente = include_str!("../src/estado_de_celula.rs");
    let prohibidos = [
        "crate::docker",
        "ClienteDocker",
        "ConexionDocker",
        "UnixStream",
        "Serialize",
        "Deserialize",
        "rusqlite",
        "panic!",
        "unreachable!",
        ".unwrap()",
        ".expect(",
        "debug_assert",
        "_ =>",
    ];
    for token in prohibidos {
        assert!(
            !fuente.contains(token),
            "el módulo de producción contiene el token prohibido: {token}"
        );
    }
}

/// Guarda de dependencias: el `Cargo.toml` del crate sigue declarando exactamente `serde` y
/// `serde_json`, sin ningún analizador de argumentos externo ni `anyhow`.
#[test]
fn el_cargo_toml_no_gano_ninguna_dependencia_nueva() {
    let manifiesto = include_str!("../Cargo.toml");
    let prohibidos = ["clap", "argh", "pico-args", "structopt", "lexopt", "anyhow"];
    for token in prohibidos {
        assert!(
            !manifiesto.contains(token),
            "el manifiesto contiene una dependencia prohibida: {token}"
        );
    }
    assert!(manifiesto.contains("serde"));
    assert!(manifiesto.contains("serde_json"));
}
