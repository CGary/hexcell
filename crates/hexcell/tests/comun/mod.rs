//! Ayudas compartidas por los tests del binario de la célula.
//!
//! Todo test que necesite persistencia crea **su propio** directorio temporal con su propia
//! `sessions.db`, y lo borra al salir de alcance. Ninguna ruta es fija ni compartida: `cargo test`
//! corre los tests de un mismo binario en hilos distintos del mismo proceso, y dos tests que
//! abrieran la misma base se pisarían de una forma que depende del orden de planificación.
//!
//! No se usa ningún crate de directorios temporales: `configuracion.rs` y `salud_http.rs` ya
//! construían los suyos con `temp_dir()` y `process::id()` desde HEX-004, y esta ayuda extiende
//! ese patrón en vez de añadir una segunda manera de hacer lo mismo. Tampoco se añade ningún
//! cliente HTTP: se habla HTTP/1.1 a mano sobre un `TcpStream` de la biblioteca estándar, y ningún
//! test alcanza más red que el loopback que él mismo vincula.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hexcell_storage::{GestorDePools, RepositorioDeSesiones};

/// Distingue dos directorios creados por el mismo proceso: `process::id()` solo separa procesos.
static SECUENCIA: AtomicUsize = AtomicUsize::new(0);

/// Directorio temporal propio de un test, borrado al salir de alcance.
pub struct DirectorioTemporal {
    ruta: PathBuf,
}

impl DirectorioTemporal {
    /// Crea un directorio temporal único para este test.
    pub fn nuevo(etiqueta: &str) -> Self {
        let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
        let ruta = std::env::temp_dir().join(format!(
            "hexcell-test-{etiqueta}-{}-{secuencia}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&ruta);
        std::fs::create_dir_all(&ruta).expect("crear el directorio temporal del test");
        Self { ruta }
    }

    /// Ruta del directorio.
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }
}

impl Drop for DirectorioTemporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.ruta);
    }
}

/// Abre los pools sobre una ruta de datos y devuelve también el repositorio que el motor necesita.
///
/// Se devuelve el `Arc<GestorDePools>` además del repositorio porque los tests de preparación
/// necesitan las sondas de vitalidad, y los de reinicio necesitan poder **soltar** los pools para
/// cerrar de verdad los archivos antes de volver a abrirlos.
pub fn abrir_persistencia(ruta_datos: &Path) -> (Arc<GestorDePools>, Arc<RepositorioDeSesiones>) {
    let pools = Arc::new(GestorDePools::abrir(ruta_datos).expect("abrir la persistencia del test"));
    let repositorio = Arc::new(RepositorioDeSesiones::nuevo(Arc::clone(&pools)));
    (pools, repositorio)
}

/// Atajo para los tests que solo necesitan el repositorio.
pub fn repositorio_temporal(ruta_datos: &Path) -> Arc<RepositorioDeSesiones> {
    abrir_persistencia(ruta_datos).1
}

/// Binario `hexcell` lanzado para el test, con limpieza automática al salir de alcance.
pub struct BinarioDePrueba {
    proceso: Child,
    /// Dirección real que el binario imprimió al vincular su servidor de salud.
    pub direccion: String,
}

impl Drop for BinarioDePrueba {
    fn drop(&mut self) {
        let _ = self.proceso.kill();
        let _ = self.proceso.wait();
    }
}

/// Lanza el binario con `HEXCELL_DIRECCION_SALUD=127.0.0.1:0` para que el sistema operativo elija
/// un puerto libre, y lee de `stdout` el puerto que acabó vinculando. Ningún test de este
/// directorio asume un puerto fijo.
pub fn lanzar_binario_con_ruta_de_datos(ruta_datos: &Path) -> BinarioDePrueba {
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

/// Hace una petición HTTP/1.1 cruda al servidor de salud y devuelve la respuesta completa.
pub fn peticion_http_cruda(direccion: &str, ruta: &str) -> String {
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
