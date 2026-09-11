//! Transporte HTTP/1.1 síncrono sobre el socket Unix del demonio de Docker.
//!
//! [`ConexionDocker`] habla la API del motor Docker por su socket Unix con un cliente HTTP/1.1
//! escrito a mano sobre [`std::os::unix::net::UnixStream`]: sin bollard, sin hyper y sin tokio.
//! Interpreta la línea de estado, las cabeceras, el cuerpo por `Content-Length` y el cuerpo por
//! `Transfer-Encoding: chunked`. Ningún camino termina en `panic`: una línea de estado inválida,
//! unas cabeceras truncadas o un flujo troceado que nunca termina se devuelven como
//! [`ErrorDeClienteDocker::RespuestaMalformada`] o [`ErrorDeClienteDocker::TiempoDeEsperaAgotado`].

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use super::error::ErrorDeClienteDocker;

/// Respuesta HTTP/1.1 interpretada del demonio de Docker.
///
/// El cuerpo es una secuencia de bytes sin interpretar: corresponde a quien consume la respuesta
/// decidir si es JSON o no (el módulo `cliente` lo analiza con `serde_json` cuando procede).
pub struct RespuestaHttp {
    /// Código de estado HTTP (200, 201, 204, 304, 404, 409, 500, …).
    pub estado: u16,
    /// Cabeceras en el orden en que llegaron, nombre y valor ya sin el espacio de separación.
    pub cabeceras: Vec<(String, String)>,
    /// Cuerpo de la respuesta, ya sin la codificación de transporte (Content-Length o chunked).
    pub cuerpo: Vec<u8>,
}

/// Conexión activa al socket Unix del demonio de Docker.
///
/// Encapsula el ciclo conectar → enviar petición → leer e interpretar respuesta. Una conexión
/// sirve exactamente una petición: la petición se escribe con `Connection: close` y el demonio
/// cierra tras responder, así que cada operación del cliente abre su propia `ConexionDocker`.
pub struct ConexionDocker {
    flujo: UnixStream,
}

impl ConexionDocker {
    /// Conecta al socket Unix en `ruta` acotando tanto la conexión como las lecturas y escrituras
    /// posteriores con `tiempo_limite`.
    ///
    /// La conexión se acota a mano porque [`UnixStream::connect`] no tiene `connect_timeout` como
    /// sí lo tiene `TcpStream`: se ejecuta en un hilo aparte que manda el resultado por un canal, y
    /// el hilo invocante espera con [`mpsc::Receiver::recv_timeout`]. Si se agota, se devuelve
    /// [`ErrorDeClienteDocker::TiempoDeEsperaAgotado`] y el hilo lanzado termina solo (se deja caer
    /// el receptor sin unirse).
    ///
    /// Tras conectar se fijan los tiempos límite de lectura y escritura con el mismo
    /// `tiempo_limite`, para que una respuesta que nunca llega tampoco cuelgue al invocante.
    pub fn conectar_con_tiempo_limite(
        ruta: &Path,
        tiempo_limite: Duration,
    ) -> Result<Self, ErrorDeClienteDocker> {
        let flujo = conectar_socket_con_limite(ruta, tiempo_limite)?;
        flujo
            .set_read_timeout(Some(tiempo_limite))
            .map_err(ErrorDeClienteDocker::Io)?;
        flujo
            .set_write_timeout(Some(tiempo_limite))
            .map_err(ErrorDeClienteDocker::Io)?;
        Ok(Self { flujo })
    }

    /// Envía una petición y devuelve la respuesta interpretada.
    ///
    /// `cuerpo` es el cuerpo de la petición, o `None` si la petición no lleva ninguno (arranque,
    /// parada, inspección y eliminación). El método escribe la línea de petición, la cabecera
    /// `Host: localhost` que la API del motor espera incluso sobre socket Unix, y `Content-Length`
    /// cuando hay cuerpo.
    pub fn enviar(
        &mut self,
        metodo: &str,
        ruta: &str,
        cuerpo: Option<&str>,
    ) -> Result<RespuestaHttp, ErrorDeClienteDocker> {
        self.escribir_peticion(metodo, ruta, cuerpo)?;
        self.leer_respuesta()
    }

