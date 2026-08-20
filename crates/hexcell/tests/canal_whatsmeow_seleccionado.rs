//! Tests de integración del canal `whatsmeow` seleccionado en el binario `hexcell`.
//!
//! Comprueba que al configurar `HEXCELL_CANAL=whatsmeow` y `HEXCELL_SOCKET_IPC`, el binario
//! real de la célula levanta `AdaptadorWhatsmeow`, conecta con el sidecar IPC, realiza el
//! saludo, entrega eventos entrantes al motor y emite las respuestas salientes producidas por
//! `ProveedorSimulado`.

mod comun;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

static CONTADOR_RUTAS: AtomicUsize = AtomicUsize::new(0);

struct FakeSidecar {
    ruta_socket: PathBuf,
    listener: UnixListener,
    conexion: Option<(
        BufReader<tokio::io::ReadHalf<UnixStream>>,
        tokio::io::WriteHalf<UnixStream>,
    )>,
}

impl FakeSidecar {
    fn nuevo() -> Self {
        let mut ruta = std::env::temp_dir();
        ruta.push(format!(
            "hexcell-fake-sidecar-sel-{}-{}",
            std::process::id(),
            CONTADOR_RUTAS.fetch_add(1, Ordering::SeqCst)
        ));

        let listener = UnixListener::bind(&ruta).expect("vincular socket unix");

        Self {
            ruta_socket: ruta,
            listener,
            conexion: None,
        }
    }

    fn ruta_socket_str(&self) -> String {
        self.ruta_socket.to_string_lossy().to_string()
    }

    async fn aceptar_y_saludar(&mut self, id_celula: &str) {
        let (stream, _) = self.listener.accept().await.expect("aceptar conexión IPC");
        let (lectura, mut escritura) = tokio::io::split(stream);
        let mut lector = BufReader::new(lectura);

        let mut linea_saludo = String::new();
        lector
            .read_line(&mut linea_saludo)
            .await
            .expect("leer saludo del núcleo");
        assert!(linea_saludo.contains("\"tipo\":\"saludo\""));
        assert!(linea_saludo.contains("\"emisor\":\"nucleo\""));

        let saludo_sidecar = format!(
            "{{\"version\":5,\"tipo\":\"saludo\",\"emisor\":\"sidecar\",\"id_celula\":\"{id_celula}\"}}\n"
        );
        escritura
            .write_all(saludo_sidecar.as_bytes())
            .await
            .expect("escribir saludo del sidecar");
        escritura.flush().await.expect("flush de saludo");

        self.conexion = Some((lector, escritura));
    }

    async fn enviar_evento_entrante(
        &mut self,
        id_deduplicacion: &str,
        id_conversacion: &str,
        id_remitente: &str,
        contenido: &str,
        marca_temporal_ms: i64,
    ) {
        let con = self.conexion.as_mut().expect("sin conexión activa");
        let frame = format!(
            "{{\"version\":5,\"tipo\":\"evento_entrante\",\"id_deduplicacion\":\"{id_deduplicacion}\",\"id_conversacion\":\"{id_conversacion}\",\"id_remitente\":\"{id_remitente}\",\"contenido\":\"{contenido}\",\"marca_temporal_ms\":{marca_temporal_ms}}}\n"
        );
        con.1
            .write_all(frame.as_bytes())
            .await
            .expect("enviar evento entrante");
        con.1.flush().await.expect("flush evento entrante");
    }

    async fn leer_linea_con_plazo(&mut self, plazo: Duration) -> String {
        let con = self.conexion.as_mut().expect("sin conexión activa");
        let mut linea = String::new();
        tokio::time::timeout(plazo, con.0.read_line(&mut linea))
            .await
            .expect("tiempo de lectura agotado")
            .expect("leer línea");
        linea.trim_end().to_string()
    }
}

impl Drop for FakeSidecar {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ruta_socket);
    }
}

#[tokio::test]
async fn canal_whatsmeow_procesa_evento_entrante_y_responde() {
    let mut sidecar = FakeSidecar::nuevo();
    let ruta_socket = sidecar.ruta_socket_str();
    let dir_temporal = comun::DirectorioTemporal::nuevo("canal-whatsmeow-e2e");

    let mut binario = comun::lanzar_binario_con_variables(
        dir_temporal.ruta(),
        &[
            ("HEXCELL_CANAL", "whatsmeow"),
            ("HEXCELL_SOCKET_IPC", &ruta_socket),
        ],
    );

    sidecar.aceptar_y_saludar("piloto-01").await;

    sidecar
        .enviar_evento_entrante("dedup-01", "conv-01", "rem-01", "hola", 1700000000000)
        .await;

    // Primer mensaje recibido: confirmación del evento entrante por parte del adaptador
    let linea_confirmacion = sidecar.leer_linea_con_plazo(Duration::from_secs(5)).await;
    assert!(linea_confirmacion.contains("\"tipo\":\"confirmacion\""));
    assert!(linea_confirmacion.contains("\"id_deduplicacion\":\"dedup-01\""));

    // Segundo mensaje recibido: respuesta saliente generada por el motor (ProveedorSimulado)
    let linea_saliente = sidecar.leer_linea_con_plazo(Duration::from_secs(5)).await;
    assert!(linea_saliente.contains("\"tipo\":\"mensaje_saliente\""));
    assert!(linea_saliente.contains("\"id_conversacion\":\"conv-01\""));
    assert!(linea_saliente.contains("\"marca_temporal_origen_ms\":1700000000000"));

    binario.enviar_sigterm();
    let salida = binario.esperar_salida(Duration::from_secs(5));
    assert!(salida.is_some_and(|s| s.success()));
}

#[tokio::test]
async fn servidor_de_salud_responde_con_canal_whatsmeow() {
    let mut sidecar = FakeSidecar::nuevo();
    let ruta_socket = sidecar.ruta_socket_str();
    let dir_temporal = comun::DirectorioTemporal::nuevo("canal-whatsmeow-salud");

    let mut binario = comun::lanzar_binario_con_variables(
        dir_temporal.ruta(),
        &[
            ("HEXCELL_CANAL", "whatsmeow"),
            ("HEXCELL_SOCKET_IPC", &ruta_socket),
        ],
    );

    sidecar.aceptar_y_saludar("piloto-01").await;

    let respuesta_live = comun::peticion_http_cruda(&binario.direccion, "/health/live");
    assert!(respuesta_live.starts_with("HTTP/1.1 200 OK"));

    let respuesta_ready = comun::peticion_http_cruda(&binario.direccion, "/health/ready");
    assert!(respuesta_ready.starts_with("HTTP/1.1 200 OK"));

    binario.enviar_sigterm();
    let salida = binario.esperar_salida(Duration::from_secs(5));
    assert!(salida.is_some_and(|s| s.success()));
}
