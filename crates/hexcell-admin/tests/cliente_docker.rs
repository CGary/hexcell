//! Tests de integración del cliente del socket Unix de Docker (`hexcell_admin::docker`).
//!
//! Cada test levanta su propio demonio falso sobre un socket Unix temporal (ver `comun`) y ejercita
//! una operación del cliente contra él. Ningún test toca un daemon real ni la red. Los criterios
//! AC-6 (404) y AC-11 (tiempo de espera) llevan una nota en el código que explica por qué deben
//! ser demostrables por mutación: son los dos invariantes que el enunciado señala por nombre.

mod comun;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::time::Duration;

use hexcell_admin::docker::{
    ClienteDocker, ErrorDeClienteDocker, OpcionesDeContenedor, ResultadoDeArranque,
};

use comun::{Guion, ServidorDockerFalso, ruta_socket_sin_vincular};

/// AC-1: crear e iniciar devuelve el identificador del contenedor.
///
/// El identificador sale siempre del cuerpo 201 de `/containers/create` (el motor devuelve `Id`
/// ahí); el cuerpo del arranque es irrelevante para conocerlo.
#[test]
fn crear_e_iniciar_devuelve_el_identificador() {
    let servidor = ServidorDockerFalso::nuevo("ac1");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        let crear = servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"abc123","Warnings":[]}"#,
        });
        let iniciar = servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        });
        (crear, iniciar)
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = cliente.crear_e_iniciar_contenedor("imagen:latest").unwrap();

    assert_eq!(
        resultado,
        ResultadoDeArranque::Iniciado {
            id_contenedor: "abc123".to_string()
        }
    );

    let (crear, iniciar) = hilo.join().unwrap();
    assert_eq!(crear.metodo, "POST");
    assert_eq!(crear.objetivo, "/containers/create");
    assert_eq!(iniciar.metodo, "POST");
    assert_eq!(iniciar.objetivo, "/containers/abc123/start");
}

/// AC-2: detener envía `t=30` al demonio y da la operación por buena sobre el 204.
#[test]
fn detener_envia_el_margen_de_gracia_de_30_segundos() {
    let servidor = ServidorDockerFalso::nuevo("ac2");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    cliente.detener_contenedor("abc123").unwrap();

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.metodo, "POST");
    assert_eq!(peticion.objetivo, "/containers/abc123/stop?t=30");
}

/// AC-3: inspeccionar devuelve el cuerpo JSON interpretado.
///
/// La respuesta viaja en `Transfer-Encoding: chunked` a propósito, para ejercitar el lector de
/// troceado del transporte (el invariante exige tanto Content-Length como chunked).
#[test]
fn inspeccionar_devuelve_el_estado_interpretado() {
    let servidor = ServidorDockerFalso::nuevo("ac3");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::Troceado {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"Id":"abc123","State":{"Status":"running","Running":true}}"#,
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let valor = cliente.inspeccionar_contenedor("abc123").unwrap();

    assert_eq!(
        valor.pointer("/State/Status").and_then(|v| v.as_str()),
        Some("running")
    );

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.metodo, "GET");
    assert_eq!(peticion.objetivo, "/containers/abc123/json");
}

/// AC-4: eliminar un contenedor devuelve éxito sobre el 204.
#[test]
fn eliminar_contenedor_devuelve_exito() {
    let servidor = ServidorDockerFalso::nuevo("ac4");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    cliente.eliminar_contenedor("abc123").unwrap();

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.metodo, "DELETE");
    assert_eq!(peticion.objetivo, "/containers/abc123");
}

/// AC-5: eliminar un volumen devuelve éxito sobre el 204.
#[test]
fn eliminar_volumen_devuelve_exito() {
    let servidor = ServidorDockerFalso::nuevo("ac5");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    cliente.eliminar_volumen("datos-piloto-01").unwrap();

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.metodo, "DELETE");
    assert_eq!(peticion.objetivo, "/volumes/datos-piloto-01");
}

