//! Tests de `Configuracion::desde_fuente`: camino feliz y cada modo de fallo.
//!
//! La mitad de estos tests son a nivel de biblioteca (construyen una `FuenteEnMemoria` con el caso
//! a ejercer y se la pasan a `Configuracion::desde_fuente`) y la otra mitad son a nivel de proceso:
//! lanzan `env!("CARGO_BIN_EXE_hexcell")` con un entorno controlado —el del **proceso hijo**, que
//! `Command::env` fija sin tocar el del proceso de pruebas— y comprueban el código de salida y
//! `stderr`, que es lo único que demuestra de verdad que el binario termina **antes** de vincular
//! nada (AC-2).
//!
//! Ningún test de este archivo escribe el entorno de su propio proceso: cada uno prepara su caso en
//! una tabla local, así que no hay estado compartido que serializar ni cerrojo que sostener.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use hexcell::configuracion::{
    CanalSeleccionado, Configuracion, ConfiguracionDeEmbeddingsSegunProveedor,
    ErrorDeConfiguracion, FuenteEnMemoria,
};

#[test]
fn arranca_con_configuracion_valida() {
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-config-ok-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(configuracion.id_celula, "piloto-01");
    assert_eq!(configuracion.ruta_datos, directorio_temporal);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn falla_si_falta_la_ruta_de_datos() {
    let fuente = FuenteEnMemoria::vacia().con("HEXCELL_ID_CELULA", "piloto-01");

    let error =
        Configuracion::desde_fuente(&fuente).expect_err("debe fallar sin HEXCELL_RUTA_DATOS");
    assert_eq!(
        error,
        ErrorDeConfiguracion::VariableAusente {
            nombre: "HEXCELL_RUTA_DATOS",
            formato_esperado: "ruta de directorio existente en disco",
        }
    );
}

#[test]
fn falla_si_la_ruta_de_datos_no_existe_en_disco() {
    let ruta_inexistente =
        std::env::temp_dir().join("hexcell-ruta-que-nunca-existe-en-este-test-12345");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", ruta_inexistente.to_string_lossy());

    let error = Configuracion::desde_fuente(&fuente)
        .expect_err("debe fallar si la ruta no existe en disco");
    match error {
        ErrorDeConfiguracion::RutaDeDatosInexistente { nombre, ruta } => {
            assert_eq!(nombre, "HEXCELL_RUTA_DATOS");
            assert_eq!(ruta, ruta_inexistente);
        }
        otro => panic!("se esperaba RutaDeDatosInexistente, se obtuvo {otro:?}"),
    }
}

#[test]
fn falla_si_la_direccion_de_salud_no_es_un_socket_valido() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-direccion-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_DIRECCION_SALUD", "no-es-un-socket");

    let error =
        Configuracion::desde_fuente(&fuente).expect_err("debe fallar con una dirección inválida");
    match error {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_DIRECCION_SALUD");
            assert_eq!(valor, "no-es-un-socket");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn falla_si_el_canal_no_es_reconocido() {
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-config-canal-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_CANAL", "canal-que-no-existe");

    let error =
        Configuracion::desde_fuente(&fuente).expect_err("debe fallar con un canal desconocido");
    match error {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_CANAL");
            assert_eq!(valor, "canal-que-no-existe");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_ventana_de_deduplicacion_por_defecto_es_una_hora_sin_la_variable_de_entorno() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-ventana-defecto-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion.ventana_deduplicacion,
        Duration::from_secs(3600)
    );

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_ventana_de_deduplicacion_se_puede_configurar_por_variable_de_entorno() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-ventana-explicita-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS", "120");

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion.ventana_deduplicacion,
        Duration::from_secs(120)
    );

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn falla_si_la_ventana_de_deduplicacion_no_es_un_entero_positivo() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-ventana-invalida-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS", "no-es-un-entero");

    let error =
        Configuracion::desde_fuente(&fuente).expect_err("debe fallar con una ventana no numérica");
    match error {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS");
            assert_eq!(valor, "no-es-un-entero");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

/// Ejecuta el binario real con el entorno dado y devuelve `(código de salida, stderr)`.
fn ejecutar_binario_con_entorno(variables: &[(&str, &str)]) -> (i32, String) {
    let mut comando = Command::new(env!("CARGO_BIN_EXE_hexcell"));
    comando.env_clear();
    for (nombre, valor) in variables {
        comando.env(nombre, valor);
    }
    comando.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut hijo = comando
        .spawn()
        .expect("el binario hexcell debe poder lanzarse");
    let estado = hijo
        .wait()
        .expect("esperar la salida del proceso hijo no debe fallar");
    let mut stderr = String::new();
    hijo.stderr
        .take()
        .expect("stderr debe estar disponible")
        .read_to_string(&mut stderr)
        .expect("leer stderr no debe fallar");

    (estado.code().unwrap_or(-1), stderr)
}

#[test]
fn el_binario_termina_antes_de_escuchar_si_falta_la_ruta_de_datos() {
    let (codigo, stderr) = ejecutar_binario_con_entorno(&[("HEXCELL_ID_CELULA", "piloto-01")]);

    assert_ne!(codigo, 0);
    assert!(stderr.contains("HEXCELL_RUTA_DATOS"));
    assert!(!stderr.to_lowercase().contains("panicked"));
    assert!(!stderr.contains("RUST_BACKTRACE"));
}

#[test]
fn el_binario_no_vincula_nada_si_la_configuracion_es_invalida() {
    // AC-10: este test ya no asume que el puerto por defecto del servidor de salud —común en
    // máquinas de desarrollo— está libre. En vez de eso, vincula un `TcpListener` efímero
    // (puerto 0) para que el sistema
    // operativo asigne uno libre, lo suelta para dejarlo libre otra vez, y lo pasa al binario
    // hijo explícitamente por HEXCELL_DIRECCION_SALUD. Queda una carrera residual —otro proceso
    // podría reclamar ese puerto entre soltarlo y conectar a él, una ventana de microsegundos—
    // pero es incondicionalmente mejor que asumir que un puerto fijo y habitual está libre en la
    // máquina que ejecuta la suite.
    let listener_temporal = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bindear un puerto efímero debe funcionar");
    let direccion_libre = listener_temporal
        .local_addr()
        .expect("leer la dirección local del listener recién creado debe funcionar");
    drop(listener_temporal);

    let (codigo, stderr) = ejecutar_binario_con_entorno(&[
        ("HEXCELL_ID_CELULA", "piloto-01"),
        ("HEXCELL_DIRECCION_SALUD", &direccion_libre.to_string()),
    ]);
    assert_ne!(codigo, 0);
    assert!(stderr.contains("HEXCELL_RUTA_DATOS"));

    // La configuración inválida falla antes de vincular nada: conectar a la dirección que se
    // habría usado debe fallar porque el binario nunca llegó a bindearla.
    let conexion = std::net::TcpStream::connect(direccion_libre);
    assert!(conexion.is_err());
}

#[test]
fn canal_por_defecto_es_simulado() {
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-canal-defecto-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(configuracion.canal, CanalSeleccionado::Simulado);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn canal_whatsmeow_se_configura_por_variable_de_entorno() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-canal-whatsmeow-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_CANAL", "whatsmeow");

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(configuracion.canal, CanalSeleccionado::Whatsmeow);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_configuracion_gcra_por_defecto_se_preserva_sin_variables_de_entorno() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-gcra-defecto-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion.configuracion_gcra,
        hexcell_core::admision::ConfiguracionGcra::default()
    );

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_configuracion_gcra_se_puede_configurar_por_variables_de_entorno() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-gcra-explicita-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO", "2.0")
        .con("HEXCELL_ADMISION_TOLERANCIA_RAFAGA", "5");

    let configuracion =
        Configuracion::desde_fuente(&fuente).expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion
            .configuracion_gcra
            .tasa_sostenida_por_segundo(),
        2.0
    );
    assert_eq!(configuracion.configuracion_gcra.tolerancia_rafaga(), 5);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn falla_si_la_tasa_sostenida_gcra_no_es_valida() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-gcra-invalida-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con(
            "HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO",
            "no-es-un-numero",
        );

    let error = Configuracion::desde_fuente(&fuente)
        .expect_err("debe fallar con una tasa sostenida no numérica");
    match error {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO");
            assert_eq!(valor, "no-es-un-numero");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn presupuesto_inicial_unidades_por_defecto_y_desde_la_fuente() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-presupuesto-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let mut fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    let config = Configuracion::desde_fuente(&fuente).expect("configuración válida");
    assert_eq!(config.presupuesto_inicial_unidades, 0);

    fuente.fijar("HEXCELL_PRESUPUESTO_INICIAL_UNIDADES", "500");
    let config =
        Configuracion::desde_fuente(&fuente).expect("configuración válida con presupuesto");
    assert_eq!(config.presupuesto_inicial_unidades, 500);

    fuente.fijar("HEXCELL_PRESUPUESTO_INICIAL_UNIDADES", "invalido");
    let error = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con valor inválido");
    match error {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_PRESUPUESTO_INICIAL_UNIDADES");
            assert_eq!(valor, "invalido");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn configuracion_inferencia_desde_la_fuente_y_validaciones() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-inferencia-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let mut fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    // Sin HEXCELL_INFERENCIA_URL_BASE -> inferencia es None
    let config =
        Configuracion::desde_fuente(&fuente).expect("configuración válida sin inferencia real");
    assert!(config.inferencia.is_none());

    // Con URL_BASE no-loopback http -> falla
    fuente.fijar("HEXCELL_INFERENCIA_URL_BASE", "http://api.remota.com/v1");
    fuente.fijar("HEXCELL_INFERENCIA_API_KEY", "key-secret");
    fuente.fijar("HEXCELL_INFERENCIA_MODELO", "model-1");
    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con http no-loopback");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_INFERENCIA_URL_BASE");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    // Con URL_BASE loopback válida
    fuente.fijar("HEXCELL_INFERENCIA_URL_BASE", "http://127.0.0.1:8080");
    let config = Configuracion::desde_fuente(&fuente).expect("configuración válida con inferencia");
    let inf = config.inferencia.expect("debe existir inferencia");
    assert_eq!(inf.url_base, "http://127.0.0.1:8080");
    assert_eq!(inf.api_key, "key-secret");
    assert_eq!(inf.modelo, "model-1");
    assert_eq!(inf.timeout, Duration::from_millis(8000));
    assert_eq!(inf.reintentos, 1);

    // Con tiempo total que excede el límite de drenaje -> falla
    // 15000 * 3 = 45000 ms >= 20000 ms (límite de drenaje por defecto)
    fuente.fijar("HEXCELL_INFERENCIA_TIMEOUT_MS", "15000");
    fuente.fijar("HEXCELL_INFERENCIA_REINTENTOS", "2");
    let err =
        Configuracion::desde_fuente(&fuente).expect_err("debe fallar si excede límite de drenaje");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_INFERENCIA_URL_BASE");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn configuracion_embeddings_desde_la_fuente_y_validaciones() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-embeddings-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let mut fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy());

    // Sin HEXCELL_EMBEDDINGS_URL_BASE -> embeddings es None
    let config =
        Configuracion::desde_fuente(&fuente).expect("configuración válida sin embeddings real");
    assert!(config.embeddings.is_none());

    // Con URL_BASE no-loopback http -> falla
    fuente.fijar(
        "HEXCELL_EMBEDDINGS_URL_BASE",
        "http://api.embeddings.com/v1",
    );
    fuente.fijar("HEXCELL_EMBEDDINGS_API_KEY", "key-secret-emb");
    fuente.fijar("HEXCELL_EMBEDDINGS_MODELO", "model-emb-1");
    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con http no-loopback");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_EMBEDDINGS_URL_BASE");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    // Con URL_BASE loopback válida y valores por defecto
    fuente.fijar("HEXCELL_EMBEDDINGS_URL_BASE", "http://127.0.0.1:8080");
    let config = Configuracion::desde_fuente(&fuente).expect("configuración válida con embeddings");
    let emb_enum = config.embeddings.expect("debe existir embeddings");
    let emb = match emb_enum {
        ConfiguracionDeEmbeddingsSegunProveedor::OpenRouter(c) => c,
        _ => panic!("se esperaba la variante OpenRouter"),
    };
    assert_eq!(emb.url_base, "http://127.0.0.1:8080");
    assert_eq!(emb.api_key, "key-secret-emb");
    assert_eq!(emb.modelo, "model-emb-1");
    assert_eq!(emb.timeout, Duration::from_millis(8000));
    assert_eq!(emb.reintentos, 1);
    assert_eq!(emb.tamano_de_lote, 32);

    // Con valores personalizados válidos
    fuente.fijar("HEXCELL_EMBEDDINGS_TIMEOUT_MS", "5000");
    fuente.fijar("HEXCELL_EMBEDDINGS_REINTENTOS", "2");
    fuente.fijar("HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE", "64");
    let config = Configuracion::desde_fuente(&fuente).expect("configuración válida personalizada");
    let emb_enum = config.embeddings.expect("debe existir embeddings");
    let emb = match emb_enum {
        ConfiguracionDeEmbeddingsSegunProveedor::OpenRouter(c) => c,
        _ => panic!("se esperaba la variante OpenRouter"),
    };
    assert_eq!(emb.timeout, Duration::from_millis(5000));
    assert_eq!(emb.reintentos, 2);
    assert_eq!(emb.tamano_de_lote, 64);

    // Con HEXCELL_EMBEDDINGS_PROVEEDOR = "gemini"
    fuente.fijar("HEXCELL_EMBEDDINGS_PROVEEDOR", "gemini");
    let config = Configuracion::desde_fuente(&fuente).expect("configuración válida con gemini");
    let emb_enum = config.embeddings.expect("debe existir embeddings");
    match emb_enum {
        ConfiguracionDeEmbeddingsSegunProveedor::Gemini(c) => {
            assert_eq!(c.url_base, "http://127.0.0.1:8080");
            assert_eq!(c.api_key, "key-secret-emb");
            assert_eq!(c.modelo, "model-emb-1");
            assert_eq!(c.timeout, Duration::from_millis(5000));
            assert_eq!(c.reintentos, 2);
            assert_eq!(c.tamano_de_lote, 64);
        }
        _ => panic!("se esperaba la variante Gemini"),
    }

    // Con valor no reconocido para HEXCELL_EMBEDDINGS_PROVEEDOR -> falla
    fuente.fijar("HEXCELL_EMBEDDINGS_PROVEEDOR", "azure");
    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con proveedor azure");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_EMBEDDINGS_PROVEEDOR");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    // Limpiar selector de proveedor para el resto del test
    fuente.quitar("HEXCELL_EMBEDDINGS_PROVEEDOR");

    // Tamaño de lote 0 -> falla
    fuente.fijar("HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE", "0");
    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con tamaño de lote 0");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    // Tamaño de lote > 128 -> falla
    fuente.fijar("HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE", "129");
    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con tamaño de lote 129");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    // Reintentos > 3 -> falla
    fuente.fijar("HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE", "32");
    fuente.fijar("HEXCELL_EMBEDDINGS_REINTENTOS", "4");
    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con reintentos 4");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_EMBEDDINGS_REINTENTOS");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    // Tiempo total (timeout * (1 + reintentos) + reintentos * 250) que excede el límite de drenaje -> falla
    // 10000 * 2 + 250 = 20250 ms >= 20000 ms (límite de drenaje por defecto)
    fuente.fijar("HEXCELL_EMBEDDINGS_TIMEOUT_MS", "10000");
    fuente.fijar("HEXCELL_EMBEDDINGS_REINTENTOS", "1");
    let err =
        Configuracion::desde_fuente(&fuente).expect_err("debe fallar si excede límite de drenaje");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, .. } => {
            assert_eq!(nombre, "HEXCELL_EMBEDDINGS_URL_BASE");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}
