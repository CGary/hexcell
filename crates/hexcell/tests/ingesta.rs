//! Pruebas de integración del flujo de ingesta de conocimiento en sombra.
//!
//! Estos tests verifican la orquestación asíncrona de la ingesta, incluyendo
//! la fragmentación uniforme por lotes, el aislamiento físico de la base de producción,
//! el manejo de respuestas parciales con huecos en los ordinales, el registro correcto
//! de metadatos de época y la interrupción ordenada ante la señal de apagado.
//!
//! Diseñado el 28 de agosto de 2026 para garantizar la robustez del motor.

mod comun;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell::ingesta::{DesenlaceDeIngesta, ejecutar_ingesta};
use hexcell::proveedor_embeddings::{ConfiguracionDeEmbeddings, ProveedorDeEmbeddingsOpenRouter};
use hexcell::proveedor_embeddings_gemini::{
    ConfiguracionDeEmbeddingsGemini, ProveedorDeEmbeddingsGemini,
};
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::{DocumentoDeIngesta, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA};

use comun::{DirectorioTemporal, abrir_persistencia};

struct ServidorFalso {
    puerto: u16,
    contador: Arc<AtomicUsize>,
}

fn crear_servidor_falso<F>(manejador: F) -> ServidorFalso
where
    F: Fn(usize, &str) -> (u16, String) + Send + Sync + 'static,
{
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("vincular puerto en loopback");
    let puerto = listener.local_addr().unwrap().port();
    let contador = Arc::new(AtomicUsize::new(0));
    let contador_clon = Arc::clone(&contador);
    let manejador = Arc::new(manejador);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let num_peticion = contador_clon.fetch_add(1, Ordering::SeqCst);
            let manejador = Arc::clone(&manejador);
            std::thread::spawn(move || {
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
                let respuesta_http = format!(
                    "HTTP/1.1 {codigo} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{cuerpo_respuesta}",
                    cuerpo_respuesta.len()
                );
                let _ = stream.write_all(respuesta_http.as_bytes());
            });
        }
    });

    ServidorFalso { puerto, contador }
}

#[tokio::test]
async fn test_ac_1_limpieza_de_basura_previa() {
    let temp = DirectorioTemporal::nuevo("ac1-limpieza");
    let ruta_datos = temp.ruta();
    let (_, repositorio) = abrir_persistencia(ruta_datos);
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    fs::write(&ruta_base, b"basura base").unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/ref-ac1".to_string(),
        titulo: "AC1".to_string(),
        contenido: "Texto".to_string(),
        actualizado_ms: 1000,
    };

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    let resumen = ejecutar_ingesta(doc, config, &servicio, ruta_datos, || false)
        .await
        .unwrap();

    assert_eq!(resumen.desenlace, DesenlaceDeIngesta::Completa);
    // La base vieja debió ser sobreescrita.
    let count = hexcell_storage::conocimiento::contar_fragmentos(ruta_datos).unwrap();
    assert!(count > 0);
}

#[tokio::test]
async fn test_ac_2_aislamiento_de_knowledge_live() {
    let temp = DirectorioTemporal::nuevo("ac2-aislamiento");
    let ruta_datos = temp.ruta();
    let (pools, repositorio) = abrir_persistencia(ruta_datos);

    pools.conocimiento().con_lectura(|_conn| Ok(())).unwrap();
    let path_live = ruta_datos.join("knowledge_live.db");
    fs::write(&path_live, b"contenido vivo inalterado").unwrap();

    let meta_ant = fs::metadata(&path_live).unwrap();
    let mtime_ant = meta_ant.modified().unwrap();
    let size_ant = meta_ant.len();

    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/ref-ac2".to_string(),
        titulo: "AC2".to_string(),
        contenido: "Este contenido de prueba debe aislarse".to_string(),
        actualizado_ms: 1000,
    };

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    ejecutar_ingesta(doc, config, &servicio, ruta_datos, || false)
        .await
        .unwrap();

    let meta_post = fs::metadata(&path_live).unwrap();
    assert_eq!(meta_post.len(), size_ant);
    assert_eq!(meta_post.modified().unwrap(), mtime_ant);
}

