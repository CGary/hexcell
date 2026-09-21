//! Tests de integración del ciclo de vida de la célula (`hexcell_admin::ciclo_de_vida`).
//!
//! Cada test levanta su propio demonio falso sobre un socket Unix temporal (ver `comun`) y
//! ejercita `pausar` o `reanudar` contra él. Ningún test toca un daemon real ni la red, y ninguno
//! sondea `/health/ready` directamente: esa ruta solo existe dentro del `Cmd` del contenedor
//! hermano, nunca en el proceso de este binario.

mod comun;

use hexcell_admin::ciclo_de_vida::{self, DatosDeSondeo, ErrorDeCicloDeVida, NombresDeCelula};
use hexcell_admin::docker::ClienteDocker;
use hexcell_admin::estado_de_celula::EstadoDeCelula;

use comun::{Guion, ServidorDockerFalso};

fn respuesta_204() -> Guion {
    Guion::SinCuerpo {
        estado: 204,
        razon: "No Content",
    }
}

/// AC-1 + AC-3: `pausar` detiene el sidecar estrictamente antes que el núcleo, con exactamente
/// dos peticiones, ambas de parada.
///
/// El orden se comprueba sobre la SECUENCIA de peticiones que el demonio falso recibió, no con
/// dos aserciones de presencia independientes: la aserción compara el `Vec` completo, así que se
/// pone roja si alguien invierte el orden en `ciclo_de_vida::pausar`. Este es el test que la
/// verificación por mutación de esta tarea invierte a mano.
#[test]
fn pausar_detiene_el_sidecar_antes_que_el_nucleo() {
    let servidor = ServidorDockerFalso::nuevo("pausar-orden");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let hilo = std::thread::spawn(move || {
        let primera = servidor.atender(respuesta_204());
        let segunda = servidor.atender(respuesta_204());
        vec![primera.objetivo, segunda.objetivo]
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::pausar(&cliente, &nombres);
    assert!(resultado.is_ok(), "pausar debe tener éxito: {resultado:?}");

    let secuencia = hilo.join().unwrap();
    assert_eq!(
        secuencia,
        vec![
            "/containers/c1-sidecar/stop?t=30".to_string(),
            "/containers/c1-nucleo/stop?t=30".to_string(),
        ],
        "la parada del sidecar debe llegar estrictamente antes que la del núcleo"
    );
}

/// AC-2: la parada del núcleo pide el margen de gracia de 30 s (vehículo del `SIGTERM` anclado
/// por `STOPSIGNAL`), y la tabla de transiciones ya vigente admite `EnEjecucion -> Suspendida`
/// como el destino en memoria de una pausa exitosa.
#[test]
fn pausar_pide_la_gracia_de_30s_para_el_nucleo_y_suspendida_es_destino_legal() {
    let servidor = ServidorDockerFalso::nuevo("pausar-gracia");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let hilo = std::thread::spawn(move || {
        let _sidecar = servidor.atender(respuesta_204());
        servidor.atender(respuesta_204())
    });

    let cliente = ClienteDocker::nuevo(ruta);
    ciclo_de_vida::pausar(&cliente, &nombres).expect("pausar debe tener éxito");

    let peticion_de_nucleo = hilo.join().unwrap();
    assert_eq!(peticion_de_nucleo.metodo, "POST");
    assert_eq!(
        peticion_de_nucleo.objetivo,
        "/containers/c1-nucleo/stop?t=30"
    );

    // El plano de control persistido queda fuera de alcance de esta tarea (non-goal), así que la
    // garantía verificable aquí es que la tabla de transiciones ya entregada admite el destino
    // que esta CLI declara alcanzar.
    assert!(EstadoDeCelula::EnEjecucion.permite(EstadoDeCelula::Suspendida));
}

/// Si la parada del sidecar falla, la parada del núcleo NUNCA se intenta: el error se propaga de
/// inmediato. El demonio falso solo atiende una petición; si `pausar` intentara una segunda
/// conexión con el hilo ya terminado, obtendría un error de conexión distinto, nunca un `Ok`.
#[test]
fn pausar_propaga_el_error_del_sidecar_sin_intentar_el_nucleo() {
    let servidor = ServidorDockerFalso::nuevo("pausar-falla-sidecar");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 500,
            razon: "Internal Server Error",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::pausar(&cliente, &nombres);

    match resultado {
        Err(ErrorDeCicloDeVida::Docker(_)) => {}
        otro => panic!("se esperaba un error de Docker propagado, se obtuvo {otro:?}"),
    }

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.objetivo, "/containers/c1-sidecar/stop?t=30");
}

/// El `Cmd` de la sonda escribe la cadencia de 100 ms como `sleep 0.1` (BusyBox no admite
/// milisegundos): el test fija este literal por su cuenta, sin importar la constante de
/// producción que la fija, para que una mutación de esa constante no mueva los dos lados de la
/// aserción a la vez.
#[test]
fn guion_de_sonda_codifica_la_cadencia_de_cien_ms_y_la_url() {
    let guion = ciclo_de_vida::guion_de_sonda("http://c1-nucleo:8081/health/ready");

    assert_eq!(guion[0], "/bin/sh");
    assert_eq!(guion[1], "-c");
    // 100 ms de cadencia == 0.1 s de `sleep`; literal escrito aquí, no derivado de producción.
    assert!(
        guion[2].contains("sleep 0.1"),
        "el guion debe dormir 0.1 s (100 ms) entre intentos: {}",
        guion[2]
    );
    assert!(
        guion[2].contains("wget"),
        "el guion debe usar wget: {}",
        guion[2]
    );
    assert!(
        guion[2].contains("http://c1-nucleo:8081/health/ready"),
        "el guion debe sondear la URL exacta: {}",
        guion[2]
    );
}

/// AC-4: `reanudar` arranca ambos contenedores, inspecciona el núcleo, crea la sonda hermana en
/// la red leída de la inspección con la URL derivada de `HEXCELL_DIRECCION_SALUD`, espera su
/// código de salida y devuelve éxito cuando ese código es 0. El sondeo de 100 ms nunca se emite
/// desde este proceso: solo viaja dentro del `Cmd` que se manda a crear.
#[test]
fn reanudar_arranca_inspecciona_crea_la_sonda_y_tiene_exito_con_200_ok() {
    let servidor = ServidorDockerFalso::nuevo("reanudar-exito");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let datos = DatosDeSondeo {
        imagen: "alpine:3".to_string(),
        limite_segundos: 60,
    };

    let hilo = std::thread::spawn(move || {
        let iniciar_nucleo = servidor.atender(respuesta_204());
        let iniciar_sidecar = servidor.atender(respuesta_204());
        let inspeccionar = servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"NetworkSettings":{"Networks":{"hexcell-c1-red":{"NetworkID":"n1"}}},"Config":{"Env":["PATH=/usr/bin","HEXCELL_DIRECCION_SALUD=0.0.0.0:8081"]}}"#,
        });
        let crear_sonda = servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        });
        let iniciar_sonda = servidor.atender(respuesta_204());
        let esperar_sonda = servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":0}"#,
        });
        let eliminar_sonda = servidor.atender(respuesta_204());
        (
            iniciar_nucleo,
            iniciar_sidecar,
            inspeccionar,
            crear_sonda,
            iniciar_sonda,
            esperar_sonda,
            eliminar_sonda,
        )
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::reanudar(&cliente, &nombres, &datos);
    assert!(
        resultado.is_ok(),
        "reanudar debe tener éxito: {resultado:?}"
    );

    let (
        iniciar_nucleo,
        iniciar_sidecar,
        inspeccionar,
        crear_sonda,
        iniciar_sonda,
        esperar_sonda,
        eliminar_sonda,
    ) = hilo.join().unwrap();

    assert_eq!(iniciar_nucleo.objetivo, "/containers/c1-nucleo/start");
    assert_eq!(iniciar_sidecar.objetivo, "/containers/c1-sidecar/start");
    assert_eq!(inspeccionar.objetivo, "/containers/c1-nucleo/json");
    assert_eq!(crear_sonda.objetivo, "/containers/create");

    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_sonda.cuerpo).unwrap();
    assert_eq!(cuerpo["Image"], "alpine:3");
    assert_eq!(cuerpo["HostConfig"]["NetworkMode"], "hexcell-c1-red");
    let cmd = cuerpo["Cmd"].as_array().expect("Cmd es un arreglo");
    let guion = cmd.last().unwrap().as_str().unwrap();
    assert!(
        guion.contains("http://c1-nucleo:8081/health/ready"),
        "la sonda debe apuntar a la dirección de salud leída de la inspección: {guion}"
    );
    assert!(
        guion.contains("sleep 0.1"),
        "la sonda debe dormir 0.1 s (100 ms) entre intentos: {guion}"
    );

    assert_eq!(iniciar_sonda.objetivo, "/containers/sonda1/start");
    assert_eq!(esperar_sonda.objetivo, "/containers/sonda1/wait");
    assert_eq!(eliminar_sonda.objetivo, "/containers/sonda1");
    assert_eq!(eliminar_sonda.metodo, "DELETE");
}

