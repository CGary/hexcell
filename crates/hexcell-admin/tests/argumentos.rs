//! Pruebas externas del analizador de argumentos `argumentos::analizar`.
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`. El analizador es una
//! función pura sobre una porción de argumentos, así que las pruebas lo ejercitan con un
//! `Vec<String>` propio sin tocar `std::env::args`. Ningún `match` sobre `Subcomando`
//! tiene brazo comodín.

use hexcell_admin::argumentos::{ErrorDeArgumentos, Subcomando, analizar};

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
    match invocacion.subcomando() {
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
    assert_eq!(p.subcomando(), Subcomando::Pausar);
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
    assert_eq!(r.subcomando(), Subcomando::Reemparejar);
    assert_eq!(r.motivo(), Some("baneo"));
    assert!(r.simular());

    let l = analizar(&args(&["cell", "list"])).unwrap();
    assert_eq!(l.subcomando(), Subcomando::Listar);
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
        "grupo desconocido: «server» (el único grupo admitido es «cell»)"
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