#[tokio::test]
async fn test_ac_3_tamano_de_lote_openrouter_y_gemini() {
    // 1. Probamos con ProveedorDeEmbeddingsOpenRouter
    // El manejador responde con tantos elementos como textos traiga la petición, porque el
    // proveedor rechaza una respuesta cuyo tamaño no case exactamente con el lote solicitado.
    let servidor_or = crear_servidor_falso(|_num, cuerpo_peticion| {
        let peticion: serde_json::Value =
            serde_json::from_str(cuerpo_peticion).unwrap_or(serde_json::Value::Null);
        let cantidad = peticion
            .get("input")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let elementos: Vec<String> = (0..cantidad)
            .map(|i| format!(r#"{{"object":"embedding","index":{i},"embedding":[0.1,0.2]}}"#))
            .collect();
        let cuerpo_respuesta = format!(
            r#"{{"object":"list","data":[{}],"usage":{{"prompt_tokens":10}}}}"#,
            elementos.join(",")
        );
        (200, cuerpo_respuesta)
    });

    let config_or = ConfiguracionDeEmbeddings {
        url_base: format!("http://127.0.0.1:{}", servidor_or.puerto),
        api_key: "key-or".to_string(),
        modelo: "model-or".to_string(),
        timeout: Duration::from_secs(5),
        reintentos: 1,
        tamano_de_lote: 2, // Lote de tamaño 2
    };
    let proveedor_or = ProveedorDeEmbeddingsOpenRouter::nuevo(config_or);

    let temp = DirectorioTemporal::nuevo("ac3-batching-or");
    let ruta_datos = temp.ruta();
    let (_, repositorio) = abrir_persistencia(ruta_datos);
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/ref-ac3-or".to_string(),
        titulo: "AC3 OR".to_string(),
        contenido: "ABCDE".to_string(), // Cinco caracteres, un carácter por fragmento -> 5 fragmentos
        actualizado_ms: 1000,
    };

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 1,
        solapamiento: 0,
    };

    let servicio_or = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::OpenRouter(Box::new(proveedor_or)),
        repositorio.clone(),
    );

    let resumen_or = ejecutar_ingesta(
        doc.clone(),
        config.clone(),
        &servicio_or,
        ruta_datos,
        || false,
    )
    .await
    .unwrap();

    assert_eq!(
        resumen_or.lotes_emitidos, 3,
        "Deberían emitirse exactamente 3 lotes (2+2+1)"
    );
    assert_eq!(servidor_or.contador.load(Ordering::SeqCst), 3);

    // 2. Probamos con ProveedorDeEmbeddingsGemini
    // El manejador responde con tantos elementos como peticiones traiga el arreglo `requests`,
    // porque este proveedor rechaza una respuesta cuya longitud no case con la petición.
    let servidor_gem = crear_servidor_falso(|_num, cuerpo_peticion| {
        let peticion: serde_json::Value =
            serde_json::from_str(cuerpo_peticion).unwrap_or(serde_json::Value::Null);
        let cantidad = peticion
            .get("requests")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let elementos: Vec<&str> = std::iter::repeat(r#"{"values":[0.1,0.2]}"#)
            .take(cantidad)
            .collect();
        let cuerpo_respuesta = format!(
            r#"{{"embeddings":[{}],"usageMetadata":{{"promptTokenCount":10}}}}"#,
            elementos.join(",")
        );
        (200, cuerpo_respuesta)
    });

    let config_gem = ConfiguracionDeEmbeddingsGemini {
        url_base: format!("http://127.0.0.1:{}", servidor_gem.puerto),
        api_key: "key-gem".to_string(),
        modelo: "model-gem".to_string(),
        timeout: Duration::from_secs(5),
        reintentos: 1,
        tamano_de_lote: 2, // Lote de tamaño 2
    };
    let proveedor_gem = ProveedorDeEmbeddingsGemini::nuevo(config_gem);

    let temp_gem = DirectorioTemporal::nuevo("ac3-batching-gem");
    let ruta_datos_gem = temp_gem.ruta();
    let (_, repositorio_gem) = abrir_persistencia(ruta_datos_gem);
    repositorio_gem
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let servicio_gem = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Gemini(Box::new(proveedor_gem)),
        repositorio_gem,
    );

    let resumen_gem = ejecutar_ingesta(doc, config, &servicio_gem, ruta_datos_gem, || false)
        .await
        .unwrap();

    assert_eq!(
        resumen_gem.lotes_emitidos, 3,
        "Deberían emitirse exactamente 3 lotes (2+2+1)"
    );
    assert_eq!(servidor_gem.contador.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_ac_4_parcial_y_huecos_ordinales_sin_huerfanos() {
    let temp = DirectorioTemporal::nuevo("ac4-parcial");
    let ruta_datos = temp.ruta();
    let (_, repositorio) = abrir_persistencia(ruta_datos);
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/ref-ac4".to_string(),
        titulo: "AC4".to_string(),
        contenido: "ABCDE".to_string(), // Cinco caracteres, un carácter por fragmento -> 5 fragmentos
        actualizado_ms: 1000,
    };

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 1,
        solapamiento: 0,
    };

    // Proveedor simulado configurado para devolver solo hasta 1 elemento resuelto por lote de tamaño 2.
    // Lote 1: solicitado [0, 1] -> devuelto [Some, None]
    // Lote 2: solicitado [2, 3] -> devuelto [Some, None]
    // Lote 3: solicitado [4] -> devuelto [Some]
    // Fragmentos escritos deberían ser: ordinal 0, 2, 4. Gaps en 1 y 3.
    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4)
        .con_tamano_de_lote(2)
        .con_limite_elementos(1);

    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    let resumen = ejecutar_ingesta(doc, config, &servicio, ruta_datos, || false)
        .await
        .unwrap();

    assert_eq!(resumen.desenlace, DesenlaceDeIngesta::Parcial);
    assert_eq!(resumen.fragmentos_solicitados, 5);
    assert_eq!(resumen.fragmentos_escritos, 3);

    // Verificamos ordinales escritos.
    let ordinales = hexcell_storage::conocimiento::listar_ordinales(ruta_datos).unwrap();
    assert_eq!(
        ordinales,
        vec![0, 2, 4],
        "Se debieron respetar los ordinales originales sin compactar"
    );

    // Verificamos que no existan huérfanos.
    // Cada fragmento escrito debe tener su vector, y no debe haber vectores sin fragmento.
    let huerfanos_fragmento =
        hexcell_storage::conocimiento::contar_fragmentos_sin_vector(ruta_datos).unwrap();
    assert_eq!(
        huerfanos_fragmento, 0,
        "No debe haber ningún fragmento sin vector"
    );
}