/// AC-5: cuando la sonda agota su límite sin un 200 OK (código de salida distinto de 0),
/// `reanudar` falla con un mensaje explícito que nombra el límite excedido en segundos, y limpia
/// siempre el contenedor de sonda, también en el camino de fallo.
#[test]
fn reanudar_falla_con_mensaje_explicito_cuando_la_sonda_agota_el_limite() {
    let servidor = ServidorDockerFalso::nuevo("reanudar-timeout");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let datos = DatosDeSondeo {
        imagen: "alpine:3".to_string(),
        limite_segundos: 60,
    };

    let hilo = std::thread::spawn(move || {
        servidor.atender(respuesta_204()); // iniciar núcleo
        servidor.atender(respuesta_204()); // iniciar sidecar
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"NetworkSettings":{"Networks":{"hexcell-c1-red":{"NetworkID":"n1"}}},"Config":{"Env":["HEXCELL_DIRECCION_SALUD=0.0.0.0:8081"]}}"#,
        });
        servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        });
        servidor.atender(respuesta_204()); // iniciar sonda
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":1}"#,
        });
        let eliminar_sonda = servidor.atender(respuesta_204());
        eliminar_sonda
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::reanudar(&cliente, &nombres, &datos);

    match &resultado {
        Err(ErrorDeCicloDeVida::TiempoDeSondeoAgotado { limite_segundos }) => {
            assert_eq!(*limite_segundos, 60);
        }
        otro => panic!("se esperaba TiempoDeSondeoAgotado, se obtuvo {otro:?}"),
    }
    let mensaje = resultado.unwrap_err().to_string();
    assert!(
        mensaje.contains("60"),
        "el mensaje debe nombrar el límite excedido en segundos: {mensaje}"
    );
    assert!(
        mensaje.to_lowercase().contains("segundos"),
        "el mensaje debe ser explícito sobre la unidad: {mensaje}"
    );

    // Camino de fallo: el contenedor de sonda se elimina de todos modos.
    let eliminar_sonda = hilo.join().unwrap();
    assert_eq!(eliminar_sonda.metodo, "DELETE");
    assert_eq!(eliminar_sonda.objetivo, "/containers/sonda1");
}

