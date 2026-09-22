//! Pruebas externas del analizador de argumentos `argumentos::analizar`.
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`. El analizador es una
//! función pura sobre una porción de argumentos, así que las pruebas lo ejercitan con un
//! `Vec<String>` propio sin tocar `std::env::args`. Ningún `match` sobre `Subcomando`
//! tiene brazo comodín.

use hexcell_admin::argumentos::{Comando, ErrorDeArgumentos, Subcomando, analizar};

fn args(snippet: &[&str]) -> Vec<String> {
    snippet.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn ausencia_de_argumentos_y_grupo() {
    assert_eq!(analizar(&[]).unwrap_err(), ErrorDeArgumentos::SinSubcomando);
    assert_eq!(
        analizar(&args(&["cell"])).unwrap_err(),
        ErrorDeArgumentos::SinSubcomando
    );
    assert_eq!(
        analizar(&args(&["server"])).unwrap_err(),
        ErrorDeArgumentos::GrupoDesconocido {
            grupo: "server".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "restart"])).unwrap_err(),
        ErrorDeArgumentos::SubcomandoDesconocido {
            nombre: "restart".to_string()
        }
    );
}

/// Recorrido exhaustivo de los seis nombres declarados: coincidencia sin brazo por
/// defecto, de modo que renombrar o quitar un nombre deja de compilar.
fn subcomando_para(nombre: &str) -> Subcomando {
    let invocacion = match nombre {
        "pause" => analizar(&args(&["cell", "pause", "--id", "c1"])).unwrap(),
        "unpause" => analizar(&args(&["cell", "unpause", "--id", "c1"])).unwrap(),
        "terminate" => {
            analizar(&args(&["cell", "terminate", "--id", "c1", "--confirmar"])).unwrap()
        }
        "rebind" => analizar(&args(&[
            "cell",
            "rebind",
            "--id",
            "c1",
            "--motivo",
            "baneo",
            "--confirmar",
        ]))
        .unwrap(),
        "list" => analizar(&args(&["cell", "list"])).unwrap(),
        "status" => analizar(&args(&["cell", "status", "--id", "c1"])).unwrap(),
        otro => panic!("nombre no reconocido: {otro}"),
    };
    match invocacion.subcomando().unwrap() {
        Subcomando::Pausar => Subcomando::Pausar,
        Subcomando::Reanudar => Subcomando::Reanudar,
        Subcomando::Retirar => Subcomando::Retirar,
        Subcomando::Reemparejar => Subcomando::Reemparejar,
        Subcomando::Listar => Subcomando::Listar,
        Subcomando::Estado => Subcomando::Estado,
    }
}

#[test]
fn los_seis_nombres_analizan_a_su_propia_variante() {
    assert_eq!(subcomando_para("pause"), Subcomando::Pausar);
    assert_eq!(subcomando_para("unpause"), Subcomando::Reanudar);
    assert_eq!(subcomando_para("terminate"), Subcomando::Retirar);
    assert_eq!(subcomando_para("rebind"), Subcomando::Reemparejar);
    assert_eq!(subcomando_para("list"), Subcomando::Listar);
    assert_eq!(subcomando_para("status"), Subcomando::Estado);
}

#[test]
fn opciones_desconocidas_repetidas_y_con_valor_faltante() {
    assert_eq!(
        analizar(&args(&["cell", "list", "--foo"])).unwrap_err(),
        ErrorDeArgumentos::OpcionDesconocida {
            subcomando: Subcomando::Listar,
            opcion: "--foo".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "list", "--simular", "--simular"])).unwrap_err(),
        ErrorDeArgumentos::OpcionRepetida {
            subcomando: Subcomando::Listar,
            opcion: "--simular".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "pause", "--id"])).unwrap_err(),
        ErrorDeArgumentos::FaltaValorDeOpcion {
            subcomando: Subcomando::Pausar,
            opcion: "--id".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "pause", "--id="])).unwrap_err(),
        ErrorDeArgumentos::FaltaValorDeOpcion {
            subcomando: Subcomando::Pausar,
            opcion: "--id".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "rebind", "--id", "c1", "--motivo"])).unwrap_err(),
        ErrorDeArgumentos::FaltaValorDeOpcion {
            subcomando: Subcomando::Reemparejar,
            opcion: "--motivo".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "rebind", "--id", "c1", "--motivo="])).unwrap_err(),
        ErrorDeArgumentos::FaltaValorDeOpcion {
            subcomando: Subcomando::Reemparejar,
            opcion: "--motivo".to_string()
        }
    );
}

#[test]
fn opciones_obligatorias_ausentes() {
    assert_eq!(
        analizar(&args(&["cell", "pause"])).unwrap_err(),
        ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando: Subcomando::Pausar,
            opcion: "--id".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "unpause"])).unwrap_err(),
        ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando: Subcomando::Reanudar,
            opcion: "--id".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "status"])).unwrap_err(),
        ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando: Subcomando::Estado,
            opcion: "--id".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "terminate", "--id", "c1"])).unwrap_err(),
        ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando: Subcomando::Retirar,
            opcion: "--confirmar".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "rebind", "--id", "c1", "--confirmar"])).unwrap_err(),
        ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando: Subcomando::Reemparejar,
            opcion: "--motivo".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "rebind", "--id", "c1", "--motivo", "x"])).unwrap_err(),
        ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando: Subcomando::Reemparejar,
            opcion: "--confirmar".to_string()
        }
    );
}

#[test]
fn opciones_no_admitidas_por_el_subcomando() {
    assert_eq!(
        analizar(&args(&["cell", "list", "--id", "c1"])).unwrap_err(),
        ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando: Subcomando::Listar,
            opcion: "--id".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "pause", "--id", "c1", "--motivo", "x"])).unwrap_err(),
        ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando: Subcomando::Pausar,
            opcion: "--motivo".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "list", "--confirmar"])).unwrap_err(),
        ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando: Subcomando::Listar,
            opcion: "--confirmar".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "pause", "--id", "c1", "--confirmar"])).unwrap_err(),
        ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando: Subcomando::Pausar,
            opcion: "--confirmar".to_string()
        }
    );
    assert_eq!(
        analizar(&args(&["cell", "list", "extra"])).unwrap_err(),
        ErrorDeArgumentos::ArgumentoPosicionalSobrante {
            subcomando: Subcomando::Listar,
            argumento: "extra".to_string()
        }
    );
}

#[test]
fn ambas_ortografias_y_validas_completas() {
    let i1 = analizar(&args(&["cell", "pause", "--id=c1"])).unwrap();
    assert_eq!(i1.id(), Some("c1"));
    let i2 = analizar(&args(&[
        "cell",
        "rebind",
        "--id=c1",
        "--motivo=baneo",
        "--confirmar",
    ]))
    .unwrap();
    assert_eq!(i2.id(), Some("c1"));
    assert_eq!(i2.motivo(), Some("baneo"));
    assert!(i2.confirmar());

    let p = analizar(&args(&["cell", "pause", "--id", "c1", "--simular"])).unwrap();
    assert_eq!(p.subcomando(), Some(Subcomando::Pausar));
    assert_eq!(p.id(), Some("c1"));
    assert!(p.simular());
    assert!(!p.confirmar());

    let r = analizar(&args(&[
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "baneo",
        "--confirmar",
        "--simular",
    ]))
    .unwrap();
    assert_eq!(r.subcomando(), Some(Subcomando::Reemparejar));
    assert_eq!(r.motivo(), Some("baneo"));
    assert!(r.simular());

    let l = analizar(&args(&["cell", "list"])).unwrap();
    assert_eq!(l.subcomando(), Some(Subcomando::Listar));
    assert_eq!(l.id(), None);
    assert!(!l.simular());
    assert!(!l.confirmar());
}

#[test]
fn mensajes_de_error_son_literales_en_espanol() {
    assert_eq!(
        ErrorDeArgumentos::SinSubcomando.to_string(),
        "falta el subcomando: se esperaba «cell <subcomando>»"
    );
    assert_eq!(
        ErrorDeArgumentos::GrupoDesconocido {
            grupo: "server".to_string()
        }
        .to_string(),
        "grupo desconocido: «server» (los grupos admitidos son «cell», «config» y «reporte»)"
    );
    assert_eq!(
        ErrorDeArgumentos::SubcomandoDesconocido {
            nombre: "restart".to_string()
        }
        .to_string(),
        "subcomando desconocido: «restart» (subcomandos admitidos: pause, unpause, terminate, \
         rebind, list, status)"
    );
}

#[test]
fn config_render_exige_las_tres_opciones_obligatorias() {
    assert!(
        analizar(&args(&[
            "config",
            "render",
            "--superposicion",
            "s",
            "--salida",
            "o"
        ]))
        .is_err(),
        "sin --defecto debe rechazarse"
    );
    assert!(
        analizar(&args(&[
            "config",
            "render",
            "--defecto",
            "d",
            "--salida",
            "o"
        ]))
        .is_err(),
        "sin --superposicion debe rechazarse"
    );
    assert!(
        analizar(&args(&[
            "config",
            "render",
            "--defecto",
            "d",
            "--superposicion",
            "s"
        ]))
        .is_err(),
        "sin --salida debe rechazarse"
    );
}

#[test]
fn config_render_rechaza_opcion_repetida() {
    let resultado = analizar(&args(&[
        "config",
        "render",
        "--defecto",
        "d",
        "--defecto",
        "d2",
        "--superposicion",
        "s",
        "--salida",
        "o",
    ]));
    assert!(resultado.is_err());
}

#[test]
fn config_render_rechaza_opcion_desconocida() {
    let resultado = analizar(&args(&[
        "config",
        "render",
        "--defecto",
        "d",
        "--superposicion",
        "s",
        "--salida",
        "o",
        "--no-existe",
    ]));
    assert!(resultado.is_err());
}

#[test]
fn config_render_rechaza_valor_inline_vacio() {
    let resultado = analizar(&args(&[
        "config",
        "render",
        "--defecto=",
        "--superposicion",
        "s",
        "--salida",
        "o",
    ]));
    assert!(resultado.is_err());
}

#[test]
fn config_render_valido_no_devuelve_subcomando_de_cell() {
    let comando = analizar(&args(&[
        "config",
        "render",
        "--defecto",
        "d",
        "--superposicion",
        "s",
        "--salida",
        "o",
    ]))
    .unwrap();
    assert_eq!(comando.subcomando(), None);
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Grupo `reporte tokens` (tarea 23 de la etapa A-6, HEX-084).
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Anclas de conversión independientes, comprobadas con `date -u` al implementar HEX-084:
/// el inicio de día en UTC de cada fecha, en milisegundos desde la época Unix.
const MS_1970_01_01: i64 = 0;
const MS_2000_02_29: i64 = 951_782_400_000;
const MS_2024_02_29: i64 = 1_709_164_800_000;
const MS_2026_02_28: i64 = 1_772_236_800_000;
const MS_2026_09_01: i64 = 1_788_220_800_000;
const MS_2026_09_10: i64 = 1_788_998_400_000;

fn invocacion_reporte(snippet: &[&str]) -> hexcell_admin::argumentos::InvocacionReporte {
    match analizar(&args(snippet)) {
        Ok(Comando::ReporteTokens(invocacion)) => invocacion,
        Ok(otro) => panic!("se esperaba ReporteTokens, llegó {otro:?}"),
        Err(error) => panic!("análisis fallido: {error}"),
    }
}

#[test]
fn reporte_tokens_valido_expone_sus_opciones() {
    let invocacion = invocacion_reporte(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        "copia.db",
        "--desde",
        "2026-09-01",
        "--hasta",
        "2026-09-10",
        "--simular",
    ]);
    assert_eq!(invocacion.celula(), "piloto-01");
    assert_eq!(invocacion.copia(), "copia.db");
    assert_eq!(invocacion.desde(), Some("2026-09-01"));
    assert_eq!(invocacion.hasta(), Some("2026-09-10"));
    assert_eq!(invocacion.desde_ms(), Some(MS_2026_09_01));
    assert_eq!(invocacion.hasta_ms(), Some(MS_2026_09_10));
    assert!(invocacion.simular());
}

#[test]
fn reporte_tokens_valido_en_ortografia_inline_y_sin_periodo() {
    let invocacion = invocacion_reporte(&[
        "reporte",
        "tokens",
        "--celula=piloto-02",
        "--copia=respaldo/copia.db",
    ]);
    assert_eq!(invocacion.celula(), "piloto-02");
    assert_eq!(invocacion.copia(), "respaldo/copia.db");
    assert_eq!(invocacion.desde(), None);
    assert_eq!(invocacion.hasta(), None);
    assert_eq!(invocacion.desde_ms(), None);
    assert_eq!(invocacion.hasta_ms(), None);
    assert!(!invocacion.simular());
}

/// El acceso genérico `Comando::subcomando` devuelve `None` para `reporte tokens`, igual
/// que para `config render`: el grupo no usa la gramática de `Subcomando`.
#[test]
fn reporte_tokens_no_devuelve_subcomando_de_cell() {
    let comando = analizar(&args(&[
        "reporte", "tokens", "--celula", "c1", "--copia", "copia.db",
    ]))
    .unwrap();
    assert_eq!(comando.subcomando(), None);
    assert_eq!(comando.id(), None);
    assert_eq!(comando.motivo(), None);
    assert!(!comando.confirmar());
}

#[test]
fn reporte_sin_subcomando_tokens_es_rechazado() {
    let resultado = analizar(&args(&["reporte"]));
    assert!(matches!(
        resultado.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));
    let resultado = analizar(&args(&["reporte", "resumen", "--celula", "c1"]));
    assert!(matches!(
        resultado.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));
}

#[test]
fn reporte_tokens_exige_celula_y_copia() {
    for snippet in [
        &["reporte", "tokens", "--copia", "copia.db"][..],
        &["reporte", "tokens", "--celula", "c1"][..],
        &["reporte", "tokens"][..],
    ] {
        let resultado = analizar(&args(snippet));
        assert!(
            matches!(
                resultado.unwrap_err(),
                ErrorDeArgumentos::ReporteInvalido { .. }
            ),
            "snippet {snippet:?}"
        );
    }
}

#[test]
fn reporte_tokens_rechaza_opciones_desconocidas_repetidas_y_posicionales() {
    let desconocida = analizar(&args(&[
        "reporte",
        "tokens",
        "--celula",
        "c1",
        "--copia",
        "copia.db",
        "--no-existe",
    ]));
    assert!(matches!(
        desconocida.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));

    let repetida = analizar(&args(&[
        "reporte", "tokens", "--celula", "c1", "--celula", "c2", "--copia", "copia.db",
    ]));
    assert!(matches!(
        repetida.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));

    let sin_valor = analizar(&args(&["reporte", "tokens", "--celula", "c1", "--copia"]));
    assert!(matches!(
        sin_valor.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));

    let inline_vacio = analizar(&args(&["reporte", "tokens", "--celula", "c1", "--copia="]));
    assert!(matches!(
        inline_vacio.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));

    let posicional = analizar(&args(&[
        "reporte", "tokens", "--celula", "c1", "--copia", "copia.db", "sobrante",
    ]));
    assert!(matches!(
        posicional.unwrap_err(),
        ErrorDeArgumentos::ReporteInvalido { .. }
    ));
}

/// AC-3 a nivel de análisis: una copia llamada `sessions.db`, o una ruta que termina en
/// `-wal`/`-shm`, se rechaza con la variante dedicada y el mensaje fijo, sin importar qué
/// más acompañe a la invocación. La comprobación es léxica: ninguna de estas rutas llega a
/// abrirse.
#[test]
fn reporte_tokens_rechaza_copia_sessions_db_o_sufijos_de_wal_y_shm() {
    for copia in [
        "sessions.db",
        "ruta/sessions.db",
        "/absoluta/sessions.db",
        "copia.db-wal",
        "/absoluta/copia.db-wal",
        "copia.db-shm",
        "respaldo/copia.db-shm",
    ] {
        let resultado = analizar(&args(&[
            "reporte", "tokens", "--celula", "c1", "--copia", copia,
        ]));
        assert_eq!(
            resultado.unwrap_err(),
            ErrorDeArgumentos::CopiaEsSessionsDb,
            "copia {copia:?}"
        );
    }
}

/// El rechazo de AC-3 es incondicional: tampoco lo salta `--simular`, porque la
/// comprobación vive en el analizador, antes de que el modo de simulación se consulte.
#[test]
fn reporte_tokens_rechaza_sessions_db_incluso_con_simular() {
    let resultado = analizar(&args(&[
        "reporte",
        "tokens",
        "--celula",
        "c1",
        "--copia",
        "sessions.db",
        "--simular",
    ]));
    assert_eq!(resultado.unwrap_err(), ErrorDeArgumentos::CopiaEsSessionsDb);
}

#[test]
fn mensaje_exacto_de_copia_sessions_db() {
    assert_eq!(
        ErrorDeArgumentos::CopiaEsSessionsDb.to_string(),
        "el reporte sólo lee copias VACUUM INTO, nunca sessions.db"
    );
}

/// AC-5 a nivel de análisis: toda fecha `AAAA-MM-DD` que no exista en el calendario
/// gregoriano en UTC, o que no tenga la forma exacta, se rechaza como `ReporteInvalido`
/// sin que la copia se abra (la comprobación ocurre antes de construir el comando).
#[test]
fn reporte_tokens_rechaza_fechas_malformadas() {
    let malformadas = [
        "2026-02-30",  // febrero de 2026 tiene 28 días.
        "2026-13-01",  // mes fuera de rango.
        "2026-00-10",  // mes cero.
        "2026-02-29",  // 2026 no es bisiesto.
        "2023-02-29",  // 2023 no es bisiesto.
        "1900-02-29",  // divisible por 100 y no por 400: no es bisiesto.
        "2026-2-01",   // longitud incorrecta.
        "2026-02-1",   // longitud incorrecta.
        "abcd-02-01",  // año no numérico.
        "2026-02-01x", // día no numérico.
        "2026 02 01",  // separadores incorrectos.
    ];
    for mala in malformadas {
        for bandera in ["--desde", "--hasta"] {
            let resultado = analizar(&args(&[
                "reporte", "tokens", "--celula", "c1", "--copia", "copia.db", bandera, mala,
            ]));
            assert!(
                matches!(
                    resultado.unwrap_err(),
                    ErrorDeArgumentos::ReporteInvalido { .. }
                ),
                "{bandera} {mala:?} debía rechazarse"
            );
        }
    }
}

/// El extremo opuesto de AC-5: las fechas que el calendario sí tiene —incluidos los dos
/// casos bisiestos y los bordes de año— se aceptan y se convierten a milisegundos.
#[test]
fn reporte_tokens_acepta_fechas_validas_incluidos_los_bisiestos() {
    for buena in [
        "1970-01-01",
        "2000-02-29", // bisiesto por divisibilidad entre 400.
        "2024-02-29", // bisiesto por divisibilidad entre 4.
        "2026-02-28",
        "2026-12-31",
    ] {
        let invocacion = invocacion_reporte(&[
            "reporte", "tokens", "--celula", "c1", "--copia", "copia.db", "--desde", buena,
        ]);
        assert_eq!(invocacion.desde(), Some(buena), "desde {buena:?}");
        assert!(
            invocacion.desde_ms().is_some(),
            "ms presente para {buena:?}"
        );
    }
}

/// Conversión exacta a milisegundos desde la época para las anclas del calendario: la
/// prueba falla si la aritmética civil se desvía en un solo día (mutación típica: un
/// `+` por `-` en `dias_desde_la_epoca`, o el año de la era mal descontado).
#[test]
fn reporte_tokens_convierte_fechas_a_milisegundos_con_anclas_independientes() {
    let casos = [
        ("1970-01-01", MS_1970_01_01),
        ("2000-02-29", MS_2000_02_29),
        ("2024-02-29", MS_2024_02_29),
        ("2026-02-28", MS_2026_02_28),
        ("2026-09-01", MS_2026_09_01),
        ("2026-09-10", MS_2026_09_10),
    ];
    for (fecha, esperado) in casos {
        let invocacion = invocacion_reporte(&[
            "reporte", "tokens", "--celula", "c1", "--copia", "copia.db", "--desde", fecha,
        ]);
        assert_eq!(invocacion.desde_ms(), Some(esperado), "fecha {fecha:?}");
    }
}