    fn escribir_peticion(
        &mut self,
        metodo: &str,
        ruta: &str,
        cuerpo: Option<&str>,
    ) -> Result<(), ErrorDeClienteDocker> {
        let mut peticion = String::new();
        peticion.push_str(metodo);
        peticion.push(' ');
        peticion.push_str(ruta);
        peticion.push_str(" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
        if let Some(c) = cuerpo {
            peticion.push_str("Content-Type: application/json\r\n");
            peticion.push_str(&format!("Content-Length: {}\r\n", c.len()));
        }
        peticion.push_str("\r\n");
        if let Some(c) = cuerpo {
            peticion.push_str(c);
        }

        self.flujo
            .write_all(peticion.as_bytes())
            .map_err(clasificar_error_de_escritura)?;
        self.flujo.flush().map_err(clasificar_error_de_escritura)?;
        Ok(())
    }

    fn leer_respuesta(&mut self) -> Result<RespuestaHttp, ErrorDeClienteDocker> {
        let mut lector = BufReader::new(&self.flujo);
        let estado = leer_linea_de_estado(&mut lector)?;
        let cabeceras = leer_cabeceras(&mut lector)?;
        let cuerpo = leer_cuerpo(&mut lector, &cabeceras)?;
        Ok(RespuestaHttp {
            estado,
            cabeceras,
            cuerpo,
        })
    }
}

/// Ejecuta `UnixStream::connect` en un hilo aparte y espera el resultado con `recv_timeout`.
fn conectar_socket_con_limite(
    ruta: &Path,
    tiempo_limite: Duration,
) -> Result<UnixStream, ErrorDeClienteDocker> {
    let (emisor, receptor) = mpsc::channel();
    let ruta_propia = ruta.to_path_buf();
    std::thread::spawn(move || {
        let resultado = UnixStream::connect(&ruta_propia);
        let _ = emisor.send(resultado);
    });

    match receptor.recv_timeout(tiempo_limite) {
        Ok(Ok(flujo)) => Ok(flujo),
        Ok(Err(error)) => Err(clasificar_error_de_conexion(error)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ErrorDeClienteDocker::TiempoDeEsperaAgotado),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ErrorDeClienteDocker::DemonioInalcanzable),
    }
}

/// Traduce el error de `connect` a su variante: `EACCES` es permiso denegado, el resto es un
/// demonio inalcanzable (socket ausente, conexión rechazada, etc.).
fn clasificar_error_de_conexion(error: std::io::Error) -> ErrorDeClienteDocker {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        ErrorDeClienteDocker::PermisoDenegado
    } else {
        ErrorDeClienteDocker::DemonioInalcanzable
    }
}

/// Traduce un error de lectura: un agotamiento del tiempo límite es
/// [`ErrorDeClienteDocker::TiempoDeEsperaAgotado`], un cierre prematuro del flujo es una respuesta
/// malformada, y el resto es un error de E/S sin clasificar.
fn clasificar_error_de_lectura(error: std::io::Error) -> ErrorDeClienteDocker {
    match error.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            ErrorDeClienteDocker::TiempoDeEsperaAgotado
        }
        std::io::ErrorKind::UnexpectedEof => ErrorDeClienteDocker::RespuestaMalformada {
            motivo: "la respuesta se truncó antes de completarse".to_string(),
        },
        _ => ErrorDeClienteDocker::Io(error),
    }
}

/// Traduce un error de escritura: un agotamiento del tiempo límite es
/// [`ErrorDeClienteDocker::TiempoDeEsperaAgotado`], el resto es un error de E/S sin clasificar.
fn clasificar_error_de_escritura(error: std::io::Error) -> ErrorDeClienteDocker {
    match error.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            ErrorDeClienteDocker::TiempoDeEsperaAgotado
        }
        _ => ErrorDeClienteDocker::Io(error),
    }
}

/// Lee una línea terminada en `\n` y la devuelve sin el `\r\n` final.
///
/// Es estricta: si el flujo termina sin un salto de línea, la respuesta se da por truncada y se
/// devuelve [`ErrorDeClienteDocker::RespuestaMalformada`].
fn leer_linea_cruda(lector: &mut impl BufRead) -> Result<String, ErrorDeClienteDocker> {
    let mut bufer = Vec::new();
    let leidos = lector
        .read_until(b'\n', &mut bufer)
        .map_err(clasificar_error_de_lectura)?;
    if leidos == 0 || !bufer.ends_with(b"\n") {
        return Err(ErrorDeClienteDocker::RespuestaMalformada {
            motivo: "la respuesta terminó antes de completar una línea".to_string(),
        });
    }
    bufer.pop();
    if bufer.ends_with(b"\r") {
        bufer.pop();
    }
    String::from_utf8(bufer).map_err(|_| ErrorDeClienteDocker::RespuestaMalformada {
        motivo: "la línea no es UTF-8 válido".to_string(),
    })
}

