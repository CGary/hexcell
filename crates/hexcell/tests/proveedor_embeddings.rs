//! Tests del proveedor de embeddings HTTPS compatible con OpenAI / OpenRouter.
//!
//! Se ejecutan de forma completamente aislada y sin conexión externa, utilizando un servidor HTTP
//! falso sobre `std::net::TcpListener` en loopback (`127.0.0.1`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hexcell::embeddings::{ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado};
use hexcell::proveedor_embeddings::{
    ConfiguracionDeEmbeddings, ErrorDeProveedorDeEmbeddings, ProveedorDeEmbeddingsOpenRouter,
};
use hexcell_core::embeddings::{PeticionDeEmbeddings, ProveedorDeEmbeddings};

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

fn crear_proveedor_test(
    puerto: u16,
    timeout_ms: u64,
    reintentos: u32,
) -> ProveedorDeEmbeddingsOpenRouter {
    ProveedorDeEmbeddingsOpenRouter::nuevo(ConfiguracionDeEmbeddings {
        url_base: format!("http://127.0.0.1:{puerto}"),
        api_key: "clave-secret-embeddings-test".to_string(),
        modelo: "text-embedding-3-small".to_string(),
        timeout: Duration::from_millis(timeout_ms),
        reintentos,
        tamano_de_lote: 32,
    })
}

#[tokio::test]
async fn respuesta_exitosa_coloca_vectores_por_indice_explicito_y_calcula_uso() {
    // El servidor devuelve los elementos de data desordenados (índice 1 primero, luego índice 0)
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":1,"embedding":[2.0,2.5]},{"object":"embedding","index":0,"embedding":[1.0,1.5]}],"usage":{"prompt_tokens":15,"completion_tokens":0}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto 0".to_string(), "texto 1".to_string()],
    };

    let respuesta = proveedor
        .incrustar_lote(peticion)
        .await
        .expect("la llamada debe tener éxito");
    assert_eq!(respuesta.vectores.len(), 2);

    let v0 = respuesta.vectores[0]
        .as_ref()
        .expect("posición 0 debe existir");
    assert_eq!(v0.valores(), &[1.0, 1.5]);

    let v1 = respuesta.vectores[1]
        .as_ref()
        .expect("posición 1 debe existir");
    assert_eq!(v1.valores(), &[2.0, 2.5]);

    assert_eq!(respuesta.unidades_consumidas, 15);
}

#[tokio::test]
async fn respuesta_con_uso_sin_completion_tokens_se_factura_como_prompt_tokens() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.5,0.75]}],"usage":{"prompt_tokens":8}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["solo prompt".to_string()],
    };

    let respuesta = proveedor
        .incrustar_lote(peticion)
        .await
        .expect("debe ser exitoso");
    assert_eq!(respuesta.unidades_consumidas, 8);
}

#[tokio::test]
async fn respuesta_sin_uso_reporta_cero_unidades() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.5,0.75]}]}"#
                .to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["sin uso".to_string()],
    };

    let respuesta = proveedor
        .incrustar_lote(peticion)
        .await
        .expect("debe ser exitoso");
    assert_eq!(respuesta.unidades_consumidas, 0);
}

#[tokio::test]
async fn respuesta_con_elementos_parciales_deja_slots_en_none() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":1,"embedding":[3.0]}],"usage":{"prompt_tokens":5}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec![
            "texto 0".to_string(),
            "texto 1".to_string(),
            "texto 2".to_string(),
        ],
    };

    let respuesta = proveedor
        .incrustar_lote(peticion)
        .await
        .expect("debe ser exitoso");
    assert_eq!(respuesta.vectores.len(), 3);
    assert!(respuesta.vectores[0].is_none());
    assert_eq!(respuesta.vectores[1].as_ref().unwrap().valores(), &[3.0]);
    assert!(respuesta.vectores[2].is_none());
}

