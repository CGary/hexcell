//! Tests de integración del ciclo de vida de la célula (`hexcell_admin::ciclo_de_vida`).
//!
//! Cada test levanta su propio demonio falso sobre un socket Unix temporal (ver `comun`) y
//! ejercita `pausar` o `reanudar` contra él. Ningún test toca un daemon real ni la red, y ninguno
//! sondea `/health/ready` directamente: esa ruta solo existe dentro del `Cmd` del contenedor
//! hermano, nunca en el proceso de este binario.
//!
//! Dos reglas hacen que estas guardas puedan ponerse rojas. Primera: **ningún accesorio coincide
//! con el valor por omisión de producción** —red `red-del-operador`, puerto 9099, imagen
//! `sonda-de-prueba:1`, límite 45 s—, porque si coincidiera la aserción se movería junto con
//! aquello que debía fijar. Segunda: **ningún test se cuelga**; las peticiones viajan por un canal
//! leído con `recv_timeout`, nunca con un `join` ciego, así que una que falte pone el test rojo
//! dentro del límite. Un test colgado es peor que uno rojo.

mod comun;

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use hexcell_admin::ciclo_de_vida::{self, DatosDeSondeo, ErrorDeCicloDeVida, NombresDeCelula};
use hexcell_admin::docker::{ClienteDocker, ErrorDeClienteDocker};

use comun::{Guion, PeticionRecibida, ServidorDockerFalso};

/// Cota de espera: finita, para que una petición que nunca llega se note como fallo y no como
/// cuelgue.
const LIMITE_DE_RECEPCION: Duration = Duration::from_secs(10);

/// Red del accesorio: distinta de `hexcell-{id}-red`, para que derivar la red del `--id` se ponga
/// rojo. Igual la imagen frente a `IMAGEN_DE_SONDA_POR_OMISION` y el límite frente a
/// `LIMITE_DE_SONDEO_S`.
const RED_DEL_ACCESORIO: &str = "red-del-operador";
const IMAGEN_DEL_ACCESORIO: &str = "sonda-de-prueba:1";
const LIMITE_DEL_ACCESORIO: u64 = 45;

fn sin_cuerpo(estado: u16, razon: &'static str) -> Guion {
    Guion::SinCuerpo { estado, razon }
}

fn datos_de_sondeo() -> DatosDeSondeo {
    DatosDeSondeo {
        imagen: IMAGEN_DEL_ACCESORIO.to_string(),
        limite_segundos: LIMITE_DEL_ACCESORIO,
    }
}

fn inspeccion_del_nucleo() -> Guion {
    Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: br#"{"NetworkSettings":{"Networks":{"red-del-operador":{"NetworkID":"n1"}}},"Config":{"Env":["PATH=/usr/bin","HEXCELL_DIRECCION_SALUD=0.0.0.0:9099"]}}"#,
    }
}

/// Atiende las siete peticiones de una reanudación y las reenvía por el canal. La séptima es la
/// limpieza de la sonda: si producción se la saltara, el `recibir` que la exige falla dentro del
/// límite en vez de colgar el test.
fn servir_reanudacion(
    servidor: &ServidorDockerFalso,
    emisor: &Sender<PeticionRecibida>,
    codigo_de_espera: &'static [u8],
) {
    let _ = emisor.send(servidor.atender(sin_cuerpo(204, "No Content"))); // iniciar núcleo
    let _ = emisor.send(servidor.atender(sin_cuerpo(204, "No Content"))); // iniciar sidecar
    let _ = emisor.send(servidor.atender(inspeccion_del_nucleo()));
    let _ = emisor.send(servidor.atender(Guion::ConCuerpo {
        estado: 201,
        razon: "Created",
        cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
    }));
    let _ = emisor.send(servidor.atender(sin_cuerpo(204, "No Content"))); // iniciar sonda
    let _ = emisor.send(servidor.atender(Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: codigo_de_espera,
    }));
    let _ = emisor.send(servidor.atender(sin_cuerpo(204, "No Content"))); // eliminar sonda
}