/// Lee la línea de estado `HTTP/1.1 <código> <razón>` y devuelve el código.
fn leer_linea_de_estado(lector: &mut impl BufRead) -> Result<u16, ErrorDeClienteDocker> {
    let linea = leer_linea_cruda(lector)?;
    let codigo = linea.split_whitespace().nth(1).ok_or_else(|| {
        ErrorDeClienteDocker::RespuestaMalformada {
            motivo: "la línea de estado no lleva código".to_string(),
        }
    })?;
    codigo
        .parse::<u16>()
        .map_err(|_| ErrorDeClienteDocker::RespuestaMalformada {
            motivo: "el código de estado no es un número".to_string(),
        })
}

/// Lee las cabeceras hasta la línea vacía y las devuelve en orden, sin el espacio de separación.
fn leer_cabeceras(
    lector: &mut impl BufRead,
) -> Result<Vec<(String, String)>, ErrorDeClienteDocker> {
    let mut cabeceras = Vec::new();
    loop {
        let linea = leer_linea_cruda(lector)?;
        if linea.is_empty() {
            break;
        }
        let (nombre, valor) =
            linea
                .split_once(':')
                .ok_or_else(|| ErrorDeClienteDocker::RespuestaMalformada {
                    motivo: "cabecera sin dos puntos".to_string(),
                })?;
        cabeceras.push((nombre.trim().to_string(), valor.trim().to_string()));
    }
    Ok(cabeceras)
}

/// Busca una cabecera por nombre, sin distinguir mayúsculas de minúsculas.
fn buscar_cabecera<'a>(cabeceras: &'a [(String, String)], nombre: &str) -> Option<&'a str> {
    cabeceras
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(nombre))
        .map(|(_, valor)| valor.as_str())
}

/// Lee el cuerpo según las cabeceras: `Transfer-Encoding: chunked` primero, después
/// `Content-Length`; si no hay ninguna de las dos, la respuesta no lleva cuerpo.
fn leer_cuerpo(
    lector: &mut impl BufRead,
    cabeceras: &[(String, String)],
) -> Result<Vec<u8>, ErrorDeClienteDocker> {
    if let Some(valor) = buscar_cabecera(cabeceras, "transfer-encoding")
        && valor.to_ascii_lowercase().contains("chunked")
    {
        return leer_cuerpo_troceado(lector);
    }
    if let Some(valor) = buscar_cabecera(cabeceras, "content-length") {
        let longitud: usize =
            valor
                .trim()
                .parse()
                .map_err(|_| ErrorDeClienteDocker::RespuestaMalformada {
                    motivo: "Content-Length no es un número válido".to_string(),
                })?;
        let mut cuerpo = vec![0u8; longitud];
        lector
            .read_exact(&mut cuerpo)
            .map_err(clasificar_error_de_lectura)?;
        return Ok(cuerpo);
    }
    Ok(Vec::new())
}

/// Lee un cuerpo codificado en `Transfer-Encoding: chunked`, fragmento a fragmento.
fn leer_cuerpo_troceado(lector: &mut impl BufRead) -> Result<Vec<u8>, ErrorDeClienteDocker> {
    let mut cuerpo = Vec::new();
    loop {
        let linea = leer_linea_cruda(lector)?;
        let tamano_hex = linea.split(';').next().unwrap_or("").trim();
        let tamano = usize::from_str_radix(tamano_hex, 16).map_err(|_| {
            ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "el tamaño de fragmento no es hexadecimal válido".to_string(),
            }
        })?;

        if tamano == 0 {
            loop {
                let cola = leer_linea_cruda(lector)?;
                if cola.is_empty() {
                    break;
                }
            }
            break;
        }

        let mut fragmento = vec![0u8; tamano];
        lector
            .read_exact(&mut fragmento)
            .map_err(clasificar_error_de_lectura)?;
        let mut crlf = [0u8; 2];
        lector
            .read_exact(&mut crlf)
            .map_err(clasificar_error_de_lectura)?;
        if crlf != *b"\r\n" {
            return Err(ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "fragmento sin el CRLF de cierre".to_string(),
            });
        }
        cuerpo.extend_from_slice(&fragmento);
    }
    Ok(cuerpo)
}