#[tokio::test]
async fn test_ac_5_dimension_y_descarte_de_metadatos() {
    // Escenario A: Con embeddings resueltos con dimensión 8.
    let temp_a = DirectorioTemporal::nuevo("ac5-metadata-a");
    let ruta_datos_a = temp_a.ruta();
    let (_, repositorio_a) = abrir_persistencia(ruta_datos_a);
    repositorio_a
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/ref-ac5-a".to_string(),
        titulo: "AC5 A".to_string(),
        contenido: "Un fragmento".to_string(),
        actualizado_ms: 1000,
    };
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 20,
        solapamiento: 0,
    };

    let proveedor_a = ProveedorDeEmbeddingsSimulado::con_dimension(8).con_tamano_de_lote(2);
    let servicio_a = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor_a),
        repositorio_a,
    );

    let resumen_a = ejecutar_ingesta(
        doc.clone(),
        config.clone(),
        &servicio_a,
        ruta_datos_a,
        || false,
    )
    .await
    .unwrap();

    assert_eq!(resumen_a.dimension_observada, Some(8));

    let metadatos_a = hexcell_storage::conocimiento::leer_metadatos_de_epoca(ruta_datos_a)
        .unwrap()
        .expect("la fila de metadatos debe sobrevivir cuando hubo al menos un embedding resuelto");

    assert!(metadatos_a.numero_de_epoca.is_none());
    assert!(metadatos_a.sellada_ms.is_none());
    assert_eq!(metadatos_a.dimension_de_embedding, 8);

    // Escenario B: Con 0 embeddings resueltos.
    let temp_b = DirectorioTemporal::nuevo("ac5-metadata-b");
    let ruta_datos_b = temp_b.ruta();
    let (_, repositorio_b) = abrir_persistencia(ruta_datos_b);
    repositorio_b
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let proveedor_b = ProveedorDeEmbeddingsSimulado::con_dimension(8)
        .con_tamano_de_lote(2)
        .con_limite_elementos(0); // 0 resueltos

    let servicio_b = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor_b),
        repositorio_b,
    );

    let resumen_b = ejecutar_ingesta(doc, config, &servicio_b, ruta_datos_b, || false)
        .await
        .unwrap();

    assert_eq!(resumen_b.desenlace, DesenlaceDeIngesta::SinIncrustaciones);
    assert_eq!(resumen_b.fragmentos_escritos, 0);

    // El registro de metadatos de época debe haber sido eliminado para no mentar una dimensión no observada.
    let metadatos_b = hexcell_storage::conocimiento::leer_metadatos_de_epoca(ruta_datos_b).unwrap();
    assert!(metadatos_b.is_none());

    // El documento fuente debe sobrevivir para diagnóstico.
    let doc_existe = hexcell_storage::conocimiento::documento_sobrevive(ruta_datos_b).unwrap();
    assert!(doc_existe);
}