/// RIESGO-1: cuando el demonio responde 404 al crear la sonda (la imagen auxiliar no está en el
/// disco del anfitrión), `reanudar` traduce el fallo a un error que NOMBRA la imagen ausente, no
/// a un "el recurso no existe" genérico.
#[test]
fn reanudar_nombra_la_imagen_ausente_ante_un_404_al_crear_la_sonda() {
    let servidor = ServidorDockerFalso::nuevo("reanudar-imagen-ausente");
    let ruta = servidor.ruta();
    let nombres = NombresDeCelula::nueva("c1");
    let datos = DatosDeSondeo {
        imagen: "alpine:3".to_string(),
        limite_segundos: 60,
    };

    let hilo = std::thread::spawn(move || {
        servidor.atender(respuesta_204()); // iniciar núcleo
        servidor.atender(respuesta_204()); // iniciar sidecar
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"NetworkSettings":{"Networks":{"hexcell-c1-red":{"NetworkID":"n1"}}},"Config":{"Env":["HEXCELL_DIRECCION_SALUD=0.0.0.0:8081"]}}"#,
        });
        servidor.atender(Guion::SinCuerpo {
            estado: 404,
            razon: "Not Found",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = ciclo_de_vida::reanudar(&cliente, &nombres, &datos);

    match &resultado {
        Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada { imagen }) => {
            assert_eq!(imagen, "alpine:3");
        }
        otro => panic!("se esperaba ImagenDeSondaNoEncontrada, se obtuvo {otro:?}"),
    }
    let mensaje = resultado.unwrap_err().to_string();
    assert!(
        mensaje.contains("alpine:3"),
        "el mensaje debe nombrar la imagen ausente: {mensaje}"
    );

    hilo.join().unwrap();
}
