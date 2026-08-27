//! Tests del proveedor de inferencia HTTPS compatible con OpenAI.
//!
//! Se ejecutan de forma aislada y sin conexión externa a red, utilizando un servidor HTTP falso
//! levantado sobre `std::net::TcpListener` en el adaptador de loopback (`127.0.0.1`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hexcell::proveedor_openai::ConfiguracionDeInferencia;
use hexcell::proveedor_openai::ProveedorOpenAi;
use hexcell_core::identidad::IdConversacion;
use hexcell_core::inferencia::{PeticionDeInferencia, ProveedorDeInferencia};

struct ServidorFalso {
    puerto: u16,
    contador: Arc<AtomicUsize>,
}

fn crear_servidor_falso<F>(manejador: F) -> ServidorFalso
where
    F: Fn(usize, &str) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("vincular puerto libre en loopback");
    let puerto = listener.local_addr().unwrap().port();
    let contador = Arc::new(AtomicUsize::new(0));
    let contador_clon = Arc::clone(&contador);
    let manejador = Arc::new(manejador);

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let num_peticion = contador_clon.fetch_add(1, Ordering::SeqCst);
            let manejador = Arc::clone(&manejador);
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
                let cuerpo_str = String::from_utf8_lossy(&cuerpo);

                let (codigo, cuerpo_respuesta) = manejador(num_peticion, &cuerpo_str);
                if codigo == 0 {
                    thread::sleep(Duration::from_secs(30));
                    return;
                }

                let razon = match codigo {
                    200 => "OK",
                    429 => "Too Many Requests",
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

    ServidorFalso { puerto, contador }
}

fn crear_proveedor_test(puerto: u16, timeout_ms: u64, reintentos: u32) -> ProveedorOpenAi {
    ProveedorOpenAi::nuevo(ConfiguracionDeInferencia {
        url_base: format!("http://127.0.0.1:{puerto}"),
        api_key: "clave-secret-test".to_string(),
        modelo: "deepseek-chat".to_string(),
        timeout: Duration::from_millis(timeout_ms),
        reintentos,
    })
}

#[tokio::test]
async fn respuesta_exitosa_mapea_contenido_y_unidades() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"choices":[{"message":{"content":"respuesta del modelo"}}],"usage":{"prompt_tokens":12,"completion_tokens":8}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conv-1"),
        contenido: "hola mundo".to_string(),
    };

    let respuesta = proveedor.generar(peticion).await.expect("debe ser exitoso");
    assert_eq!(respuesta.contenido, "respuesta del modelo");
    assert_eq!(respuesta.unidades_consumidas, 20);
}

#[tokio::test]
async fn respuesta_sin_metadatos_de_uso_falla_fail_closed() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"choices":[{"message":{"content":"respuesta sin uso"}}]}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conv-2"),
        contenido: "test".to_string(),
    };

    let error = proveedor
        .generar(peticion)
        .await
        .expect_err("debe fallar sin metadatos de uso");
    assert!(format!("{error:?}").contains("RespuestaInvalida"));
}

#[tokio::test]
async fn respuesta_con_arreglo_choices_vacio_falla() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":5}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conv-3"),
        contenido: "test".to_string(),
    };

    let error = proveedor
        .generar(peticion)
        .await
        .expect_err("debe fallar con choices vacío");
    assert!(format!("{error:?}").contains("RespuestaInvalida"));
}

#[tokio::test]
async fn servidor_que_no_responde_falla_por_timeout_acotado() {
    let servidor = crear_servidor_falso(|_num, _body| (0, "".to_string()));

    let proveedor = crear_proveedor_test(servidor.puerto, 150, 1);
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conv-4"),
        contenido: "stall test".to_string(),
    };

    let inicio = Instant::now();
    let error = proveedor
        .generar(peticion)
        .await
        .expect_err("debe fallar por timeout");
    let duracion = inicio.elapsed();

    assert!(format!("{error:?}").contains("TiempoAgotado"));
    assert!(duracion >= Duration::from_millis(300));
    assert!(duracion < Duration::from_millis(2000));
}

#[tokio::test]
async fn error_429_no_se_reintenta_nunca() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (429, r#"{"error":{"message":"Quota exceeded"}}"#.to_string())
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 3);
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conv-5"),
        contenido: "test 429".to_string(),
    };

    let error = proveedor
        .generar(peticion)
        .await
        .expect_err("debe fallar con 429");
    assert!(format!("{error:?}").contains("429"));
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn error_500_se_reintenta_hasta_el_tope_fijo() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            500,
            r#"{"error":{"message":"Internal Server Error"}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 2);
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conv-6"),
        contenido: "test 500".to_string(),
    };

    let error = proveedor
        .generar(peticion)
        .await
        .expect_err("debe fallar tras agotar reintentos");
    assert!(format!("{error:?}").contains("500"));
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 3);
}
