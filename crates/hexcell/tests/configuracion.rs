//! Tests de `Configuracion::desde_entorno`: camino feliz y cada modo de fallo.
//!
//! La mitad de estos tests son a nivel de biblioteca (llaman a `Configuracion::desde_entorno`
//! directamente) y la otra mitad son a nivel de proceso: lanzan `env!("CARGO_BIN_EXE_hexcell")`
//! con un entorno controlado y comprueban el código de salida y `stderr`, que es lo único que
//! demuestra de verdad que el binario termina **antes** de vincular nada (AC-2).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use hexcell::configuracion::{CanalSeleccionado, Configuracion, ErrorDeConfiguracion};

/// `cargo test` ejecuta los tests de un mismo binario en hilos distintos del mismo proceso, y
/// `std::env::set_var`/`remove_var` son estado **del proceso completo**, no del hilo. Sin esta
/// exclusión mutua, dos tests de este archivo que fijan variables `HEXCELL_*` distintas a la vez
/// se pisan entre sí y el resultado depende de una carrera. Cada test que toque el entorno del
/// proceso adquiere este cerrojo antes de tocar nada y lo mantiene vivo durante todo su cuerpo.
static CERROJO_DE_ENTORNO: Mutex<()> = Mutex::new(());

fn limpiar_entorno_de_hexcell() {
    for variable in [
        "HEXCELL_ID_CELULA",
        "HEXCELL_RUTA_DATOS",
        "HEXCELL_DIRECCION_SALUD",
        "HEXCELL_CANAL",
        "HEXCELL_CAPACIDAD_COLA",
        "HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS",
        "HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO",
        "HEXCELL_ADMISION_TOLERANCIA_RAFAGA",
    ] {
        unsafe {
            std::env::remove_var(variable);
        }
    }
}

#[test]
fn arranca_con_configuracion_valida() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-config-ok-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
    assert_eq!(configuracion.id_celula, "piloto-01");
    assert_eq!(configuracion.ruta_datos, directorio_temporal);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn falla_si_falta_la_ruta_de_datos() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
    }

    let error = Configuracion::desde_entorno().expect_err("debe fallar sin HEXCELL_RUTA_DATOS");
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
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let ruta_inexistente =
        std::env::temp_dir().join("hexcell-ruta-que-nunca-existe-en-este-test-12345");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &ruta_inexistente);
    }

    let error =
        Configuracion::desde_entorno().expect_err("debe fallar si la ruta no existe en disco");
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
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-direccion-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var("HEXCELL_DIRECCION_SALUD", "no-es-un-socket");
    }

    let error = Configuracion::desde_entorno().expect_err("debe fallar con una dirección inválida");
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
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-config-canal-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var("HEXCELL_CANAL", "canal-que-no-existe");
    }

    let error = Configuracion::desde_entorno().expect_err("debe fallar con un canal desconocido");
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
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-ventana-defecto-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion.ventana_deduplicacion,
        Duration::from_secs(3600)
    );

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_ventana_de_deduplicacion_se_puede_configurar_por_variable_de_entorno() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-ventana-explicita-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var("HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS", "120");
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion.ventana_deduplicacion,
        Duration::from_secs(120)
    );

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn falla_si_la_ventana_de_deduplicacion_no_es_un_entero_positivo() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-ventana-invalida-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var("HEXCELL_VENTANA_DEDUPLICACION_SEGUNDOS", "no-es-un-entero");
    }

    let error =
        Configuracion::desde_entorno().expect_err("debe fallar con una ventana no numérica");
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
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-canal-defecto-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
    assert_eq!(configuracion.canal, CanalSeleccionado::Simulado);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn canal_whatsmeow_se_configura_por_variable_de_entorno() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-canal-whatsmeow-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var("HEXCELL_CANAL", "whatsmeow");
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
    assert_eq!(configuracion.canal, CanalSeleccionado::Whatsmeow);

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_configuracion_gcra_por_defecto_se_preserva_sin_variables_de_entorno() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-gcra-defecto-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
    assert_eq!(
        configuracion.configuracion_gcra,
        hexcell_core::admision::ConfiguracionGcra::default()
    );

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn la_configuracion_gcra_se_puede_configurar_por_variables_de_entorno() {
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-gcra-explicita-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var("HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO", "2.0");
        std::env::set_var("HEXCELL_ADMISION_TOLERANCIA_RAFAGA", "5");
    }

    let configuracion =
        Configuracion::desde_entorno().expect("la configuración válida no debe fallar");
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
    let _guardia = CERROJO_DE_ENTORNO.lock().unwrap_or_else(|e| e.into_inner());
    limpiar_entorno_de_hexcell();
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-gcra-invalida-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");
    unsafe {
        std::env::set_var("HEXCELL_ID_CELULA", "piloto-01");
        std::env::set_var("HEXCELL_RUTA_DATOS", &directorio_temporal);
        std::env::set_var(
            "HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO",
            "no-es-un-numero",
        );
    }

    let error =
        Configuracion::desde_entorno().expect_err("debe fallar con una tasa sostenida no numérica");
    match error {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO");
            assert_eq!(valor, "no-es-un-numero");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}