/// AC-6: un 404 en inspección o eliminación se traduce a `NoEncontrado`.
///
/// Este test debe ser demostrable por mutación: si en `cliente.rs` se borra el brazo
/// `404 => ErrorDeClienteDocker::NoEncontrado` de `clasificar_estado`, el 404 cae en
/// `ErrorDelDaemon { estado: 404 }` y esta aserción falla. No basta con que la rama esté cubierta
/// por líneas: el test la señala como requisito.
#[test]
fn no_encontrado_ante_un_404() {
    let servidor = ServidorDockerFalso::nuevo("ac6");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        let _ = servidor.atender(Guion::SinCuerpo {
            estado: 404,
            razon: "Not Found",
        });
        servidor.atender(Guion::SinCuerpo {
            estado: 404,
            razon: "Not Found",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado_de_inspeccion = cliente.inspeccionar_contenedor("ausente");
    assert!(matches!(
        resultado_de_inspeccion,
        Err(ErrorDeClienteDocker::NoEncontrado)
    ));

    let resultado_de_eliminacion = cliente.eliminar_contenedor("ausente");
    assert!(matches!(
        resultado_de_eliminacion,
        Err(ErrorDeClienteDocker::NoEncontrado)
    ));

    hilo.join().unwrap();
}

/// AC-7: un 409 en eliminación se traduce a `Conflicto`, distinto de `NoEncontrado`.
#[test]
fn conflicto_ante_un_409() {
    let servidor = ServidorDockerFalso::nuevo("ac7");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 409,
            razon: "Conflict",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = cliente.eliminar_contenedor("en-ejecucion");

    assert!(matches!(resultado, Err(ErrorDeClienteDocker::Conflicto)));
    assert!(!matches!(
        resultado,
        Err(ErrorDeClienteDocker::NoEncontrado)
    ));

    hilo.join().unwrap();
}

/// AC-8: una respuesta que no es HTTP válido se traduce a `RespuestaMalformada`, sin pánico.
#[test]
fn respuesta_malformada_no_panica() {
    let servidor = ServidorDockerFalso::nuevo("ac8");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::Crudo(b"ESTO NO ES UNA RESPUESTA HTTP\r\n\r\n"))
    });

    let cliente = ClienteDocker::nuevo(ruta);
    // El cierre de pánico cruzaría la frontera del test y lo haría fallar: basta con que la
    // llamada devuelva Err, nunca propague un pánico.
    let resultado = cliente.detener_contenedor("abc123");

    assert!(matches!(
        resultado,
        Err(ErrorDeClienteDocker::RespuestaMalformada { .. })
    ));

    hilo.join().unwrap();
}

/// AC-9: un socket sin demonio detrás se traduce a `DemonioInalcanzable`.
#[test]
fn demonio_inalcanzable_sin_listener() {
    // Ruta temporal que nadie vinculó: `connect` falla con ENOENT, no con un agotamiento.
    let ruta = ruta_socket_sin_vincular("ac9");
    let cliente = ClienteDocker::nuevo(ruta);

    let resultado = cliente.detener_contenedor("abc123");

    assert!(matches!(
        resultado,
        Err(ErrorDeClienteDocker::DemonioInalcanzable)
    ));
}

/// AC-10: un socket con permisos denegados se traduce a `PermisoDenegado`.
///
/// Este test ejecuta SIEMPRE una aserción real, nunca un salto silencioso (un salto indistinguible
/// de un pase es un guardia vacío). Detecta si el proceso corre como root —que sortea los bits de
/// permiso vía `CAP_DAC_OVERRIDE`— y, en ese caso, asevera lo contrario: que `PermisoDenegado` no
/// se devuelve para esa misma petición.
#[test]
fn permiso_denegado_sin_autoridad() {
    // Se vincula un listener solo para que el archivo de socket exista con modo 0000; el listener
    // se mantiene vivo durante el test porque es lo que crea el archivo, no porque atienda nada.
    let ruta = ruta_socket_sin_vincular("ac10");
    let _listener = std::os::unix::net::UnixListener::bind(&ruta)
        .expect("vincular el socket del demonio falso");
    std::fs::set_permissions(&ruta, std::fs::Permissions::from_mode(0o000)).unwrap();

    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_millis(300));
    let resultado = cliente.detener_contenedor("abc123");

    let es_root = std::fs::metadata("/proc/self")
        .map(|m| m.uid() == 0)
        .unwrap_or(false);

    if es_root {
        assert!(
            !matches!(resultado, Err(ErrorDeClienteDocker::PermisoDenegado)),
            "como root, CAP_DAC_OVERRIDE sortea los permisos del socket: no debe darse PermisoDenegado"
        );
    } else {
        assert!(
            matches!(resultado, Err(ErrorDeClienteDocker::PermisoDenegado)),
            "sin privilegios, un socket con modo 0000 debe rechazar con PermisoDenegado"
        );
    }

    let _ = std::fs::remove_file(&ruta);
}

