//! Tests HTTP crudos de `/health/live` y `/health/ready` contra el binario real.
//!
//! Se lanza `env!("CARGO_BIN_EXE_hexcell")` con `HEXCELL_DIRECCION_SALUD=127.0.0.1:0` para que el
//! sistema operativo elija un puerto libre, se lee el puerto real que el binario imprime en
//! `stdout` al arrancar, y se habla HTTP/1.1 a mano sobre un `std::net::TcpStream`: no se añade
//! ningún crate cliente HTTP solo para probar, y ningún test alcanza más red que loopback.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Binario `hexcell` lanzado para el test, con limpieza automática al salir de alcance.
struct BinarioDePrueba {
    proceso: Child,
    direccion: String,
}

impl Drop for BinarioDePrueba {
    fn drop(&mut self) {
        let _ = self.proceso.kill();
        let _ = self.proceso.wait();
    }
}

fn lanzar_binario_con_ruta_de_datos(ruta_datos: &std::path::Path) -> BinarioDePrueba {
    let mut proceso = Command::new(env!("CARGO_BIN_EXE_hexcell"))
        .env_clear()
        .env("HEXCELL_ID_CELULA", "piloto-01")
        .env("HEXCELL_RUTA_DATOS", ruta_datos)
        .env("HEXCELL_DIRECCION_SALUD", "127.0.0.1:0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("el binario hexcell debe poder lanzarse");

    let salida = proceso
        .stdout
        .take()
        .expect("stdout del proceso hijo debe estar disponible");
    let mut lector = BufReader::new(salida);
    let mut direccion = None;
    for _ in 0..20 {
        let mut linea = String::new();
        if lector.read_line(&mut linea).unwrap_or(0) == 0 {
            break;
        }
        if let Some(resto) = linea
            .trim()
            .strip_prefix("hexcell: servidor de salud escuchando en ")
        {
            direccion = Some(resto.to_string());
            break;
        }
    }

    let direccion = direccion.unwrap_or_else(|| {
        let _ = proceso.kill();
        panic!("no se encontró la línea con la dirección de salud en stdout del binario")
    });

    BinarioDePrueba { proceso, direccion }
}

fn peticion_http_cruda(direccion: &str, ruta: &str) -> String {
    let mut intentos_restantes = 20;
    let mut flujo = loop {
        match TcpStream::connect(direccion) {
            Ok(flujo) => break flujo,
            Err(_) if intentos_restantes > 0 => {
                intentos_restantes -= 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("no se pudo conectar a {direccion}: {error}"),
        }
    };

    let peticion = format!("GET {ruta} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    flujo
        .write_all(peticion.as_bytes())
        .expect("escribir la petición cruda no debe fallar");

    let mut respuesta = String::new();
    flujo
        .read_to_string(&mut respuesta)
        .expect("leer la respuesta cruda no debe fallar");
    respuesta
}

#[test]
fn health_live_responde_200_sin_comprobar_nada() {
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-salud-live-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let binario = lanzar_binario_con_ruta_de_datos(&directorio_temporal);
    let respuesta = peticion_http_cruda(&binario.direccion, "/health/live");

    assert!(respuesta.starts_with("HTTP/1.1 200"));

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn health_ready_responde_con_el_esqueleto_documentado() {
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-salud-ready-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let binario = lanzar_binario_con_ruta_de_datos(&directorio_temporal);
    let respuesta = peticion_http_cruda(&binario.direccion, "/health/ready");

    assert!(respuesta.starts_with("HTTP/1.1 200"));
    assert!(respuesta.contains("HEX-006"));

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[test]
fn una_ruta_desconocida_responde_404() {
    let directorio_temporal =
        std::env::temp_dir().join(format!("hexcell-test-salud-404-{}", std::process::id()));
    std::fs::create_dir_all(&directorio_temporal)
        .expect("crear el directorio temporal del test debe funcionar");

    let binario = lanzar_binario_con_ruta_de_datos(&directorio_temporal);
    let respuesta = peticion_http_cruda(&binario.direccion, "/no-existe");

    assert!(respuesta.starts_with("HTTP/1.1 404"));

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}