fn recibir(receptor: &Receiver<PeticionRecibida>) -> PeticionRecibida {
    receptor
        .recv_timeout(LIMITE_DE_RECEPCION)
        .expect("el demonio falso debía haber atendido otra petición dentro del límite")
}

/// AC-1 + AC-2 + AC-3: `pausar` detiene el sidecar estrictamente antes que el núcleo, con
/// exactamente dos peticiones de parada y sin fijar el plazo desde la CLI.
///
/// El orden se comprueba sobre la SECUENCIA completa de peticiones, no con dos aserciones de
/// presencia independientes: la aserción compara el `Vec` entero, así que se pone roja si alguien
/// invierte el orden. Esa misma comparación literal fija AC-2 por tres lados: las rutas son
/// `/stop` pelado —ningún `?t=`, de modo que el `stop_grace_period` de la plantilla queda como
/// única fuente de verdad del plazo—, son `/stop` y no `/pause` —los dos contenedores acaban en
/// `exited`, nunca en `paused`— y el sidecar lleva la misma parada con gracia que el núcleo,
/// porque tiene que cerrar su websocket saliente y dejar su almacén consistente.
#[test]
fn pausar_detiene_ambos_contenedores_en_orden_y_sin_plazo_explicito() {
    let servidor = ServidorDockerFalso::nuevo("pausar-orden");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let (emisor, receptor) = std::sync::mpsc::channel();
    let _hilo = std::thread::spawn(move || {
        let _ = emisor.send(servidor.atender(sin_cuerpo(204, "No Content")));
        let _ = emisor.send(servidor.atender(sin_cuerpo(204, "No Content")));
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::pausar(&cliente, &nombres);
    assert!(resultado.is_ok(), "pausar debe tener éxito: {resultado:?}");

    let sidecar = recibir(&receptor);
    let nucleo = recibir(&receptor);
    assert_eq!(sidecar.metodo, "POST");
    assert_eq!(nucleo.metodo, "POST");
    assert_eq!(
        vec![sidecar.objetivo, nucleo.objetivo],
        vec![
            "/containers/c1-sidecar/stop".to_string(),
            "/containers/c1-nucleo/stop".to_string(),
        ],
        "el sidecar para estrictamente antes que el núcleo y ninguna parada lleva `t`"
    );
}

/// Si la parada del sidecar falla, la parada del núcleo NUNCA se intenta: el error se propaga de
/// inmediato.
///
/// La aserción exige el error EXACTO que el demonio falso devolvió —`ErrorDelDaemon` con estado
/// 500—, no un error de Docker cualquiera. Con `Docker(_)` bastaba con que algo fallara: si
/// alguien ignorase el fallo del sidecar y siguiera al núcleo, la segunda conexión moriría con
/// `DemonioInalcanzable` y ese `Docker(_)` seguiría pasando, sin probar en absoluto que el núcleo
/// no se intentó.
#[test]
fn pausar_propaga_el_error_del_sidecar_sin_intentar_el_nucleo() {
    let servidor = ServidorDockerFalso::nuevo("pausar-falla-sidecar");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let (emisor, receptor) = std::sync::mpsc::channel();
    let _hilo = std::thread::spawn(move || {
        let _ = emisor.send(servidor.atender(sin_cuerpo(500, "Internal Server Error")));
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::pausar(&cliente, &nombres);

    match resultado {
        Err(ErrorDeCicloDeVida::Docker(ErrorDeClienteDocker::ErrorDelDaemon {
            estado: 500,
            ..
        })) => {}
        otro => panic!("se esperaba el 500 del sidecar propagado tal cual, se obtuvo {otro:?}"),
    }

    assert_eq!(recibir(&receptor).objetivo, "/containers/c1-sidecar/stop");
}

/// El `Cmd` de la sonda se aserta como literal EXACTO, no por subcadenas, y el literal lo escribe
/// este test sin importar ninguna constante de producción: si tomara la cadencia o las
/// iteraciones del código bajo prueba, una mutación movería los dos lados a la vez. Fija de una
/// sola vez las iteraciones (600 = 60 s a 100 ms), el `exit 0` DENTRO de la rama de éxito de
/// `wget` —agotar el límite nunca puede devolver 0— y el `exit 1` posterior al bucle.
#[test]
fn guion_de_sonda_es_el_literal_exacto_con_su_cadencia_y_sus_codigos_de_salida() {
    let guion = ciclo_de_vida::guion_de_sonda("http://c1-nucleo:9099/health/ready");

    assert_eq!(guion[0], "/bin/sh");
    assert_eq!(guion[1], "-c");
    assert_eq!(
        guion[2],
        "i=0; while [ \"$i\" -lt 600 ]; do if wget -q -O /dev/null \"http://c1-nucleo:9099/health/ready\"; then exit 0; fi; i=$((i+1)); sleep 0.1; done; exit 1",
        "el guion de la sonda cambió: 600 intentos a 100 ms (sleep 0.1), exit 0 solo tras el 200 OK de wget y exit 1 al agotar el límite"
    );
}

/// AC-4: `reanudar` arranca ambos contenedores, inspecciona el núcleo, crea la sonda hermana en
/// la red y con el puerto LEÍDOS de esa inspección, con la imagen y el límite que le llegan en
/// `DatosDeSondeo`, espera su código de salida y devuelve éxito cuando ese código es 0. Los
/// cuatro valores del accesorio divergen del valor por omisión, así que cada uno es una guarda
/// viva: codificar 8081, derivar la red del `--id`, usar la constante de imagen en vez de
/// `datos.imagen` o la de límite en vez de `datos.limite_segundos` pone roja esta aserción.
#[test]
fn reanudar_arranca_inspecciona_crea_la_sonda_y_tiene_exito_con_200_ok() {
    let servidor = ServidorDockerFalso::nuevo("reanudar-exito");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let (emisor, receptor) = std::sync::mpsc::channel();
    let _hilo =
        std::thread::spawn(move || servir_reanudacion(&servidor, &emisor, br#"{"StatusCode":0}"#));

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::reanudar(&cliente, &nombres, &datos_de_sondeo());
    assert!(
        resultado.is_ok(),
        "reanudar debe tener éxito: {resultado:?}"
    );

    assert_eq!(recibir(&receptor).objetivo, "/containers/c1-nucleo/start");
    assert_eq!(recibir(&receptor).objetivo, "/containers/c1-sidecar/start");
    assert_eq!(recibir(&receptor).objetivo, "/containers/c1-nucleo/json");

    let crear_sonda = recibir(&receptor);
    assert_eq!(crear_sonda.objetivo, "/containers/create");
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_sonda.cuerpo).unwrap();
    assert_eq!(
        cuerpo["Image"], "sonda-de-prueba:1",
        "la imagen sale de DatosDeSondeo, no de la constante por omisión"
    );
    assert_eq!(
        cuerpo["HostConfig"]["NetworkMode"], RED_DEL_ACCESORIO,
        "la red sale de la inspección, no del --id"
    );
    // Literal exacto: el puerto 9099 sale de HEXCELL_DIRECCION_SALUD y los 450 intentos del
    // límite de 45 s que llegó en DatosDeSondeo, ambos distintos de los valores por omisión.
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "/bin/sh",
            "-c",
            "i=0; while [ \"$i\" -lt 450 ]; do if wget -q -O /dev/null \"http://c1-nucleo:9099/health/ready\"; then exit 0; fi; i=$((i+1)); sleep 0.1; done; exit 1"
        ])
    );

    assert_eq!(recibir(&receptor).objetivo, "/containers/sonda1/start");
    assert_eq!(recibir(&receptor).objetivo, "/containers/sonda1/wait");
    let eliminar_sonda = recibir(&receptor);
    assert_eq!(eliminar_sonda.objetivo, "/containers/sonda1");
    assert_eq!(eliminar_sonda.metodo, "DELETE");
}

/// AC-5: cuando la sonda agota su límite sin un 200 OK (código de salida distinto de 0),
/// `reanudar` falla con un mensaje explícito que nombra el límite excedido en segundos, y limpia
/// siempre el contenedor de sonda, también en el camino de fallo. El límite del accesorio es 45,
/// no el 60 por omisión: el mensaje solo puede nombrarlo si sale de `DatosDeSondeo`. El `DELETE`
/// se lee del canal con `recv_timeout`, así que saltarse la limpieza pone el test rojo dentro del
/// límite en vez de dejarlo colgado en un `join` que nunca vuelve.
#[test]
fn reanudar_falla_con_mensaje_explicito_cuando_la_sonda_agota_el_limite() {
    let servidor = ServidorDockerFalso::nuevo("reanudar-timeout");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let (emisor, receptor) = std::sync::mpsc::channel();
    let _hilo =
        std::thread::spawn(move || servir_reanudacion(&servidor, &emisor, br#"{"StatusCode":1}"#));

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::reanudar(&cliente, &nombres, &datos_de_sondeo());

    match &resultado {
        Err(ErrorDeCicloDeVida::TiempoDeSondeoAgotado { limite_segundos }) => {
            assert_eq!(*limite_segundos, LIMITE_DEL_ACCESORIO);
        }
        otro => panic!("se esperaba TiempoDeSondeoAgotado, se obtuvo {otro:?}"),
    }
    let mensaje = resultado.unwrap_err().to_string();
    assert!(
        mensaje.contains("45"),
        "el mensaje debe nombrar el límite excedido que llegó en DatosDeSondeo: {mensaje}"
    );
    assert!(
        mensaje.to_lowercase().contains("segundos"),
        "el mensaje debe ser explícito sobre la unidad: {mensaje}"
    );

    // Camino de fallo: la sonda se elimina de todos modos. Se descartan las seis peticiones
    // previas; la séptima es la que esta guarda existe para exigir.
    (0..6).for_each(|_| drop(recibir(&receptor)));
    let eliminar_sonda = recibir(&receptor);
    assert_eq!(eliminar_sonda.metodo, "DELETE");
    assert_eq!(eliminar_sonda.objetivo, "/containers/sonda1");
}

/// RIESGO-1: cuando el demonio responde 404 al crear la sonda (la imagen auxiliar no está en el
/// disco del anfitrión), `reanudar` traduce el fallo a un error que NOMBRA la imagen ausente, no
/// a un "el recurso no existe" genérico. La imagen del accesorio no es la de la constante por
/// omisión, así que el nombre solo puede salir de `DatosDeSondeo`.
#[test]
fn reanudar_nombra_la_imagen_ausente_ante_un_404_al_crear_la_sonda() {
    let servidor = ServidorDockerFalso::nuevo("reanudar-imagen-ausente");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let _hilo = std::thread::spawn(move || {
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar núcleo
        servidor.atender(sin_cuerpo(204, "No Content")); // iniciar sidecar
        servidor.atender(inspeccion_del_nucleo());
        servidor.atender(sin_cuerpo(404, "Not Found"));
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::reanudar(&cliente, &nombres, &datos_de_sondeo());

    match &resultado {
        Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada { imagen }) => {
            assert_eq!(imagen, IMAGEN_DEL_ACCESORIO);
        }
        otro => panic!("se esperaba ImagenDeSondaNoEncontrada, se obtuvo {otro:?}"),
    }
    let mensaje = resultado.unwrap_err().to_string();
    assert!(
        mensaje.contains("sonda-de-prueba:1"),
        "el mensaje debe nombrar la imagen ausente: {mensaje}"
    );
}
