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
//!
//! # Por qué las dos tuberías del hijo se drenan en hilos propios (HEX-007)
//!
//! Antes de esta tarea, `lanzar_binario_con_ruta_de_datos` envolvía `stdout` en un `BufReader`
//! local y lo dejaba caer al volver: eso cierra el extremo de lectura de la tubería. Mientras el
//! binario no imprimía nada después del arranque no se notaba, pero desde que el motor emite una
//! línea de registro por cada evento procesado, el hijo recibiría `EPIPE` al escribir en una
//! tubería sin lector y `println!`/`registro::emitir` entrarían en pánico — y bajo
//! `panic = "abort"` eso es una muerte silenciosa. Por eso ambas tuberías se drenan aquí, en hilos
//! propios, durante toda la vida del proceso hijo, hacia un búfer compartido.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Extrae, sin ningún analizador JSON, el valor del campo `"detalle"` de una línea de registro ya
/// formada por `crate::registro::formatear`. Basta con buscar el literal `"campo":"` y leer hasta
/// la comilla de cierre: el formato lo controla este mismo árbol, así que no hace falta un
/// analizador completo para un valor que nunca lleva comillas internas sin escapar en estos tests.
fn extraer_campo<'a>(linea: &'a str, campo: &str) -> Option<&'a str> {
    let marca = format!("\"{campo}\":\"");
    let inicio = linea.find(&marca)? + marca.len();
    let resto = &linea[inicio..];
    let fin = resto.find('"')?;
    Some(&resto[..fin])
}

/// Binario `hexcell` lanzado para el test, con limpieza automática al salir de alcance.
///
/// Ambas tuberías del hijo se drenan en hilos de fondo durante toda su vida, hacia un búfer
/// compartido: ver la nota del módulo sobre por qué esto ya no es opcional desde HEX-007.
pub struct BinarioDePrueba {
    proceso: Child,
    buffer: Arc<Mutex<String>>,
    /// Dirección real que el binario imprimió al vincular su servidor de salud.
    pub direccion: String,
}

impl Drop for BinarioDePrueba {
    fn drop(&mut self) {
        let _ = self.proceso.kill();
        let _ = self.proceso.wait();
    }
}

impl BinarioDePrueba {
    /// Espera hasta `plazo` a que aparezca una línea que contenga `fragmento` en la salida
    /// capturada hasta ahora, sondeando el búfer compartido. Devuelve la línea completa.
    pub fn esperar_linea(&self, fragmento: &str, plazo: Duration) -> Option<String> {
        let limite = Instant::now() + plazo;
        loop {
            {
                let contenido = self
                    .buffer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(linea) = contenido.lines().find(|linea| linea.contains(fragmento)) {
                    return Some(linea.to_string());
                }
            }
            if Instant::now() >= limite {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Instantánea de toda la salida (`stdout` + `stderr`) capturada hasta este momento.
    pub fn salida_capturada(&self) -> String {
        self.buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Envía `SIGTERM` al proceso hijo con `/bin/kill`.
    ///
    /// No se añade `libc` como dependencia de test solo para invocar una función: el mismo trato
    /// que este árbol ya dio a la pila HTTP interna, escrita a mano sobre `TcpStream` en vez de
    /// sumar un cliente.
    pub fn enviar_sigterm(&self) {
        let pid = self.proceso.id().to_string();
        let estado = Command::new("/bin/kill").arg("-TERM").arg(&pid).status();
        assert!(
            estado.is_ok_and(|estado| estado.success()),
            "/bin/kill -TERM {pid} debe poder ejecutarse"
        );
    }

    /// Sondea `try_wait` hasta `plazo` y devuelve el estado de salida si el proceso ya terminó.
    pub fn esperar_salida(&mut self, plazo: Duration) -> Option<ExitStatus> {
        let limite = Instant::now() + plazo;
        loop {
            if let Ok(Some(estado)) = self.proceso.try_wait() {
                return Some(estado);
            }
            if Instant::now() >= limite {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Lanza el binario con `HEXCELL_DIRECCION_SALUD=127.0.0.1:0` para que el sistema operativo elija
/// un puerto libre, y lee de la salida capturada la dirección real que acabó vinculando (línea de
/// registro `salud_vinculada`). Ningún test de este directorio asume un puerto fijo.
pub fn lanzar_binario_con_ruta_de_datos(ruta_datos: &Path) -> BinarioDePrueba {
    lanzar_binario_con_variables(ruta_datos, &[])
}

/// Igual que [`lanzar_binario_con_ruta_de_datos`], con variables de entorno adicionales
/// (`HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE`, `HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS`, etc.).
pub fn lanzar_binario_con_variables(
    ruta_datos: &Path,
    variables_extra: &[(&str, &str)],
) -> BinarioDePrueba {
    let mut comando = Command::new(env!("CARGO_BIN_EXE_hexcell"));
    comando
        .env_clear()
        .env("HEXCELL_ID_CELULA", "piloto-01")
        .env("HEXCELL_RUTA_DATOS", ruta_datos)
        .env("HEXCELL_DIRECCION_SALUD", "127.0.0.1:0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (nombre, valor) in variables_extra {
        comando.env(nombre, valor);
    }

    let mut proceso = comando
        .spawn()
        .expect("el binario hexcell debe poder lanzarse");

    let salida_de_stdout = proceso
        .stdout
        .take()
        .expect("stdout del proceso hijo debe estar disponible");
    let salida_de_stderr = proceso
        .stderr
        .take()
        .expect("stderr del proceso hijo debe estar disponible");

    let buffer = Arc::new(Mutex::new(String::new()));

    let buffer_de_stdout = Arc::clone(&buffer);
    std::thread::spawn(move || drenar(BufReader::new(salida_de_stdout), &buffer_de_stdout));
    let buffer_de_stderr = Arc::clone(&buffer);
    std::thread::spawn(move || drenar(BufReader::new(salida_de_stderr), &buffer_de_stderr));

    let mut binario = BinarioDePrueba {
        proceso,
        buffer,
        direccion: String::new(),
    };

    let linea = binario
        .esperar_linea("salud_vinculada", Duration::from_secs(5))
        .unwrap_or_else(|| {
            let capturada = binario.salida_capturada();
            let _ = binario.proceso.kill();
            panic!("no se encontró la línea salud_vinculada en la salida del binario: {capturada}")
        });
    binario.direccion = extraer_campo(&linea, "detalle")
        .unwrap_or_else(|| panic!("la línea salud_vinculada no lleva campo detalle: {linea}"))
        .to_string();

    binario
}

/// Lee líneas del extremo dado hasta que se cierra, añadiéndolas al búfer compartido.
fn drenar(lector: BufReader<impl Read>, buffer: &Arc<Mutex<String>>) {
    for linea in lector.lines() {
        let Ok(linea) = linea else { break };
        let mut contenido = buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        contenido.push_str(&linea);
        contenido.push('\n');
    }
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