/// AC-11: una respuesta que nunca llega se traduce a `TiempoDeEsperaAgotado`, sin colgar.
///
/// Este test debe ser demostrable por mutación: si en `transporte.rs` se elimina la llamada
/// `set_read_timeout` de `ConexionDocker::conectar_con_tiempo_limite`, la lectura bloquea para
/// siempre y el test cuelga (falla por agotamiento del runner) en vez de devolver el error.
#[test]
fn tiempo_de_espera_agotado_sin_respuesta() {
    let servidor = ServidorDockerFalso::nuevo("ac11");
    let ruta = servidor.ruta();
    // El hilo se aparca para siempre sosteniendo la conexión abierta; no se une.
    std::thread::spawn(move || {
        servidor.atender(Guion::Mudo);
    });

    let cliente = ClienteDocker::con_tiempo_limite(ruta, Duration::from_millis(300));
    let inicio = std::time::Instant::now();
    let resultado = cliente.detener_contenedor("abc123");

    assert!(matches!(
        resultado,
        Err(ErrorDeClienteDocker::TiempoDeEsperaAgotado)
    ));
    // Acotado, no colgado: debe resolverse mucho antes del plazo del runner.
    assert!(inicio.elapsed() < Duration::from_secs(5));
}

/// AC-12: un error 5xx del demonio se traduce a `ErrorDelDaemon` con su código.
#[test]
fn error_del_daemon_ante_un_500() {
    let servidor = ServidorDockerFalso::nuevo("ac12");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::ConCuerpo {
            estado: 500,
            razon: "Internal Server Error",
            cuerpo: br#"{"message":"boom"}"#,
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = cliente.inspeccionar_contenedor("abc123");

    match resultado {
        Err(ErrorDeClienteDocker::ErrorDelDaemon { estado, .. }) => assert_eq!(estado, 500),
        otro => panic!("se esperaba ErrorDelDaemon, se obtuvo {otro:?}"),
    }

    hilo.join().unwrap();
}

/// AC-13: un 304 en el arranque significa que ya estaba en ejecución, con el mismo identificador.
#[test]
fn ya_en_ejecucion_ante_un_304() {
    let servidor = ServidorDockerFalso::nuevo("ac13");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        let crear = servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"abc123","Warnings":[]}"#,
        });
        let iniciar = servidor.atender(Guion::SinCuerpo {
            estado: 304,
            razon: "Not Modified",
        });
        (crear, iniciar)
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = cliente.crear_e_iniciar_contenedor("imagen:latest").unwrap();

    assert_eq!(
        resultado,
        ResultadoDeArranque::YaEnEjecucion {
            id_contenedor: "abc123".to_string()
        }
    );

    let (crear, iniciar) = hilo.join().unwrap();
    assert_eq!(crear.objetivo, "/containers/create");
    assert_eq!(iniciar.objetivo, "/containers/abc123/start");
}

/// `iniciar_contenedor` arranca un contenedor YA CREADO con una sola petición
/// `POST /containers/{id}/start`, sin pasar por `/containers/create`.
#[test]
fn iniciar_contenedor_emite_post_start_y_acepta_204() {
    let servidor = ServidorDockerFalso::nuevo("iniciar-204");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    cliente.iniciar_contenedor("abc123").unwrap();

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.metodo, "POST");
    assert_eq!(peticion.objetivo, "/containers/abc123/start");
}

