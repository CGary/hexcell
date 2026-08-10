//! Doble de pruebas del sidecar: cada binario de test integra este módulo por separado
//! (`mod comun;`), así que no todos usan todos los métodos. Sigue el mismo patrón que
//! `crates/hexcell/tests/comun/mod.rs`.
#![allow(dead_code)]

use std::env;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

static CONTADOR_RUTAS: AtomicUsize = AtomicUsize::new(0);

/// Doble de pruebas simulado del sidecar.
pub struct SidecarSimulado {
    ruta_socket: PathBuf,
    listener: UnixListener,
    conexion: Option<(
        BufReader<tokio::io::ReadHalf<UnixStream>>,
        tokio::io::WriteHalf<UnixStream>,
    )>,
}

impl SidecarSimulado {
    /// Crea el directorio temporal y la ruta del socket.
    pub fn nuevo() -> Self {
        let mut ruta = env::temp_dir();
        ruta.push(format!(
            "hexcell-sidecar-test-{}-{}",
            process::id(),
            CONTADOR_RUTAS.fetch_add(1, Ordering::SeqCst)
        ));

        let listener = UnixListener::bind(&ruta).expect("no se pudo vincular el socket unix");

        Self {
            ruta_socket: ruta,
            listener,
            conexion: None,
        }
    }

    /// Devuelve la ruta del socket.
    pub fn ruta_socket(&self) -> &PathBuf {
        &self.ruta_socket
    }

    /// Acepta una conexión entrante.
    pub async fn aceptar_conexion(&mut self) {
        let (stream, _) = self
            .listener
            .accept()
            .await
            .expect("no se pudo aceptar la conexión");
        let (lectura, escritura) = tokio::io::split(stream);
        self.conexion = Some((BufReader::new(lectura), escritura));
    }

    /// Envía un saludo con la versión dada.
    pub async fn enviar_saludo(&mut self, version: i64, id_celula: &str) {
        let saludo = hexcell_canal_whatsmeow::mensajes::Saludo {
            version,
            tipo: "saludo".to_string(),
            emisor: "sidecar".to_string(),
            id_celula: id_celula.to_string(),
        };
        let linea = serde_json::to_string(&saludo).unwrap();
        self.enviar_linea_cruda(&linea).await;
    }

    /// Lee y devuelve el saludo del núcleo.
    pub async fn leer_saludo(&mut self) -> hexcell_canal_whatsmeow::mensajes::Saludo {
        let linea = self.leer_linea().await;
        serde_json::from_str(&linea).expect("no se pudo parsear el saludo")
    }

    /// Envía un evento entrante.
    pub async fn enviar_evento(
        &mut self,
        id_deduplicacion: &str,
        id_conversacion: &str,
        id_remitente: &str,
        contenido: &str,
        marca_temporal_ms: i64,
    ) {
        let evento = hexcell_canal_whatsmeow::mensajes::EventoEntranteIpc {
            version: 4,
            tipo: "evento_entrante".to_string(),
            id_deduplicacion: id_deduplicacion.to_string(),
            id_conversacion: id_conversacion.to_string(),
            id_remitente: id_remitente.to_string(),
            contenido: contenido.to_string(),
            marca_temporal_ms,
        };
        let linea = serde_json::to_string(&evento).unwrap();
        self.enviar_linea_cruda(&linea).await;
    }

    /// Lee y devuelve una confirmación del núcleo.
    pub async fn leer_confirmacion(&mut self) -> hexcell_canal_whatsmeow::mensajes::Confirmacion {
        let linea = self.leer_linea().await;
        serde_json::from_str(&linea).expect("no se pudo parsear la confirmación")
    }

    /// Envía un estado de sesión.
    pub async fn enviar_estado_sesion(
        &mut self,
        estado: &str,
        causa: &str,
        codigo: i64,
        expira_en_ms: i64,
    ) {
        let estado_sesion = hexcell_canal_whatsmeow::mensajes::EstadoSesionIpc {
            version: 4,
            tipo: "estado_sesion".to_string(),
            estado: estado.to_string(),
            causa: causa.to_string(),
            codigo,
            expira_en_ms,
        };
        let linea = serde_json::to_string(&estado_sesion).unwrap();
        self.enviar_linea_cruda(&linea).await;
    }

    /// Envía texto arbitrario para pruebas de errores de protocolo.
    pub async fn enviar_linea_cruda(&mut self, linea: &str) {
        let con = self.conexion.as_mut().expect("no hay conexión");
        con.1.write_all(linea.as_bytes()).await.unwrap();
        con.1.write_all(b"\n").await.unwrap();
        con.1.flush().await.unwrap();
    }

    /// Lee y devuelve un mensaje saliente del núcleo.
    pub async fn leer_mensaje_saliente(
        &mut self,
    ) -> hexcell_canal_whatsmeow::mensajes::MensajeSalienteIpc {
        let linea = self.leer_linea().await;
        serde_json::from_str(&linea).expect("no se pudo parsear el mensaje saliente")
    }

    /// Envía un acuse de envío.
    pub async fn enviar_acuse_envio(
        &mut self,
        id_mensaje: &str,
        estado: &str,
        id_correlacion: &str,
        motivo: &str,
        marca_temporal_ms: i64,
    ) {
        let acuse = hexcell_canal_whatsmeow::mensajes::AcuseEnvioIpc {
            version: 4,
            tipo: "acuse_envio".to_string(),
            id_mensaje: id_mensaje.to_string(),
            estado: estado.to_string(),
            id_correlacion: id_correlacion.to_string(),
            motivo: motivo.to_string(),
            marca_temporal_ms,
        };
        let linea = serde_json::to_string(&acuse).unwrap();
        self.enviar_linea_cruda(&linea).await;
    }

    /// Lee una línea cruda del núcleo.
    pub async fn leer_linea(&mut self) -> String {
        let con = self.conexion.as_mut().expect("no hay conexión");
        let mut linea = String::new();
        con.0.read_line(&mut linea).await.unwrap();
        linea.trim_end().to_string()
    }

    /// Cierra la conexión.
    pub fn cerrar(&mut self) {
        self.conexion = None;
    }
}

impl Drop for SidecarSimulado {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ruta_socket);
    }
}
