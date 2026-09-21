//! Pruebas externas del servicio de aplicación `comandos::ejecutar` y `comandos::ejecutar_con_efectos`.
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`. Cada prueba inyecta dos
//! búferes en memoria en `Salida::nueva` y aserta el código de salida, los bytes exactos
//! de cada sumidero y la vacuidad del otro. Ningún `match` sobre `Subcomando` tiene brazo
//! comodín.

mod comun;

use std::io::Write;

use hexcell_admin::argumentos::{Subcomando, analizar};
use hexcell_admin::ciclo_de_vida::DatosDeSondeo;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::{ejecutar, ejecutar_con_efectos, estado_objetivo};
use hexcell_admin::docker::ClienteDocker;
use hexcell_admin::estado_de_celula::EstadoDeCelula;
use hexcell_admin::salida::Salida;

use comun::{Guion, ServidorDockerFalso, ruta_socket_sin_vincular};

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

#[test]
fn un_escritor_que_falla_en_diagnostico_sin_simular_devuelve_fallo() {
    let mut salida = Salida::nueva(Vec::<u8>::new(), EscritorQueFalla);
    let resultado = analizar(&args(&["cell", "list"]));
    assert_eq!(ejecutar(resultado, &mut salida), CodigoDeSalida::Fallo);
}

#[test]
fn un_escritor_que_falla_en_diagnostico_con_error_de_analisis_devuelve_fallo() {
    let mut salida = Salida::nueva(Vec::<u8>::new(), EscritorQueFalla);
    let resultado = analizar(&args(&[]));
    assert_eq!(ejecutar(resultado, &mut salida), CodigoDeSalida::Fallo);
}

fn ejecutar_con_efectos_con(
    snippet: &[&str],
    cliente: &ClienteDocker,
) -> (CodigoDeSalida, String, String) {
    let resultado = analizar(&args(snippet));
    let datos = DatosDeSondeo {
        imagen: "sonda-de-prueba:1".to_string(),
        limite_segundos: 45,
    };
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    let codigo = {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        ejecutar_con_efectos(resultado, &mut salida, cliente, datos)
    };
    let estandar = String::from_utf8(bufer_estandar).expect("UTF-8 en el estándar");
    let diagnostico = String::from_utf8(bufer_diagnostico).expect("UTF-8 en el diagnóstico");
    (codigo, estandar, diagnostico)
}

/// AC-6: `ejecutar_con_efectos` despacha `cell pause` a `ciclo_de_vida::pausar`: ya no cae en el
/// brazo `NoImplementadoTodavia` que `ejecutar` sigue usando para la CLI sin efectos.
#[test]
fn ejecutar_con_efectos_despacha_pausar_a_ciclo_de_vida() {
    let servidor = ServidorDockerFalso::nuevo("efectos-pausar");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        });
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        });
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let (codigo, estandar, diagnostico) =
        ejecutar_con_efectos_con(&["cell", "pause", "--id", "c1"], &cliente);

    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(estandar, "cell pause completado para «c1»\n");
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");

    hilo.join().unwrap();
}

/// AC-6: `ejecutar_con_efectos` despacha `cell unpause` a `ciclo_de_vida::reanudar`, atravesando
/// las cinco operaciones Docker de la reanudación hasta el 200 de la sonda.
#[test]
fn ejecutar_con_efectos_despacha_reanudar_a_ciclo_de_vida() {
    let servidor = ServidorDockerFalso::nuevo("efectos-reanudar");
    let ruta = servidor.ruta();
    // El fin del guion viaja por un canal leído con `recv_timeout`, no por un `join` ciego: una
    // petición que falte pone el test rojo dentro del límite en vez de colgarlo.
    let (emisor, receptor) = std::sync::mpsc::channel();
    let _hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        }); // iniciar núcleo
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        }); // iniciar sidecar
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"NetworkSettings":{"Networks":{"red-del-operador":{"NetworkID":"n1"}}},"Config":{"Env":["HEXCELL_DIRECCION_SALUD=0.0.0.0:9099"]}}"#,
        });
        servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        });
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        }); // iniciar sonda
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":0}"#,
        });
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        }); // eliminar sonda
        let _ = emisor.send(());
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let (codigo, estandar, diagnostico) =
        ejecutar_con_efectos_con(&["cell", "unpause", "--id", "c1"], &cliente);

    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(estandar, "cell unpause completado para «c1»\n");
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");

    receptor
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("el demonio falso debía haber atendido las siete peticiones dentro del límite");
}

/// AC-6: `cell terminate`, `cell rebind`, `cell list` y `cell status` siguen devolviendo
/// `NoImplementadoTodavia` a través de `ejecutar_con_efectos`, sin `--simular`. El cliente
/// apunta a un socket sin vincular a propósito: si el despacho intentara tocar Docker para
/// cualquiera de los cuatro, la operación fallaría con `DemonioInalcanzable` (código `Fallo`) en
/// vez de devolver `NoImplementadoTodavia`, así que el propio código de salida es la prueba de
/// que ningún `ClienteDocker` se invocó. `cell list` es el caso señalado por HEX-080: nunca
/// admite `--id`, así que el despacho tiene que resolverlo ANTES de exigir un identificador.
#[test]
fn ejecutar_con_efectos_deja_los_otros_cuatro_subcomandos_en_no_implementado_sin_tocar_docker() {
    let ruta = ruta_socket_sin_vincular("efectos-sin-docker");
    let cliente = ClienteDocker::nuevo(ruta);
    let casos = [
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
        let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(snippet, &cliente);
        assert_eq!(
            codigo,
            CodigoDeSalida::NoImplementadoTodavia,
            "snippet {snippet:?}"
        );
        assert!(
            estandar.is_empty(),
            "estándar vacío para «{nombre}»: {estandar:?}"
        );
        assert!(
            diagnostico.contains("todavía no implementado"),
            "diagnóstico de «{nombre}»: {diagnostico:?}"
        );
    }
}

/// AC-6: el modo `--simular` y los errores de análisis siguen resolviéndose por
/// `comandos::ejecutar` sin construir ningún `ClienteDocker`: el mismo socket sin vincular que
/// haría fallar a Docker no impide ni el `Exito` de la simulación ni el `UsoIncorrecto` del
/// análisis, porque ninguno de los dos caminos lo toca.
#[test]
fn ejecutar_con_efectos_resuelve_simular_y_errores_de_analisis_sin_construir_cliente_docker() {
    let ruta = ruta_socket_sin_vincular("efectos-simular");
    let cliente = ClienteDocker::nuevo(ruta);

    let (codigo, estandar, diagnostico) =
        ejecutar_con_efectos_con(&["cell", "pause", "--id", "c1", "--simular"], &cliente);
    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(
        estandar,
        "simulación: cell pause --id c1 -> estado objetivo: suspendida\n"
    );
    assert!(diagnostico.is_empty());

    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(&["cell", "restart"], &cliente);
    assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto);
    assert!(estandar.is_empty());
    assert!(diagnostico.contains("Uso:"));
}
