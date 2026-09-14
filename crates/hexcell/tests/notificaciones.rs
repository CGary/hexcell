//! Tests del puerto de notificación: el sumidero simulado, el selector estático de la célula y el
//! sumidero Telegram contra un servidor de loopback falso, sin ninguna llamada de red real.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use hexcell::notificacion::{SumideroDeCelula, SumideroSimulado};
use hexcell::notificador_telegram::{ConfiguracionDeTelegram, NotificadorTelegram};
use hexcell_core::identidad::IdConversacion;
use hexcell_core::notificacion::{
    CodigoDeNotificacion, ComponenteDeCelula, Notificacion, SumideroDeNotificaciones, ValorDeDato,
};

#[tokio::test]
async fn el_sumidero_simulado_captura_una_notificacion_sin_red() {
    let sumidero = SumideroSimulado::nuevo();
    let notificacion = Notificacion::nueva(CodigoDeNotificacion::nuevo("codigo-de-prueba"));

    sumidero
        .notificar(notificacion.clone())
        .await
        .expect("el sumidero simulado no debe fallar por defecto");

    assert_eq!(sumidero.cantidad(), 1);
    assert_eq!(sumidero.notificaciones(), vec![notificacion]);
}

#[tokio::test]
async fn el_sumidero_simulado_que_falla_devuelve_err_y_no_captura_nada() {
    let sumidero = SumideroSimulado::que_falla();
    let notificacion = Notificacion::nueva(CodigoDeNotificacion::nuevo("codigo-de-prueba"));

    let error = sumidero
        .notificar(notificacion)
        .await
        .expect_err("el sumidero configurado para fallar debe devolver Err");
    assert!(format!("{error}").contains("avería"));
    assert_eq!(sumidero.cantidad(), 0);
}

#[tokio::test]
async fn el_payload_admite_instante_conversacion_y_componente_y_rondan_intactos() {
    let sumidero = SumideroSimulado::nuevo();
    let instante = SystemTime::UNIX_EPOCH + Duration::from_secs(1_757_000_000);
    let conversacion = IdConversacion::nuevo("conv-afectada-1");

    let notificacion = Notificacion::nueva(CodigoDeNotificacion::nuevo("codigo-de-prueba"))
        .con_dato("expira_en", ValorDeDato::Instante(instante))
        .con_dato(
            "conversacion_afectada",
            ValorDeDato::Conversacion(conversacion.clone()),
        )
        .con_dato(
            "contenedor_en_bucle",
            ValorDeDato::Componente(ComponenteDeCelula::Sidecar),
        );

    sumidero
        .notificar(notificacion.clone())
        .await
        .expect("no debe fallar");

    let capturadas = sumidero.notificaciones();
    assert_eq!(capturadas.len(), 1);
    let capturada = &capturadas[0];
    assert_eq!(capturada.codigo, notificacion.codigo);
    assert_eq!(capturada.datos.len(), 3);
    assert_eq!(capturada.datos[0].valor, ValorDeDato::Instante(instante));
    assert_eq!(
        capturada.datos[1].valor,
        ValorDeDato::Conversacion(conversacion)
    );
    assert_eq!(
        capturada.datos[2].valor,
        ValorDeDato::Componente(ComponenteDeCelula::Sidecar)
    );
}

#[tokio::test]
async fn el_selector_de_celula_elige_simulado_sin_configuracion_y_telegram_con_ella() {
    let simulado = SumideroDeCelula::desde_configuracion(None);
    assert!(matches!(simulado, SumideroDeCelula::Simulado(_)));

    let telegram = SumideroDeCelula::desde_configuracion(Some(ConfiguracionDeTelegram {
        url_base: "http://127.0.0.1:9".to_string(),
        token: "token-de-prueba".to_string(),
        id_chat: "123".to_string(),
        timeout: Duration::from_millis(100),
    }));
    assert!(matches!(telegram, SumideroDeCelula::Telegram(_)));
}

struct ServidorFalso {
    puerto: u16,
    contador: Arc<AtomicUsize>,
    ultimo_cuerpo: Arc<std::sync::Mutex<Option<String>>>,
}

