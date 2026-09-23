//! Pruebas externas del servicio de aplicación `comandos::ejecutar` y `comandos::ejecutar_con_efectos`.
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`. Cada prueba inyecta dos
//! búferes en memoria en `Salida::nueva` y aserta el código de salida, los bytes exactos
//! de cada sumidero y la vacuidad del otro. Ningún `match` sobre `Subcomando` tiene brazo
//! comodín.

mod comun;

use std::io::Write;

use hexcell_admin::almacen_plano_de_control::{AlmacenDelPlanoDeControl, etiqueta_persistida};
use hexcell_admin::argumentos::{Subcomando, analizar};
use hexcell_admin::ciclo_de_vida::DatosDeSondeo;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::{ejecutar, ejecutar_con_efectos, estado_objetivo};
use hexcell_admin::docker::{ClienteDocker, InventarioDocker};
use hexcell_admin::estado_de_celula::EstadoDeCelula;
use hexcell_admin::salida::Salida;

use comun::{
    AlmacenTemporal, Guion, PeticionRecibida, ServidorDockerFalso, exigir_silencio,
    ruta_socket_sin_vincular, secuencia_recibida, servir_guiones,
};

/// Reloj que la prueba inyecta en `ejecutar_con_efectos`.
///
/// Distinto de cualquier valor que produzca `SystemTime::now()` en la máquina que corre la
/// prueba: si producción sellara la fila con su propio reloj en vez de con el `ahora_ms` que
/// recibe, las aserciones sobre `actualizado_ms` y `registrado_ms` se pondrían rojas.
const RELOJ_INYECTADO: i64 = 1_700_000_000_000;

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
        // Grupo `reporte tokens` (tarea 23 de la etapa A-6, HEX-084).
        (&["reporte", "otro"][..], "tokens"),
        (&["reporte", "tokens", "--celula", "c1"][..], "--copia"),
        (
            &["reporte", "tokens", "--copia", "copia.db"][..],
            "--celula",
        ),
        (
            &[
                "reporte",
                "tokens",
                "--celula",
                "c1",
                "--copia",
                "sessions.db",
            ][..],
            "nunca sessions.db",
        ),
        (
            &[
                "reporte",
                "tokens",
                "--celula",
                "c1",
                "--copia",
                "copia.db",
                "--desde",
                "2026-02-30",
            ][..],
            "fecha inválida",
        ),
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
    inventario: &InventarioDocker,
    ruta_almacen: &str,
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
        ejecutar_con_efectos(
            resultado,
            &mut salida,
            cliente,
            inventario,
            ruta_almacen,
            RELOJ_INYECTADO,
            datos,
        )
    };
    let estandar = String::from_utf8(bufer_estandar).expect("UTF-8 en el estándar");
    let diagnostico = String::from_utf8(bufer_diagnostico).expect("UTF-8 en el diagnóstico");
    (codigo, estandar, diagnostico)
}

/// Respuesta sin cuerpo del demonio falso, en una línea.
fn sin_cuerpo(estado: u16, razon: &'static str) -> Guion {
    Guion::SinCuerpo { estado, razon }
}

/// Sirve en otro hilo las siete peticiones de una reanudación —arrancada del núcleo, arrancada
/// del sidecar, inspección, creación de la sonda, arrancada de la sonda, espera de su código de
/// salida y borrado— y avisa por el canal al terminar. `veredicto` es el cuerpo de `/wait`:
/// `StatusCode: 0` es el primer 200 OK de `/health/ready` y cualquier otro código es el límite
/// agotado. El borrado se sirve en los dos casos porque `reanudar` limpia también al fallar.
fn guion_de_reanudacion(
    servidor: ServidorDockerFalso,
    veredicto: &'static [u8],
) -> std::sync::mpsc::Receiver<()> {
    let (emisor, receptor) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar núcleo
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar sidecar
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
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar sonda
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: veredicto,
        });
        servidor.atender(sin_cuerpo(204, "No Content")); // eliminar sonda
        let _ = emisor.send(());
    });
    receptor
}

/// Cota finita: una petición que falte pone el test rojo en vez de colgarlo.
fn esperar_guion(receptor: &std::sync::mpsc::Receiver<()>) {
    receptor
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("el demonio falso debía haber atendido las siete peticiones dentro del límite");
}

/// AC-6: `ejecutar_con_efectos` despacha `cell pause` a `ciclo_de_vida::pausar`: ya no cae en el
/// brazo `NoImplementadoTodavia` que `ejecutar` sigue usando para la CLI sin efectos.
#[test]
fn ejecutar_con_efectos_despacha_pausar_a_ciclo_de_vida() {
    let servidor = ServidorDockerFalso::nuevo("efectos-pausar");
    let ruta = servidor.ruta();
    let receptor = servir_guiones(
        servidor,
        vec![sin_cuerpo(204, "No Content"), sin_cuerpo(204, "No Content")],
    );

    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-pausar");
    let ruta_almacen = almacen.texto();
    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "pause", "--id", "c1"],
        &cliente,
        &inventario,
        &ruta_almacen,
    );

    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(estandar, "cell pause completado para «c1»\n");
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");

    assert_eq!(
        secuencia_recibida(&receptor, 2),
        vec![
            "POST /containers/c1-sidecar/stop",
            "POST /containers/c1-nucleo/stop"
        ],
        "la pausa detiene primero el sidecar y después el núcleo"
    );
}

/// AC-6: `ejecutar_con_efectos` despacha `cell unpause` a `ciclo_de_vida::reanudar`, atravesando
/// las cinco operaciones Docker de la reanudación hasta el 200 de la sonda.
#[test]
fn ejecutar_con_efectos_despacha_reanudar_a_ciclo_de_vida() {
    let servidor = ServidorDockerFalso::nuevo("efectos-reanudar");
    let ruta = servidor.ruta();
    let receptor = guion_de_reanudacion(servidor, br#"{"StatusCode":0}"#);

    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-reanudar");
    let ruta_almacen = almacen.texto();
    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "unpause", "--id", "c1"],
        &cliente,
        &inventario,
        &ruta_almacen,
    );

    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(estandar, "cell unpause completado para «c1»\n");
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");

    esperar_guion(&receptor);
}

/// AC-7 (HEX-085-b): `cell rebind` ya no devuelve `NoImplementadoTodavia`; se despacha al camino
/// real. Con una fila en `Suspendida` y un socket sin vincular (Docker inalcanzable a
/// propósito), el comando termina en `Fallo` con el diagnóstico de célula suspendida y SIN
/// emitir ninguna petición Docker, probando que la validación del estado inicial ocurre antes
/// de cualquier interacción con el demonio.
#[test]
fn ejecutar_con_efectos_despacha_rebind_y_falla_antes_de_docker_cuando_esta_suspendida() {
    let ruta = ruta_socket_sin_vincular("rebind-suspendida");
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("rebind-suspendida");
    let ruta_almacen = almacen.texto();

    // Sembrar una fila en `Suspendida`.
    {
        let a = AlmacenDelPlanoDeControl::abrir(std::path::Path::new(&ruta_almacen)).unwrap();
        a.registrar_transicion("c1", None, EstadoDeCelula::Suspendida, "prueba", 1000)
            .unwrap();
    }

    let snippet = &[
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "sustitución de número",
        "--confirmar",
    ][..];
    let (codigo, estandar, diagnostico) =
        ejecutar_con_efectos_con(snippet, &cliente, &inventario, &ruta_almacen);

    assert_eq!(
        codigo,
        CodigoDeSalida::Fallo,
        "una célula suspendida debe fallar, no devolver NoImplementadoTodavia"
    );
    assert!(
        estandar.is_empty(),
        "estándar debe quedar vacío en el fallo: {estandar:?}"
    );
    assert!(
        diagnostico.contains("ejecute cell unpause antes de cell rebind"),
        "diagnóstico debe nombrar el estado suspendido: {diagnostico:?}"
    );
}

/// AC-6: el modo `--simular` y los errores de análisis siguen resolviéndose por
/// `comandos::ejecutar` sin construir ningún `ClienteDocker`: el mismo socket sin vincular que
/// haría fallar a Docker no impide ni el `Exito` de la simulación ni el `UsoIncorrecto` del
/// análisis, porque ninguno de los dos caminos lo toca.
#[test]
fn ejecutar_con_efectos_resuelve_simular_y_errores_de_analisis_sin_construir_cliente_docker() {
    let ruta = ruta_socket_sin_vincular("efectos-simular");
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-simular");
    let ruta_almacen = almacen.texto();

    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "pause", "--id", "c1", "--simular"],
        &cliente,
        &inventario,
        &ruta_almacen,
    );
    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(
        estandar,
        "simulación: cell pause --id c1 -> estado objetivo: suspendida\n"
    );
    assert!(diagnostico.is_empty());

    let (codigo, estandar, diagnostico) =
        ejecutar_con_efectos_con(&["cell", "restart"], &cliente, &inventario, &ruta_almacen);
    assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto);
    assert!(estandar.is_empty());
    assert!(diagnostico.contains("Uso:"));
}

/// AC-5: el camino `Err` de `ejecutar_con_efectos`, el que fija el código de salida del proceso.
///
/// Con la sonda agotando su límite, el despacho tiene que hacer las TRES cosas a la vez: código
/// distinto de cero, sumidero estándar VACÍO y el mensaje del error por el de diagnóstico. Cada
/// una sola deja viva una mutación distinta: con sólo el código sobrevive un despacho que se
/// traga el diagnóstico; con sólo el mensaje sobrevive uno que lo escribe y aun así devuelve
/// `Exito`, es decir `cell unpause` respondiendo 0 con la célula no disponible. El texto se
/// escribe entero aquí, sin importar el `Display`, para que una mutación no mueva los dos lados.
#[test]
fn ejecutar_con_efectos_reporta_fallo_con_diagnostico_cuando_la_sonda_agota_el_limite() {
    let servidor = ServidorDockerFalso::nuevo("efectos-reanudar-limite");
    let ruta = servidor.ruta();
    let receptor = guion_de_reanudacion(servidor, br#"{"StatusCode":1}"#);

    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-reanudar-limite");
    let ruta_almacen = almacen.texto();
    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "unpause", "--id", "c1"],
        &cliente,
        &inventario,
        &ruta_almacen,
    );

    assert_eq!(codigo, CodigoDeSalida::Fallo, "diag: {diagnostico:?}");
    assert!(estandar.is_empty(), "estándar vacío: {estandar:?}");
    assert_eq!(
        diagnostico,
        "la célula no alcanzó /health/ready: se agotó el límite de 45 segundos\n"
    );

    esperar_guion(&receptor);
}

// --------------------------------------------------------------------------------------------
// AC-2, AC-3 y AC-4 al nivel del COMANDO: lo que queda escrito en el almacén, y lo que no.
//
// Las pruebas del almacén demuestran que `registrar_transicion` escribe bien; éstas demuestran
// que `ejecutar_con_efectos` la llama —y sólo la llama— cuando debe. Sin ellas, un despacho que
// nunca persistiera nada, que persistiera antes de pedirle nada a Docker o que persistiera
// también en el camino de fallo pasaría el conjunto entero.
// --------------------------------------------------------------------------------------------

/// Siembra UNA fila en `celulas` y ninguna en `transiciones`.
///
/// La fila se inserta con `rusqlite` y no con `registrar_transicion` para que `transiciones`
/// empiece VACÍA: así «después del comando hay exactamente una transición» es una aserción sobre
/// lo que hizo el comando. El motivo y la marca sembrados divergen de los que produce producción,
/// así que tienen que cambiar para que la prueba pase.
fn sembrar_fila(temporal: &AlmacenTemporal, id: &str, estado: EstadoDeCelula) {
    drop(AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("crear el almacén del accesorio"));
    rusqlite::Connection::open(temporal.ruta())
        .expect("abrir el accesorio")
        .execute(
            "INSERT INTO celulas (id, estado, motivo, actualizado_ms) VALUES (?1, ?2, 'sembrada', 1)",
            rusqlite::params![id, etiqueta_persistida(estado)],
        )
        .expect("sembrar la fila de celulas");
}

fn lector(temporal: &AlmacenTemporal) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        temporal.ruta(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("abrir el almacén para leerlo")
}

/// La fila de `id` como `«estado motivo ms»`, o `None` si no tiene fila.
fn fila_de(temporal: &AlmacenTemporal, id: &str) -> Option<String> {
    lector(temporal)
        .query_row(
            "SELECT estado, motivo, actualizado_ms FROM celulas WHERE id = ?1",
            [id],
            |f| {
                Ok(format!(
                    "{} {} {}",
                    f.get::<_, String>(0)?,
                    f.get::<_, String>(1)?,
                    f.get::<_, i64>(2)?
                ))
            },
        )
        .ok()
}

/// Las filas de `transiciones` como `«id de>a motivo ms»`.
fn transiciones_de(temporal: &AlmacenTemporal) -> Vec<String> {
    let conexion = lector(temporal);
    let mut s = conexion
        .prepare("SELECT id_celula, de, a, motivo, registrado_ms FROM transiciones ORDER BY id")
        .unwrap();
    let v = s
        .query_map([], |f| {
            Ok(format!(
                "{} {}>{} {} {}",
                f.get::<_, String>(0)?,
                f.get::<_, String>(1)?,
                f.get::<_, String>(2)?,
                f.get::<_, String>(3)?,
                f.get::<_, i64>(4)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    v
}

/// Los siete guiones de una reanudación completa, en orden.
fn guiones_de_reanudacion(veredicto: &'static [u8]) -> Vec<Guion> {
    vec![
        sin_cuerpo(204, "No Content"),
        sin_cuerpo(204, "No Content"),
        Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"NetworkSettings":{"Networks":{"red-del-operador":{"NetworkID":"n1"}}},"Config":{"Env":["HEXCELL_DIRECCION_SALUD=0.0.0.0:9099"]}}"#,
        },
        Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        },
        sin_cuerpo(204, "No Content"),
        Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: veredicto,
        },
        sin_cuerpo(204, "No Content"),
    ]
}

fn dos_paradas() -> Vec<Guion> {
    vec![sin_cuerpo(204, "No Content"), sin_cuerpo(204, "No Content")]
}

/// Resultado de una invocación con efectos sobre un accesorio recién montado.
struct Corrida {
    codigo: CodigoDeSalida,
    estandar: String,
    diagnostico: String,
    almacen: AlmacenTemporal,
    /// Contenido del archivo del almacén ANTES del despacho, para la guarda de «no escribió».
    bytes_antes: Option<Vec<u8>>,
    receptor: std::sync::mpsc::Receiver<PeticionRecibida>,
}

/// Monta demonio falso y almacén, siembra la fila si se pide y despacha `snippet`.
fn correr_con_efectos(
    etiqueta: &str,
    fila: Option<EstadoDeCelula>,
    guiones: Vec<Guion>,
    snippet: &[&str],
) -> Corrida {
    let servidor = ServidorDockerFalso::nuevo(etiqueta);
    let ruta = servidor.ruta();
    let almacen = AlmacenTemporal::nuevo(etiqueta);
    if let Some(estado) = fila {
        sembrar_fila(&almacen, "c1", estado);
    }
    let receptor = servir_guiones(servidor, guiones);
    let bytes_antes = almacen.bytes();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let (codigo, estandar, diagnostico) =
        ejecutar_con_efectos_con(snippet, &cliente, &inventario, &almacen.texto());
    Corrida {
        codigo,
        estandar,
        diagnostico,
        almacen,
        bytes_antes,
        receptor,
    }
}

/// AC-2: `cell pause` sobre una fila en `en_ejecucion` deja `suspendida` en `celulas` y UNA sola
/// fila en `transiciones`, con el origen leído del almacén y el reloj inyectado.
#[test]
fn cell_pause_persiste_suspendida_y_una_sola_transicion() {
    let c = correr_con_efectos(
        "pausa-persiste",
        Some(EstadoDeCelula::EnEjecucion),
        dos_paradas(),
        &["cell", "pause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Exito, "diag: {:?}", c.diagnostico);
    assert_eq!(secuencia_recibida(&c.receptor, 2).len(), 2);
    assert_eq!(
        fila_de(&c.almacen, "c1").as_deref(),
        Some("suspendida  1700000000000"),
        "la fila sembrada queda suspendida, sin motivo de alta y con el reloj inyectado"
    );
    assert_eq!(
        transiciones_de(&c.almacen),
        ["c1 en_ejecucion>suspendida  1700000000000"],
        "exactamente una transición, con el origen leído del almacén"
    );
}

/// AC-2: `cell unpause` sobre una fila en `suspendida` deja `en_ejecucion` tras las siete
/// peticiones de la reanudación.
#[test]
fn cell_unpause_persiste_en_ejecucion_tras_la_secuencia_completa() {
    let c = correr_con_efectos(
        "reanuda-persiste",
        Some(EstadoDeCelula::Suspendida),
        guiones_de_reanudacion(br#"{"StatusCode":0}"#),
        &["cell", "unpause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Exito, "diag: {:?}", c.diagnostico);
    assert_eq!(secuencia_recibida(&c.receptor, 7).len(), 7);
    assert_eq!(
        fila_de(&c.almacen, "c1").as_deref(),
        Some("en_ejecucion  1700000000000")
    );
    assert_eq!(
        transiciones_de(&c.almacen),
        ["c1 suspendida>en_ejecucion  1700000000000"]
    );
}

/// AC-2, camino de fallo: con Docker respondiendo 500 a la PRIMERA parada, `cell pause` devuelve
/// `Fallo` y el almacén queda idéntico byte a byte.
///
/// La comparación de bytes no depende de la granularidad de la marca de tiempo del sistema de
/// archivos y pone roja tanto una escritura que preceda a Docker como una que se cuele en el
/// camino de error.
#[test]
fn cell_pause_no_escribe_nada_cuando_docker_falla() {
    let c = correr_con_efectos(
        "pausa-docker-500",
        Some(EstadoDeCelula::EnEjecucion),
        vec![
            sin_cuerpo(500, "Internal Server Error"),
            sin_cuerpo(204, "No Content"),
        ],
        &["cell", "pause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Fallo);
    assert!(c.estandar.is_empty(), "estándar vacío: {:?}", c.estandar);
    assert!(!c.diagnostico.is_empty(), "el fallo se diagnostica");
    assert_eq!(secuencia_recibida(&c.receptor, 1).len(), 1);
    assert_eq!(
        c.almacen.bytes(),
        c.bytes_antes,
        "el almacén queda idéntico byte a byte tras el fallo de Docker"
    );
    assert_eq!(
        fila_de(&c.almacen, "c1").as_deref(),
        Some("en_ejecucion sembrada 1"),
        "la fila sembrada no se toca"
    );
    assert!(
        transiciones_de(&c.almacen).is_empty(),
        "ninguna transición se registra sin éxito de Docker"
    );
}

/// AC-3: `cell unpause` sobre una célula almacenada como `retirada` —el único estado terminal—
/// devuelve `Fallo` nombrando la transición ilegal y emite CERO peticiones Docker.
///
/// El cero se observa por el canal con `recv_timeout`, no por el código de salida: el socket está
/// vinculado y queda un guion pendiente, así que si el despacho conectara, la petición llegaría.
/// Un `assert_eq!` sobre el código no distingue «no pidió nada» de «pidió y falló».
#[test]
fn cell_unpause_sobre_retirada_falla_sin_emitir_ninguna_peticion_docker() {
    let c = correr_con_efectos(
        "retirada-sin-docker",
        Some(EstadoDeCelula::Retirada),
        vec![sin_cuerpo(204, "No Content")],
        &["cell", "unpause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Fallo);
    assert!(c.estandar.is_empty(), "estándar vacío: {:?}", c.estandar);
    assert_eq!(
        c.diagnostico,
        "el estado «retirada» es terminal: no admite ninguna transición hacia «en ejecución»\n",
        "el diagnóstico es el rechazo tipado de TransicionInvalida, literal y entero"
    );
    exigir_silencio(&c.receptor);
    assert_eq!(c.almacen.bytes(), c.bytes_antes, "el almacén no se toca");
    assert_eq!(
        fila_de(&c.almacen, "c1").as_deref(),
        Some("retirada sembrada 1")
    );
    assert!(transiciones_de(&c.almacen).is_empty());
}

/// AC-3: `cell pause` sobre una célula ya `suspendida` también se rechaza: los pares identidad
/// están ausentes de la tabla de transiciones a propósito. Cero peticiones Docker.
#[test]
fn cell_pause_sobre_suspendida_falla_sin_emitir_ninguna_peticion_docker() {
    let c = correr_con_efectos(
        "suspendida-sin-docker",
        Some(EstadoDeCelula::Suspendida),
        vec![sin_cuerpo(204, "No Content")],
        &["cell", "pause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Fallo);
    assert!(c.estandar.is_empty(), "estándar vacío: {:?}", c.estandar);
    assert_eq!(
        c.diagnostico,
        "transición no permitida: de «suspendida» a «suspendida» no figura en la tabla\n"
    );
    exigir_silencio(&c.receptor);
    assert!(transiciones_de(&c.almacen).is_empty());
}

/// AC-4: `cell pause` sobre un id sin fila la da de alta con motivo `alta_implicita`, y la
/// transición deja el origen vacío porque no había estado previo.
#[test]
fn cell_pause_sin_fila_da_de_alta_con_alta_implicita() {
    let c = correr_con_efectos(
        "alta-pausa",
        None,
        dos_paradas(),
        &["cell", "pause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Exito, "diag: {:?}", c.diagnostico);
    assert_eq!(secuencia_recibida(&c.receptor, 2).len(), 2);
    assert_eq!(
        fila_de(&c.almacen, "c1").as_deref(),
        Some("suspendida alta_implicita 1700000000000")
    );
    assert_eq!(
        transiciones_de(&c.almacen),
        ["c1 >suspendida alta_implicita 1700000000000"]
    );
}

/// AC-4: lo mismo para `cell unpause`, que llega a `en_ejecucion` por la reanudación completa.
#[test]
fn cell_unpause_sin_fila_da_de_alta_con_alta_implicita() {
    let c = correr_con_efectos(
        "alta-reanuda",
        None,
        guiones_de_reanudacion(br#"{"StatusCode":0}"#),
        &["cell", "unpause", "--id", "c1"],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Exito, "diag: {:?}", c.diagnostico);
    assert_eq!(secuencia_recibida(&c.receptor, 7).len(), 7);
    assert_eq!(
        fila_de(&c.almacen, "c1").as_deref(),
        Some("en_ejecucion alta_implicita 1700000000000")
    );
}

// ============================================================================
// Tests de terminate (HEX-082-b)
// ============================================================================

/// Nombre de volumen del accesorio para terminate: no derivable del --id.
const VOLUMEN_DE_TERMINATE: &str = "volumen-datos-terminate-z8w3q5";

/// Inspección del núcleo para terminate: con red, puerto de admin y volumen del accesorio.
fn inspeccion_nucleo_para_terminate() -> Guion {
    Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: br#"{"State":{"Status":"running"},"NetworkSettings":{"Networks":{"red-terminate":{"NetworkID":"n1"}}},"Config":{"Env":["PATH=/usr/bin","HEXCELL_DIRECCION_ADMIN=0.0.0.0:7071"]},"Mounts":[{"Type":"volume","Name":"volumen-datos-terminate-z8w3q5","Destination":"/var/lib/hexcell"}]}"#,
    }
}

/// Sirve las peticiones de un terminate exitoso.
fn servir_terminate(servidor: ServidorDockerFalso) -> std::sync::mpsc::Receiver<()> {
    let (emisor, receptor) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        servidor.atender(inspeccion_nucleo_para_terminate()); // inspeccionar núcleo
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"State":{"Status":"running"}}"#,
        }); // inspeccionar sidecar
        servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda-cierre","Warnings":[]}"#,
        }); // crear sonda
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar sonda
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":0}"#,
        }); // esperar sonda
        servidor.atender(sin_cuerpo(204, "No Content")); // eliminar sonda
        servidor.atender(sin_cuerpo(204, "No Content")); // detener sidecar
        servidor.atender(sin_cuerpo(204, "No Content")); // detener núcleo
        servidor.atender(sin_cuerpo(204, "No Content")); // eliminar sidecar
        servidor.atender(sin_cuerpo(204, "No Content")); // eliminar núcleo
        servidor.atender(sin_cuerpo(204, "No Content")); // eliminar volumen
        let _ = emisor.send(());
    });
    receptor
}

/// Sirve las peticiones de un terminate que aborta en el paso 3: la sonda de cierre de sesión
/// responde con código de salida distinto de cero, así que ningún `stop` ni `rm` se emite
/// después de su propia limpieza (R6).
fn servir_terminate_con_cierre_fallido(
    servidor: ServidorDockerFalso,
) -> std::sync::mpsc::Receiver<()> {
    let (emisor, receptor) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        servidor.atender(inspeccion_nucleo_para_terminate()); // inspeccionar núcleo
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"State":{"Status":"running"}}"#,
        }); // inspeccionar sidecar
        servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda-cierre","Warnings":[]}"#,
        }); // crear sonda
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar sonda
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":1}"#,
        }); // esperar sonda: cierre de sesión fallido
        servidor.atender(sin_cuerpo(204, "No Content")); // eliminar sonda (limpieza en ambos caminos)
        let _ = emisor.send(());
    });
    receptor
}

/// AC-9: `cell terminate --id X --confirmar` despacha a `ciclo_de_vida::retirar` a través de
/// `ejecutar_con_efectos`, imprimiendo las tres líneas fijas en el sumidero estándar.
#[test]
fn ejecutar_con_efectos_despacha_terminate_a_ciclo_de_vida() {
    let servidor = ServidorDockerFalso::nuevo("efectos-terminate");
    let ruta = servidor.ruta();
    let receptor = servir_terminate(servidor);

    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-terminate");
    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "terminate", "--id", "c1", "--confirmar"],
        &cliente,
        &inventario,
        &almacen.texto(),
    );

    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert_eq!(
        estandar,
        format!(
            "sesión cerrada\ncontenedores eliminados\nvolumen {VOLUMEN_DE_TERMINATE} eliminado\n"
        ),
        "las tres líneas fijas en orden"
    );
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");

    esperar_guion(&receptor);
}

/// AC-9: `cell terminate --simular` sigue resolviéndose por `comandos::ejecutar` sin construir
/// ningún `ClienteDocker`: el socket sin vincular haría fallar a Docker, pero el modo simulación
/// short-circuita antes de tocar nada.
#[test]
fn ejecutar_con_efectos_resuelve_terminate_simular_sin_construir_cliente_docker() {
    let ruta = ruta_socket_sin_vincular("efectos-terminate-simular");
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-terminate-simular");

    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &[
            "cell",
            "terminate",
            "--id",
            "c1",
            "--confirmar",
            "--simular",
        ],
        &cliente,
        &inventario,
        &almacen.texto(),
    );

    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(
        estandar,
        "simulación: cell terminate --id c1 -> estado objetivo: retirada\n"
    );
    assert!(diagnostico.is_empty());
}

/// R6 (ratificación 2026-09-22): tras el éxito de los seis pasos de `ciclo_de_vida::retirar`,
/// `ejecutar_con_efectos` persiste `Retirada` con motivo `sesion_cerrada`. Sin fila previa en el
/// almacén, la transición se inserta directamente con el origen vacío, igual que la alta
/// implícita de `cell pause`/`cell unpause` pero con su propio motivo fijo.
#[test]
fn cell_terminate_persiste_retirada_con_motivo_sesion_cerrada() {
    let servidor = ServidorDockerFalso::nuevo("terminate-persiste");
    let ruta = servidor.ruta();
    let receptor = servir_terminate(servidor);

    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("terminate-persiste");
    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "terminate", "--id", "c1", "--confirmar"],
        &cliente,
        &inventario,
        &almacen.texto(),
    );

    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert!(!estandar.is_empty(), "las tres líneas fijas salieron");
    assert_eq!(
        fila_de(&almacen, "c1").as_deref(),
        Some("retirada sesion_cerrada 1700000000000"),
        "la fila queda en retirada con el motivo sesion_cerrada y el reloj inyectado"
    );
    assert_eq!(
        transiciones_de(&almacen),
        ["c1 >retirada sesion_cerrada 1700000000000"],
        "sin fila previa, la transición se inserta con el origen vacío"
    );

    esperar_guion(&receptor);
}

/// R6: si el paso 3 (`POST /admin/sesion/cierre`) falla, `ciclo_de_vida::retirar` devuelve
/// `Err` ANTES de llegar al paso 6, así que `ejecutar_con_efectos` nunca invoca
/// `registrar_transicion`: el almacén queda idéntico byte a byte, igual que en el camino de
/// fallo de Docker de `cell pause`/`cell unpause`.
#[test]
fn cell_terminate_no_toca_el_almacen_cuando_el_cierre_de_sesion_falla() {
    let servidor = ServidorDockerFalso::nuevo("terminate-cierre-fallido");
    let ruta = servidor.ruta();
    let receptor = servir_terminate_con_cierre_fallido(servidor);

    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("terminate-cierre-fallido");
    sembrar_fila(&almacen, "c1", EstadoDeCelula::EnEjecucion);
    let bytes_antes = almacen.bytes();

    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &["cell", "terminate", "--id", "c1", "--confirmar"],
        &cliente,
        &inventario,
        &almacen.texto(),
    );

    assert_eq!(codigo, CodigoDeSalida::Fallo);
    assert!(estandar.is_empty(), "estándar vacío: {estandar:?}");
    assert!(!diagnostico.is_empty(), "el fallo se diagnostica");
    assert_eq!(
        almacen.bytes(),
        bytes_antes,
        "el almacén queda idéntico byte a byte cuando el cierre de sesión falla"
    );
    assert_eq!(
        fila_de(&almacen, "c1").as_deref(),
        Some("en_ejecucion sembrada 1"),
        "la fila sembrada no se toca"
    );
    assert!(transiciones_de(&almacen).is_empty());

    esperar_guion(&receptor);
}

// ─────────────────────────────────────────────────────────────────────────────────────────
// Grupo `reporte tokens` (tarea 23 de la etapa A-6, HEX-084).
// ─────────────────────────────────────────────────────────────────────────────────────────

/// Ruta temporal de copia que no existe en disco: si un camino la abriera, devolvería
/// `Fallo`; usarla para demostrar que el camino no la abre.
fn ruta_de_copia_inexistente(etiqueta: &str) -> std::path::PathBuf {
    let ruta = std::env::temp_dir().join(format!(
        "hexcell-admin-copia-{etiqueta}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&ruta);
    ruta
}

/// AC-4 a nivel de despacho: `--simular` devuelve `Exito` con la línea de simulación en el
/// estándar, sin abrir la copia —la ruta no existe en disco y la prueba pasaría a `Fallo`
/// si algún camino la abriera— y sin diagnóstico.
#[test]
fn reporte_tokens_con_simular_devuelve_exito_y_linea_en_estandar() {
    let copia = ruta_de_copia_inexistente("simular");
    assert!(!copia.exists(), "premisa: la ruta no existe");
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        copia.to_str().unwrap(),
        "--simular",
    ]);
    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(
        estandar,
        format!(
            "simulación: reporte tokens --celula piloto-01 --copia {}\n",
            copia.display()
        )
    );
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");
}

/// AC-3 a nivel de despacho: una copia llamada `sessions.db` es `UsoIncorrecto` con el
/// mensaje exacto por diagnóstico y el estándar vacío, aunque el archivo exista con
/// contenido basura (de haberse abierto, sería `Fallo`). El archivo vive en un directorio
/// temporal propio para no tocar ningún `sessions.db` ajeno del directorio temporal.
#[test]
fn reporte_tokens_rechaza_copia_sessions_db_con_mensaje_exacto() {
    let directorio = std::env::temp_dir().join(format!(
        "hexcell-admin-sessions-falsa-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directorio);
    std::fs::create_dir_all(&directorio).expect("crear el directorio de la sesión falsa");
    let copia = directorio.join("sessions.db");
    std::fs::write(&copia, b"basura").expect("escribir la sesión falsa");
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        copia.to_str().unwrap(),
    ]);
    assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto);
    assert!(estandar.is_empty(), "estándar vacío: {estandar:?}");
    assert!(
        diagnostico.contains("el reporte sólo lee copias VACUUM INTO, nunca sessions.db"),
        "diagnóstico: {diagnostico:?}"
    );
    let _ = std::fs::remove_dir_all(&directorio);
}

/// `ejecutar_con_efectos` delega `reporte tokens` a `ejecutar` sin tocar Docker, igual que
/// `config render`: con una copia inexistente y sin `--simular` el desenlace es `Fallo`
/// (la lectura fracasa), nunca `NoImplementadoTodavia` ni un error del cliente Docker;
/// con `--simular` el desenlace es `Exito`. El cliente apunta a un socket sin vincular a
/// propósito: cualquier intento de tocar Docker sería `DemonioInalcanzable`.
#[test]
fn ejecutar_con_efectos_delega_reporte_tokens_sin_tocar_docker() {
    let ruta = ruta_socket_sin_vincular("efectos-reporte");
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("efectos-reporte");
    let ruta_almacen = almacen.texto();
    let copia = ruta_de_copia_inexistente("delegacion");

    let (codigo, estandar, diagnostico) = ejecutar_con_efectos_con(
        &[
            "reporte",
            "tokens",
            "--celula",
            "c1",
            "--copia",
            copia.to_str().unwrap(),
        ],
        &cliente,
        &inventario,
        &ruta_almacen,
    );
    assert_eq!(codigo, CodigoDeSalida::Fallo, "diag: {diagnostico:?}");
    assert!(estandar.is_empty(), "estándar vacío: {estandar:?}");
    assert!(
        !diagnostico.is_empty(),
        "diagnóstico presente: {diagnostico:?}"
    );

    let (codigo, estandar, _) = ejecutar_con_efectos_con(
        &[
            "reporte",
            "tokens",
            "--celula",
            "c1",
            "--copia",
            copia.to_str().unwrap(),
            "--simular",
        ],
        &cliente,
        &inventario,
        &ruta_almacen,
    );
    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert!(
        estandar.contains("simulación: reporte tokens"),
        "{estandar:?}"
    );
}
