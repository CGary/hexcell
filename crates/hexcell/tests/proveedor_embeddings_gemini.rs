//! Tests del proveedor de embeddings HTTPS de Google AI Studio (Gemini).
//!
//! Se ejecutan de forma completamente aislada y sin conexión externa, utilizando un servidor HTTP
//! falso sobre `std::net::TcpListener` en loopback (`127.0.0.1`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hexcell::embeddings::ProveedorDeEmbeddingsDeCelula;
use hexcell::proveedor_embeddings_gemini::{
    ConfiguracionDeEmbeddingsGemini, ErrorDeProveedorDeEmbeddingsGemini,
    ProveedorDeEmbeddingsGemini,
};
use hexcell_core::embeddings::{PeticionDeEmbeddings, ProveedorDeEmbeddings};

struct ServidorFalso {
    puerto: u16,
    contador: Arc<AtomicUsize>,
}

fn crear_servidor_falso<F>(manejador: F) -> ServidorFalso
where
    F: Fn(usize, &str, &str, &[String]) -> (u16, String) + Send + Sync + 'static,
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
                let mut cabeceras = Vec::new();
                loop {
                    let mut linea = String::new();
                    if reader.read_line(&mut linea).is_err() || linea.trim().is_empty() {
                        break;
                    }
                    cabeceras.push(linea.trim().to_string());
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

                let (codigo, cuerpo_respuesta) =
                    manejador(num_peticion, primera_linea.trim(), &cuerpo_str, &cabeceras);
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
) -> ProveedorDeEmbeddingsGemini {
    ProveedorDeEmbeddingsGemini::nuevo(ConfiguracionDeEmbeddingsGemini {
        url_base: format!("http://127.0.0.1:{puerto}"),
        api_key: "clave-secret-embeddings-test".to_string(),
        modelo: "text-embedding-004".to_string(),
        timeout: Duration::from_millis(timeout_ms),
        reintentos,
        tamano_de_lote: 32,
    })
}

#[tokio::test]
async fn respuesta_exitosa_coloca_vectores_por_orden_y_calcula_uso() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (
            200,
            r#"{"embeddings":[{"values":[0.0,0.5]},{"values":[1.0,1.5]},{"values":[2.0,2.5]}],"usageMetadata":{"promptTokenCount":18}}"#.to_string(),
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
        .expect("la llamada debe tener éxito");
    assert_eq!(respuesta.vectores.len(), 3);

    for i in 0..3 {
        let v = respuesta.vectores[i]
            .as_ref()
            .expect("debe existir el vector");
        assert_eq!(v.valores()[0], i as f32);
    }
    assert_eq!(respuesta.unidades_consumidas, 18);
}

#[tokio::test]
async fn error_si_longitud_no_coincide() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (
            200,
            r#"{"embeddings":[{"values":[0.0,0.5]},{"values":[1.0,1.5]}]}"#.to_string(),
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

    let resultado = proveedor.incrustar_lote(peticion).await;
    match resultado {
        Err(ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(err)) => {
            assert!(err.contains("longitud"));
        }
        other => panic!("Se esperaba RespuestaInvalida, pero se obtuvo: {:?}", other),
    }
}

#[tokio::test]
async fn reintentos_limite_y_comportamiento_con_429() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (
            429,
            r#"{"error":{"message":"Too many requests"}}"#.to_string(),
        )
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 2);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto".to_string()],
    };

    let inicio = Instant::now();
    let resultado = proveedor.incrustar_lote(peticion).await;
    let duracion = inicio.elapsed();

    assert!(resultado.is_err());
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 1);
    assert!(duracion < Duration::from_millis(100));
}

#[tokio::test]
async fn reintentos_limite_y_comportamiento_con_500() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (500, r#"{"error":{"message":"Internal error"}}"#.to_string())
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 2);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto".to_string()],
    };

    let inicio = Instant::now();
    let resultado = proveedor.incrustar_lote(peticion).await;
    let duracion = inicio.elapsed();

    assert!(resultado.is_err());
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 3);
    assert!(duracion >= Duration::from_millis(500));
}

#[tokio::test]
async fn reintentos_limite_y_comportamiento_con_cuerpo_malformado() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (200, r#"{"embeddings": [{"values": "#.to_string())
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 2);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto".to_string()],
    };

    let resultado = proveedor.incrustar_lote(peticion).await;
    assert!(matches!(
        resultado,
        Err(ErrorDeProveedorDeEmbeddingsGemini::RespuestaInvalida(_))
    ));
    assert_eq!(servidor.contador.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn respuesta_sin_uso_de_token_devuelve_cero() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (200, r#"{"embeddings":[{"values":[0.1,0.2]}]}"#.to_string())
    });

    let proveedor = crear_proveedor_test(servidor.puerto, 5000, 1);
    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto".to_string()],
    };

    let respuesta = proveedor.incrustar_lote(peticion).await.unwrap();
    assert_eq!(respuesta.unidades_consumidas, 0);
}

#[tokio::test]
async fn clave_api_solo_en_cabecera_no_en_url() {
    let clave_sentinela = "CLAVE_SECRET_UNICA_EMBED_GEMINI_TEST_SENTINEL";
    let servidor = crear_servidor_falso(move |_num, req_line, _body, headers| {
        assert!(
            !req_line.contains(clave_sentinela),
            "La clave secreta se filtró en la línea de petición HTTP!"
        );

        let mut encontrada = false;
        for h in headers {
            let h_lower = h.to_lowercase();
            if h_lower.starts_with("x-goog-api-key:") && h.contains(clave_sentinela) {
                encontrada = true;
            }
        }
        assert!(
            encontrada,
            "No se encontró la cabecera x-goog-api-key con la clave secreta"
        );

        (200, r#"{"embeddings":[{"values":[0.1,0.2]}]}"#.to_string())
    });

    let proveedor = ProveedorDeEmbeddingsGemini::nuevo(ConfiguracionDeEmbeddingsGemini {
        url_base: format!("http://127.0.0.1:{}", servidor.puerto),
        api_key: clave_sentinela.to_string(),
        modelo: "text-embedding-004".to_string(),
        timeout: Duration::from_secs(5),
        reintentos: 1,
        tamano_de_lote: 32,
    });

    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto".to_string()],
    };

    let _ = proveedor.incrustar_lote(peticion).await.unwrap();
}

#[tokio::test]
async fn proveedor_de_embeddings_de_celula_despacho_por_enum_gemini() {
    let servidor = crear_servidor_falso(|_num, _req_line, _body, _headers| {
        (200, r#"{"embeddings":[{"values":[9.0,10.0]}]}"#.to_string())
    });

    let config = ConfiguracionDeEmbeddingsGemini {
        url_base: format!("http://127.0.0.1:{}", servidor.puerto),
        api_key: "clave-secret-embeddings-test".to_string(),
        modelo: "text-embedding-004".to_string(),
        timeout: Duration::from_secs(5),
        reintentos: 1,
        tamano_de_lote: 32,
    };
    let gemini = Box::new(ProveedorDeEmbeddingsGemini::nuevo(config));
    let selector_gemini = ProveedorDeEmbeddingsDeCelula::Gemini(gemini);

    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto gemini".to_string()],
    };

    let res_gemini = selector_gemini
        .incrustar_lote(peticion)
        .await
        .expect("debe despachar a gemini");
    assert_eq!(res_gemini.vectores.len(), 1);
    assert_eq!(
        res_gemini.vectores[0].as_ref().unwrap().valores(),
        &[9.0, 10.0]
    );
}
