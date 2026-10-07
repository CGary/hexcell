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
            "{{\"version\":7,\"tipo\":\"saludo\",\"emisor\":\"sidecar\",\"id_celula\":\"{id_celula}\"}}\n"
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
            "{{\"version\":7,\"tipo\":\"evento_entrante\",\"id_deduplicacion\":\"{id_deduplicacion}\",\"id_conversacion\":\"{id_conversacion}\",\"id_remitente\":\"{id_remitente}\",\"contenido\":\"{contenido}\",\"marca_temporal_ms\":{marca_temporal_ms}}}\n"
        );
        con.1
            .write_all(frame.as_bytes())
            .await
            .expect("enviar evento entrante");
        con.1.flush().await.expect("flush evento entrante");
    }

    /// Envía un estado_sesion con la misma forma que los que produce el sidecar real
    /// (`sidecar/internal/ipc/mensajes.go`). `expira_en_ms` viaja tal cual (0 si no aplica).
    /// Lo usa el test de /health/ready con estado real: sin este método el FakeSidecar no
    /// podría alterar el watch del adaptador.
    async fn enviar_estado_sesion(
        &mut self,
        estado: &str,
        causa: &str,
        codigo: i64,
        expira_en_ms: i64,
    ) {
        let con = self.conexion.as_mut().expect("sin conexión activa");
        let frame = format!(
            "{{\"version\":7,\"tipo\":\"estado_sesion\",\"estado\":\"{estado}\",\"causa\":\"{causa}\",\"codigo\":{codigo},\"expira_en_ms\":{expira_en_ms}}}\n"
        );
        con.1
            .write_all(frame.as_bytes())
            .await
            .expect("enviar estado_sesion");
        con.1.flush().await.expect("flush estado_sesion");
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

/// Sondea `peticion_http_cruda` hasta que la respuesta empiece por el `status_line` esperado o
/// el plazo se agote. Devuelve la última respuesta leída. Nunca usa un sleep fijo como espera:
/// la condición puede cumplirse antes del plazo, y el plazo acota el peor caso.
fn sondear_http_hasta(direccion: &str, ruta: &str, status_line: &str, plazo: Duration) -> String {
    let limite = std::time::Instant::now() + plazo;
    let mut ultima;
    loop {
        ultima = comun::peticion_http_cruda(direccion, ruta);
        if ultima.starts_with(status_line) {
            return ultima;
        }
        if std::time::Instant::now() >= limite {
            return ultima;
        }
        std::thread::sleep(Duration::from_millis(50));
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

    // El watch del adaptador nace en Reconectando y el adaptador publica Activa tras el saludo
    // exitoso, así que /health/ready NO es 200 inmediatamente: se sondea con plazo en vez de
    // asumir un 200 inmediato, que con el estado real sería una carrera.
    let respuesta_ready = sondear_http_hasta(
        &binario.direccion,
        "/health/ready",
        "HTTP/1.1 200 OK",
        Duration::from_secs(5),
    );
    assert!(
        respuesta_ready.starts_with("HTTP/1.1 200 OK"),
        "/health/ready no llegó a 200 en 5s: {respuesta_ready}"
    );

    binario.enviar_sigterm();
    let salida = binario.esperar_salida(Duration::from_secs(5));
    assert!(salida.is_some_and(|s| s.success()));
}

// HEX-094 AC-4: /health/ready refleja el estado real del canal publicado por el sidecar a
// través del IPC. El watch del adaptador nace en Reconectando, así que primero esperamos a
// que el saludo dispare la publicación de Activa (200); luego el fake envía reconectando y
// /health/ready debe caer a 503 con componente `sesion-del-canal`; finalmente el fake envía
// activa y /health/ready vuelve a 200. Cada transición se observa por sondeo con plazo ≤ 5 s,
// nunca por sleep fijo.
#[tokio::test]
async fn salud_ready_refleja_el_estado_de_sesion_del_sidecar() {
    let mut sidecar = FakeSidecar::nuevo();
    let ruta_socket = sidecar.ruta_socket_str();
    let dir_temporal = comun::DirectorioTemporal::nuevo("canal-whatsmeow-salud-real");

    let mut binario = comun::lanzar_binario_con_variables(
        dir_temporal.ruta(),
        &[
            ("HEXCELL_CANAL", "whatsmeow"),
            ("HEXCELL_SOCKET_IPC", &ruta_socket),
        ],
    );

    sidecar.aceptar_y_saludar("piloto-01").await;

    // 1. Esperar a que el adaptador publique Activa tras el saludo (200).
    let primera = sondear_http_hasta(
        &binario.direccion,
        "/health/ready",
        "HTTP/1.1 200 OK",
        Duration::from_secs(5),
    );
    assert!(
        primera.starts_with("HTTP/1.1 200 OK"),
        "/health/ready no llegó a 200 tras el saludo: {primera}"
    );

    // 2. El sidecar falso publica reconectando: /health/ready debe caer a 503 con componente
    //    `sesion-del-canal`.
    sidecar
        .enviar_estado_sesion("reconectando", "fallo_de_conexion", 0, 0)
        .await;
    let tras_reconectando = sondear_http_hasta(
        &binario.direccion,
        "/health/ready",
        "HTTP/1.1 503",
        Duration::from_secs(5),
    );
    assert!(
        tras_reconectando.contains("sesion-del-canal"),
        "503 sin componente sesion-del-canal: {tras_reconectando}"
    );

    // 3. El sidecar falso publica activa: /health/ready vuelve a 200.
    sidecar.enviar_estado_sesion("activa", "", 0, 0).await;
    let tras_activa = sondear_http_hasta(
        &binario.direccion,
        "/health/ready",
        "HTTP/1.1 200 OK",
        Duration::from_secs(5),
    );
    assert!(
        tras_activa.starts_with("HTTP/1.1 200 OK"),
        "/health/ready no volvió a 200 tras activa: {tras_activa}"
    );

    binario.enviar_sigterm();
    let salida = binario.esperar_salida(Duration::from_secs(5));
    assert!(salida.is_some_and(|s| s.success()));
}