/// `iniciar_contenedor` acepta un 304: el contenedor ya estaba en ejecución.
#[test]
fn iniciar_contenedor_acepta_304() {
    let servidor = ServidorDockerFalso::nuevo("iniciar-304");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::SinCuerpo {
            estado: 304,
            razon: "Not Modified",
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let resultado = cliente.iniciar_contenedor("abc123");
    assert!(
        resultado.is_ok(),
        "un 304 no debe ser un error: {resultado:?}"
    );

    hilo.join().unwrap();
}

/// `crear_e_iniciar_contenedor_con_opciones` añade `HostConfig.NetworkMode` y `Cmd` al cuerpo de
/// creación, que es lo que hace falta para que la sonda hermana viva dentro de la red de la
/// célula y ejecute su bucle de sondeo.
#[test]
fn crear_e_iniciar_contenedor_con_opciones_envia_red_y_cmd() {
    let servidor = ServidorDockerFalso::nuevo("crear-con-opciones");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        let crear = servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        });
        let iniciar = servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        });
        (crear, iniciar)
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let opciones = OpcionesDeContenedor {
        red: "hexcell-c1-red".to_string(),
        cmd: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo hola".to_string(),
        ],
    };
    let resultado = cliente
        .crear_e_iniciar_contenedor_con_opciones("alpine:3", opciones)
        .unwrap();

    assert_eq!(
        resultado,
        ResultadoDeArranque::Iniciado {
            id_contenedor: "sonda1".to_string()
        }
    );

    let (crear, iniciar) = hilo.join().unwrap();
    assert_eq!(crear.objetivo, "/containers/create");
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear.cuerpo).unwrap();
    assert_eq!(cuerpo["Image"], "alpine:3");
    assert_eq!(cuerpo["HostConfig"]["NetworkMode"], "hexcell-c1-red");
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!(["/bin/sh", "-c", "echo hola"])
    );
    assert_eq!(iniciar.objetivo, "/containers/sonda1/start");
}

/// `esperar_contenedor` emite `POST /containers/{id}/wait` y lee `StatusCode` del cuerpo 200.
#[test]
fn esperar_contenedor_lee_el_codigo_de_salida_del_cuerpo() {
    let servidor = ServidorDockerFalso::nuevo("esperar");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":0}"#,
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let codigo = cliente.esperar_contenedor("sonda1").unwrap();
    assert_eq!(codigo, 0);

    let peticion = hilo.join().unwrap();
    assert_eq!(peticion.metodo, "POST");
    assert_eq!(peticion.objetivo, "/containers/sonda1/wait");
}

/// `esperar_contenedor` propaga un código de salida distinto de 0 sin traducirlo a error: quien
/// decide si eso es un fallo es la capa de ciclo de vida, no el cliente Docker.
#[test]
fn esperar_contenedor_propaga_un_codigo_de_salida_distinto_de_cero() {
    let servidor = ServidorDockerFalso::nuevo("esperar-no-cero");
    let ruta = servidor.ruta();
    let hilo = std::thread::spawn(move || {
        servidor.atender(Guion::ConCuerpo {
            estado: 200,
            razon: "OK",
            cuerpo: br#"{"StatusCode":137}"#,
        })
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let codigo = cliente.esperar_contenedor("sonda1").unwrap();
    assert_eq!(codigo, 137);

    hilo.join().unwrap();
}

/// Si la creación devuelve 201 pero el arranque falla, el contenedor recién creado NO se queda
/// huérfano: el cliente emite su `DELETE` en el mejor esfuerzo antes de propagar el error, que
/// sigue siendo el del arranque (500 del demonio) y no el de la limpieza.
#[test]
fn crear_e_iniciar_con_opciones_elimina_el_contenedor_si_falla_el_arranque() {
    let servidor = ServidorDockerFalso::nuevo("crear-arranque-falla");
    let ruta = servidor.ruta();
    let (emisor, receptor) = std::sync::mpsc::channel();
    let _hilo = std::thread::spawn(move || {
        let _ = emisor.send(servidor.atender(Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        }));
        let _ = emisor.send(servidor.atender(Guion::SinCuerpo {
            estado: 500,
            razon: "Internal Server Error",
        }));
        let _ = emisor.send(servidor.atender(Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        }));
    });

    let cliente = ClienteDocker::nuevo(ruta);
    let opciones = OpcionesDeContenedor {
        red: "red-del-operador".to_string(),
        cmd: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "exit 1".to_string(),
        ],
    };
    let resultado = cliente.crear_e_iniciar_contenedor_con_opciones("sonda-de-prueba:1", opciones);

    match resultado {
        Err(ErrorDeClienteDocker::ErrorDelDaemon { estado: 500, .. }) => {}
        otro => panic!("se esperaba el 500 del arranque propagado, se obtuvo {otro:?}"),
    }

    let cota = std::time::Duration::from_secs(10);
    let mut secuencia = Vec::new();
    for _ in 0..3 {
        let peticion = receptor
            .recv_timeout(cota)
            .expect("el DELETE del contenedor huérfano debe llegar dentro del límite");
        secuencia.push(format!("{} {}", peticion.metodo, peticion.objetivo));
    }
    assert_eq!(
        secuencia,
        vec![
            "POST /containers/create".to_string(),
            "POST /containers/sonda1/start".to_string(),
            "DELETE /containers/sonda1".to_string(),
        ]
    );
}