#[tokio::test]
async fn test_ac_6_apagado_en_frontera_de_lote_y_capas_de_presupuesto() {
    let temp = DirectorioTemporal::nuevo("ac6-apagado");
    let ruta_datos = temp.ruta();
    let (pools, repositorio) = abrir_persistencia(ruta_datos);
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/ref-ac6".to_string(),
        titulo: "AC6".to_string(),
        contenido: "A B C D E".to_string(), // Chunks de 1 -> 5 fragmentos
        actualizado_ms: 1000,
    };
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 1,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    // Predicado de apagado que se activa tras procesar el primer lote (es decir, en el lote index 1).
    let bandera_apagado = Arc::new(AtomicUsize::new(0));
    let bandera_apagado_clon = Arc::clone(&bandera_apagado);
    let deudor_apagado = move || {
        let val = bandera_apagado_clon.fetch_add(1, Ordering::SeqCst);
        val >= 1 // Lote 0 pasa (val=0), Lote 1 se detiene (val=1)
    };

    let resumen = ejecutar_ingesta(doc, config, &servicio, ruta_datos, deudor_apagado)
        .await
        .unwrap();

    assert_eq!(resumen.desenlace, DesenlaceDeIngesta::DetenidaPorApagado);
    assert_eq!(
        resumen.lotes_emitidos, 1,
        "Debería detenerse tras emitir exactamente 1 lote"
    );

    // Verificamos que saldo.reservado esté en cero.
    let saldo = pools
        .sesiones()
        .con_lectura(|conn| {
            let disp: i64 = conn
                .query_row("SELECT disponible FROM saldo WHERE id = 1", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let res: i64 = conn
                .query_row("SELECT reservado FROM saldo WHERE id = 1", [], |r| r.get(0))
                .unwrap();
            Ok((disp, res))
        })
        .unwrap();

    assert_eq!(
        saldo.1, 0,
        "No debe quedar presupuesto atrapado en reservado"
    );

    // El número de reservas en estado 'activa' debe ser 0.
    let reservas_activas: i64 = pools
        .sesiones()
        .con_lectura(|conn| {
            let cantidad: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM reservas WHERE estado = 'activa'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(cantidad)
        })
        .unwrap();
    assert_eq!(reservas_activas, 0);

    // El número total de filas de reservas (independientemente de su estado) debe ser igual al
    // número de lotes emitidos, demostrando que la ingesta no realiza reservas redundantes.
    let total_reservas: i64 = pools
        .sesiones()
        .con_lectura(|conn| {
            let cantidad: i64 = conn
                .query_row("SELECT COUNT(*) FROM reservas", [], |r| r.get(0))
                .unwrap();
            Ok(cantidad)
        })
        .unwrap();
    assert_eq!(total_reservas, 1);
}