#[tokio::test]
async fn respuesta_con_indice_duplicado_falla() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[1.0]},{"object":"embedding","index":0,"embedding":[2.0]}]}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["a".to_string(), "b".to_string()],
    };

    let err = proveedor
        .incrustar_lote(peticion)
        .await
        .expect_err("debe fallar con índice duplicado");
    assert!(matches!(
        err,
        ErrorDeProveedorDeEmbeddings::RespuestaInvalida(_)
    ));
}

#[tokio::test]
async fn respuesta_con_indice_fuera_de_rango_falla() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":10,"embedding":[1.0]}]}"#
                .to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["a".to_string()],
    };

    let err = proveedor
        .incrustar_lote(peticion)
        .await
        .expect_err("debe fallar con índice fuera de rango");
    assert!(matches!(
        err,
        ErrorDeProveedorDeEmbeddings::RespuestaInvalida(_)
    ));
}

#[tokio::test]
async fn respuesta_con_vector_de_longitud_cero_falla() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[]}]}"#
                .to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["a".to_string()],
    };

    let err = proveedor
        .incrustar_lote(peticion)
        .await
        .expect_err("debe fallar con vector vacío");
    assert!(matches!(
        err,
        ErrorDeProveedorDeEmbeddings::RespuestaInvalida(_)
    ));
}

#[tokio::test]
async fn error_429_no_se_reintenta_nunca() {
    let servidor = crear_servidor_falso(|_num, _body| {
        (
            429,
            r#"{"error":{"message":"Rate limit exceeded"}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 3);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["test 429".to_string()],
    };

    let err = proveedor
        .incrustar_lote(peticion)
        .await
        .expect_err("debe fallar inmediatamente");
    assert!(matches!(
        err,
        ErrorDeProveedorDeEmbeddings::CodigoDeEstadoHttp { codigo: 429, .. }
    ));
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
    let peticion = PeticionDeEmbeddings {
        textos: vec!["test 500".to_string()],
    };

    let inicio = Instant::now();
    let err = proveedor
        .incrustar_lote(peticion)
        .await
        .expect_err("debe fallar tras agotar reintentos");
    let duracion = inicio.elapsed();

    assert!(matches!(
        err,
        ErrorDeProveedorDeEmbeddings::CodigoDeEstadoHttp { codigo: 500, .. }
    ));
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 3);
    // 2 reintentos con 250 ms fijo cada uno -> al menos 500 ms transcurridos
    assert!(duracion >= Duration::from_millis(500));
}

#[tokio::test]
async fn cuerpo_malformado_en_200_falla_sin_reintento() {
    let servidor = crear_servidor_falso(|_num, _body| (200, "no es un json valido".to_string()));

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 2);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["test malformado".to_string()],
    };

    let err = proveedor
        .incrustar_lote(peticion)
        .await
        .expect_err("debe fallar por JSON malformado");
    assert!(matches!(
        err,
        ErrorDeProveedorDeEmbeddings::RespuestaInvalida(_)
    ));
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proveedor_de_embeddings_de_celula_despacho_por_enum() {
    let simulado = ProveedorDeEmbeddingsSimulado::nuevo();
    let selector_simulado = ProveedorDeEmbeddingsDeCelula::Simulado(simulado);

    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto simulado".to_string()],
    };

    let res_simulada = selector_simulado
        .incrustar_lote(peticion.clone())
        .await
        .expect("debe despachar a simulado");
    assert_eq!(res_simulada.vectores.len(), 1);

    let servidor = crear_servidor_falso(|_num, _body| {
        (
            200,
            r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[7.0,8.0]}],"usage":{"prompt_tokens":3}}"#.to_string(),
        )
    });

    let openrouter = Box::new(crear_proveedor_test(servidor.puerto, 5000, 1));
    let selector_openrouter = ProveedorDeEmbeddingsDeCelula::OpenRouter(openrouter);

    let res_openrouter = selector_openrouter
        .incrustar_lote(peticion)
        .await
        .expect("debe despachar a openrouter");
    assert_eq!(res_openrouter.vectores.len(), 1);
    assert_eq!(
        res_openrouter.vectores[0].as_ref().unwrap().valores(),
        &[7.0, 8.0]
    );
}
