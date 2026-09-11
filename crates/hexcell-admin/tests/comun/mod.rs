//! Ayudas compartidas por los tests del cliente del socket Unix de Docker.
//!
//! Todo test levanta su **propio** demonio falso sobre un socket Unix temporal que borra al salir
//! de alcance, y ninguno toca un daemon real ni la red: la API del motor se simula leyendo la
//! petición y escribiendo una respuesta programada, todo sobre `std::os::unix::net` y
//! `std::thread`, sin ningún runtime asíncrono ni dependencia nueva.
//!
//! El hilo que atiende el socket corre aparte porque el cliente bloquea esperando la respuesta:
//! si el demonio falso atendiera en el hilo del test, el test se quedaría esperando una conexión
//! que nadie acepta.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Distingue dos sockets creados por el mismo proceso: `process::id()` solo separa procesos.
static SECUENCIA: AtomicUsize = AtomicUsize::new(0);

/// Petición que el demonio falso leyó de una conexión.
pub struct PeticionRecibida {
    /// Método HTTP en mayúsculas (`POST`, `GET`, `DELETE`).
    pub metodo: String,
    /// Ruta y consulta tal y como llegaron, p. ej. `/containers/abc/stop?t=30`.
    pub objetivo: String,
    /// Cuerpo de la petición, ya sin la codificación de transporte.
    pub cuerpo: Vec<u8>,
}

/// Respuesta programada que el demonio falso escribe en una conexión.
pub enum Guion {
    /// Respuesta HTTP normal con cuerpo (Content-Length).
    ConCuerpo {
        estado: u16,
        razon: &'static str,
        cuerpo: &'static [u8],
    },
    /// Respuesta HTTP con el cuerpo en `Transfer-Encoding: chunked` (para ejercitar el lector de
    /// troceado del transporte).
    Troceado {
        estado: u16,
        razon: &'static str,
        cuerpo: &'static [u8],
    },
    /// Respuesta HTTP sin cuerpo (204/304/404/409).
    SinCuerpo { estado: u16, razon: &'static str },
    /// Escribe bytes crudos inválidos (para el caso de respuesta malformada).
    Crudo(&'static [u8]),
    /// Acepta la conexión y no escribe nada (para el caso de tiempo de espera agotado).
    Mudo,
}

/// Demonio de Docker falso: vincula un socket Unix temporal y atiende una conexión por llamada a
/// [`ServidorDockerFalso::atender`], en el hilo que la invoca.
pub struct ServidorDockerFalso {
    listener: UnixListener,
    ruta: PathBuf,
}

impl ServidorDockerFalso {
    /// Vincula un socket Unix en una ruta temporal única para este test.
    pub fn nuevo(etiqueta: &str) -> Self {
        let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
        let ruta = std::env::temp_dir().join(format!(
            "hexcell-docker-{etiqueta}-{}-{secuencia}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&ruta);
        let listener = UnixListener::bind(&ruta).expect("vincular el socket del demonio falso");
        Self { listener, ruta }
    }

    /// Ruta del socket, para pasársela al cliente bajo prueba.
    pub fn ruta(&self) -> PathBuf {
        self.ruta.clone()
    }

    /// Acepta una conexión, lee la petición, escribe la respuesta programada y devuelve la
    /// petición para que el test la compruebe.
    ///
    /// Se invoca desde el hilo que atiende el demonio falso, no desde el hilo del test.
    pub fn atender(&self, guion: Guion) -> PeticionRecibida {
        let (mut flujo, _) = self
            .listener
            .accept()
            .expect("aceptar la conexión del cliente");
        let peticion = leer_peticion(&mut flujo);
        aplicar_guion(&mut flujo, guion);
        peticion
    }
}

impl Drop for ServidorDockerFalso {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ruta);
    }
}

/// Ruta temporal de socket sin vincular: para el caso de demonio inalcanzable.
pub fn ruta_socket_sin_vincular(etiqueta: &str) -> PathBuf {
    let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "hexcell-docker-{etiqueta}-{}-{secuencia}",
        std::process::id()
    ))
}

/// Lee la línea de petición, las cabeceras y el cuerpo (por Content-Length) de una conexión.
pub fn leer_peticion(flujo: &mut UnixStream) -> PeticionRecibida {
    let mut lector = BufReader::new(&mut *flujo);

    let mut linea_de_peticion = String::new();
    lector
        .read_line(&mut linea_de_peticion)
        .expect("leer la línea de petición");
    let mut partes = linea_de_peticion.split_whitespace();
    let metodo = partes.next().expect("método").to_string();
    let objetivo = partes.next().expect("objetivo").to_string();

    let mut longitud_de_cuerpo = 0usize;
    loop {
        let mut cabecera = String::new();
        lector.read_line(&mut cabecera).expect("leer cabecera");
        let cabecera = cabecera.trim_end();
        if cabecera.is_empty() {
            break;
        }
        if let Some((nombre, valor)) = cabecera.split_once(':') {
            if nombre.trim().eq_ignore_ascii_case("content-length") {
                longitud_de_cuerpo = valor.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut cuerpo = vec![0u8; longitud_de_cuerpo];
    if longitud_de_cuerpo > 0 {
        lector
            .read_exact(&mut cuerpo)
            .expect("leer el cuerpo de la petición");
    }

    PeticionRecibida {
        metodo,
        objetivo,
        cuerpo,
    }
}

/// Escribe en la conexión la respuesta que dicta el guion.
fn aplicar_guion(flujo: &mut UnixStream, guion: Guion) {
    match guion {
        Guion::ConCuerpo {
            estado,
            razon,
            cuerpo,
        } => {
            let cabecera = format!(
                "HTTP/1.1 {estado} {razon}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                cuerpo.len()
            );
            flujo.write_all(cabecera.as_bytes()).unwrap();
            flujo.write_all(cuerpo).unwrap();
            flujo.flush().unwrap();
        }
        Guion::Troceado {
            estado,
            razon,
            cuerpo,
        } => {
            let cabecera = format!(
                "HTTP/1.1 {estado} {razon}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n"
            );
            flujo.write_all(cabecera.as_bytes()).unwrap();
            if !cuerpo.is_empty() {
                flujo
                    .write_all(format!("{:x}\r\n", cuerpo.len()).as_bytes())
                    .unwrap();
                flujo.write_all(cuerpo).unwrap();
                flujo.write_all(b"\r\n").unwrap();
            }
            flujo.write_all(b"0\r\n\r\n").unwrap();
            flujo.flush().unwrap();
        }
        Guion::SinCuerpo { estado, razon } => {
            let cabecera = format!("HTTP/1.1 {estado} {razon}\r\n\r\n");
            flujo.write_all(cabecera.as_bytes()).unwrap();
            flujo.flush().unwrap();
        }
        Guion::Crudo(bytes) => {
            flujo.write_all(bytes).unwrap();
            flujo.flush().unwrap();
        }
        Guion::Mudo => {
            // Acepta y no escribe nada: el cliente debe agotar su tiempo límite de lectura. El
            // hilo queda aparcado para siempre sosteniendo el socket abierto; no se une.
            std::thread::park();
        }
    }
}