fn crear_servidor_falso<F>(manejador: F) -> ServidorFalso
where
    F: Fn(usize, &str) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("vincular puerto libre en loopback");
    let puerto = listener.local_addr().unwrap().port();
    let contador = Arc::new(AtomicUsize::new(0));
    let contador_clon = Arc::clone(&contador);
    let ultimo_cuerpo = Arc::new(std::sync::Mutex::new(None));
    let ultimo_cuerpo_clon = Arc::clone(&ultimo_cuerpo);
    let manejador = Arc::new(manejador);

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let num_peticion = contador_clon.fetch_add(1, Ordering::SeqCst);
            let manejador = Arc::clone(&manejador);
            let ultimo_cuerpo_clon = Arc::clone(&ultimo_cuerpo_clon);
            thread::spawn(move || {
                let mut reader = BufReader::new(&stream);
                let mut primera_linea = String::new();
                if reader.read_line(&mut primera_linea).is_err() {
                    return;
                }

                let mut longitud_cuerpo = 0;
                loop {
                    let mut linea = String::new();
                    if reader.read_line(&mut linea).is_err() || linea.trim().is_empty() {
                        break;
                    }
                    if linea.to_lowercase().starts_with("content-length:") {
                        if let Some(val) = linea.split(':').nth(1) {
                            longitud_cuerpo = val.trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }

                let mut cuerpo = vec![0u8; longitud_cuerpo];
                if longitud_cuerpo > 0 {
                    let _ = reader.read_exact(&mut cuerpo);
                }
                let cuerpo_str = String::from_utf8_lossy(&cuerpo).to_string();
                *ultimo_cuerpo_clon.lock().unwrap() = Some(cuerpo_str.clone());

                let (codigo, cuerpo_respuesta) = manejador(num_peticion, &cuerpo_str);
                let razon = match codigo {
                    200 => "OK",
                    401 => "Unauthorized",
                    500 => "Internal Server Error",
                    _ => "Error",
                };
                let respuesta_http = format!(
                    "HTTP/1.1 {codigo} {razon}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{cuerpo_respuesta}",
                    cuerpo_respuesta.len()
                );
                let _ = stream.write_all(respuesta_http.as_bytes());
            });
        }
    });

    ServidorFalso {
        puerto,
        contador,
        ultimo_cuerpo,
    }
}

#[tokio::test]
async fn el_notificador_telegram_postea_a_sendmessage_con_el_codigo_en_el_cuerpo() {
    let servidor = crear_servidor_falso(|_num, _body| (200, r#"{"ok":true}"#.to_string()));

    let notificador = NotificadorTelegram::nuevo(ConfiguracionDeTelegram {
        url_base: format!("http://127.0.0.1:{}", servidor.puerto),
        token: "token-secreto-123".to_string(),
        id_chat: "42".to_string(),
        timeout: Duration::from_secs(5),
    });

    let notificacion = Notificacion::nueva(CodigoDeNotificacion::nuevo("condicion-de-prueba"));
    notificador
        .notificar(notificacion)
        .await
        .expect("debe ser exitoso contra el servidor de loopback");

    assert_eq!(servidor.contador.load(Ordering::SeqCst), 1);
    let cuerpo = servidor
        .ultimo_cuerpo
        .lock()
        .unwrap()
        .clone()
        .expect("debe haberse recibido un cuerpo");
    assert!(cuerpo.contains("condicion-de-prueba"));
    assert!(cuerpo.contains("\"chat_id\":\"42\""));
}

#[tokio::test]
async fn el_notificador_telegram_mapea_un_estado_no_2xx_a_error() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            401,
            r#"{"ok":false,"description":"no autorizado"}"#.to_string(),
        )
    });

    let notificador = NotificadorTelegram::nuevo(ConfiguracionDeTelegram {
        url_base: format!("http://127.0.0.1:{}", servidor.puerto),
        token: "token-invalido".to_string(),
        id_chat: "42".to_string(),
        timeout: Duration::from_secs(5),
    });

    let notificacion = Notificacion::nueva(CodigoDeNotificacion::nuevo("condicion-de-prueba"));
    let error = notificador
        .notificar(notificacion)
        .await
        .expect_err("debe fallar con estado 401");
    assert!(format!("{error:?}").contains("401"));
    assert!(!format!("{error:?}").contains("token-invalido"));
}
