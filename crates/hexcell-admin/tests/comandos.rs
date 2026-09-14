//! Pruebas externas del servicio de aplicación `comandos::ejecutar`.
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`. Cada prueba inyecta dos
//! búferes en memoria en `Salida::nueva` y aserta el código de salida, los bytes exactos
//! de cada sumidero y la vacuidad del otro. Ningún `match` sobre `Subcomando` tiene brazo
//! comodín.

use std::io::Write;

use hexcell_admin::argumentos::{Subcomando, analizar};
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::{ejecutar, estado_objetivo};
use hexcell_admin::estado_de_celula::EstadoDeCelula;
use hexcell_admin::salida::Salida;

fn args(snippet: &[&str]) -> Vec<String> {
    snippet.iter().map(|s| (*s).to_string()).collect()
}

fn ejecutar_con(snippet: &[&str]) -> (CodigoDeSalida, String, String) {
    let argumentos = args(snippet);
    let resultado = analizar(&argumentos);
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        let codigo = ejecutar(resultado, &mut salida);
        drop(salida);
        let estandar = String::from_utf8(bufer_estandar).expect("UTF-8 en el estándar");
        let diagnostico = String::from_utf8(bufer_diagnostico).expect("UTF-8 en el diagnóstico");
        (codigo, estandar, diagnostico)
    }
}

#[test]
fn errores_de_analisis_devuelven_uso_incorrecto_con_diagnostico_y_estandar_vacio() {
    let casos = [
        (&[][..], "falta el subcomando"),
        (&["server"][..], "grupo desconocido"),
        (&["cell", "restart"][..], "restart"),
        (&["cell", "list", "--foo"][..], "--foo"),
        (&["cell", "pause", "--id"][..], "--id"),
        (&["cell", "pause", "--id", "c1", "--id", "c2"][..], "--id"),
        (&["cell", "list", "--id", "c1"][..], "--id"),
        (&["cell", "list", "extra"][..], "extra"),
    ];
    for (snippet, token) in casos {
        let (codigo, estandar, diagnostico) = ejecutar_con(&snippet);
        assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto, "snippet {snippet:?}");
        assert_ne!(codigo, CodigoDeSalida::Exito);
        assert_ne!(codigo, CodigoDeSalida::Fallo);
        assert!(
            estandar.is_empty(),
            "estándar vacío para {snippet:?}: {estandar:?}"
        );
        assert!(
            diagnostico.contains(token),
            "diagnóstico contiene «{token}» para {snippet:?}: {diagnostico:?}"
        );
        assert!(
            diagnostico.contains("Uso:"),
            "texto de uso para {snippet:?}: {diagnostico:?}"
        );
    }
}

#[test]
fn subcomando_valido_sin_simular_devuelve_no_implementado_todavia() {
    let casos = [
        (&["cell", "pause", "--id", "c1"][..], "pause"),
        (&["cell", "unpause", "--id", "c1"][..], "unpause"),
        (
            &["cell", "terminate", "--id", "c1", "--confirmar"][..],
            "terminate",
        ),
        (
            &[
                "cell",
                "rebind",
                "--id",
                "c1",
                "--motivo",
                "x",
                "--confirmar",
            ][..],
            "rebind",
        ),
        (&["cell", "list"][..], "list"),
        (&["cell", "status", "--id", "c1"][..], "status"),
    ];
    for (snippet, nombre) in casos {
        let (codigo, estandar, diagnostico) = ejecutar_con(&snippet);
        assert_eq!(
            codigo,
            CodigoDeSalida::NoImplementadoTodavia,
            "snippet {snippet:?}"
        );
        assert!(
            estandar.is_empty(),
            "estándar vacío para «{nombre}»: {estandar:?}"
        );
        let esperado = format!(
            "subcomando «{nombre}» todavía no implementado (tareas 11 a 15 de la etapa A-6)\n"
        );
        assert_eq!(
            diagnostico, esperado,
            "diagnóstico de «{nombre}»: {diagnostico:?}"
        );
    }
}

#[test]
fn subcomando_valido_con_simular_devuelve_exito_y_linea_en_estandar() {
    let casos = [
        (
            &["cell", "pause", "--id", "c1", "--simular"][..],
            "simulación: cell pause --id c1 -> estado objetivo: suspendida\n",
        ),
        (
            &["cell", "unpause", "--id", "c1", "--simular"][..],
            "simulación: cell unpause --id c1 -> estado objetivo: en ejecución\n",
        ),
        (
            &[
                "cell",
                "terminate",
                "--id",
                "c1",
                "--confirmar",
                "--simular",
            ][..],
            "simulación: cell terminate --id c1 -> estado objetivo: retirada\n",
        ),
        (
            &[
                "cell",
                "rebind",
                "--id",
                "c1",
                "--motivo",
                "baneo permanente",
                "--confirmar",
                "--simular",
            ][..],
            "simulación: cell rebind --id c1 --motivo \"baneo permanente\" -> estado objetivo: reemparejando\n",
        ),
        (
            &["cell", "list", "--simular"][..],
            "simulación: cell list\n",
        ),
        (
            &["cell", "status", "--id", "c1", "--simular"][..],
            "simulación: cell status --id c1\n",
        ),
    ];
    for (snippet, esperado) in casos {
        let (codigo, estandar, diagnostico) = ejecutar_con(&snippet);
        assert_eq!(codigo, CodigoDeSalida::Exito, "snippet {snippet:?}");
        assert_eq!(estandar, esperado, "estándar de {snippet:?}");
        assert!(
            diagnostico.is_empty(),
            "diagnóstico vacío para {snippet:?}: {diagnostico:?}"
        );
    }
}

#[test]
fn los_mensajes_de_simulacion_no_contienen_la_taxonomia_de_sesion_del_ipc() {
    let casos = [
        &["cell", "pause", "--id", "c1", "--simular"][..],
        &["cell", "unpause", "--id", "c1", "--simular"][..],
        &[
            "cell",
            "terminate",
            "--id",
            "c1",
            "--confirmar",
            "--simular",
        ][..],
        &[
            "cell",
            "rebind",
            "--id",
            "c1",
            "--motivo",
            "x",
            "--confirmar",
            "--simular",
        ][..],
    ];
    for snippet in casos {
        let (_, estandar, _) = ejecutar_con(&snippet);
        assert!(
            !estandar.contains("desvinculada_sesion_cerrada"),
            "taxonomía del IPC ausente: {estandar:?}"
        );
    }
}

struct EscritorQueFalla;

impl Write for EscritorQueFalla {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("fallo simulado de escritura"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn fallos_de_escritura_se_convierten_en_fallo() {
    let argumentos = args(&["cell", "list", "--simular"]);
    let resultado = analizar(&argumentos);
    let mut salida = Salida::nueva(EscritorQueFalla, Vec::<u8>::new());
    assert_eq!(ejecutar(resultado, &mut salida), CodigoDeSalida::Fallo);

    let argumentos = args(&["cell", "list"]);
    let resultado = analizar(&argumentos);
    let mut salida = Salida::nueva(Vec::<u8>::new(), EscritorQueFalla);
    assert_eq!(ejecutar(resultado, &mut salida), CodigoDeSalida::Fallo);

    let argumentos = args(&[]);
    let resultado = analizar(&argumentos);
    let mut salida = Salida::nueva(Vec::<u8>::new(), EscritorQueFalla);
    assert_eq!(ejecutar(resultado, &mut salida), CodigoDeSalida::Fallo);
}

#[test]
fn los_flujos_nunca_se_cruzan() {
    let (_, estandar, diagnostico) = ejecutar_con(&["cell", "pause", "--id", "c1", "--simular"]);
    assert!(!estandar.is_empty());
    assert!(diagnostico.is_empty());

    let (_, estandar, diagnostico) = ejecutar_con(&["cell", "pause", "--id", "c1"]);
    assert!(estandar.is_empty());
    assert!(!diagnostico.is_empty());
}

#[test]
fn estado_objetivo_es_exhaustivo_y_nombra_el_destino_correcto() {
    assert_eq!(
        estado_objetivo(Subcomando::Pausar),
        Some(EstadoDeCelula::Suspendida)
    );
    assert_eq!(
        estado_objetivo(Subcomando::Reanudar),
        Some(EstadoDeCelula::EnEjecucion)
    );
    assert_eq!(
        estado_objetivo(Subcomando::Retirar),
        Some(EstadoDeCelula::Retirada)
    );
    assert_eq!(
        estado_objetivo(Subcomando::Reemparejar),
        Some(EstadoDeCelula::Reemparejando)
    );
    assert_eq!(estado_objetivo(Subcomando::Listar), None);
    assert_eq!(estado_objetivo(Subcomando::Estado), None);
}
