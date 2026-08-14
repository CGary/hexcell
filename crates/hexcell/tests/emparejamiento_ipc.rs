use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hexcell::emparejar::{
    ErrorModoEmparejar, ResultadoEmparejamiento, ejecutar, ordenar_emparejamiento,
};
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::mensajes::CodigoEmparejamiento;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
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
            "hexcell-fake-sidecar-emp-{}-{}",
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

    fn ruta_socket(&self) -> &PathBuf {
        &self.ruta_socket
    }

    async fn aceptar_y_saludar(&mut self, id_celula: &str) {
        let (stream, _) = self.listener.accept().await.expect("aceptar conexion");
        let (lectura, mut escritura) = tokio::io::split(stream);
        let mut lector = BufReader::new(lectura);

        // Lee saludo del núcleo
        let mut linea_saludo = String::new();
        lector.read_line(&mut linea_saludo).await.unwrap();

        // Envía saludo del sidecar
        let saludo_sidecar = format!(
            "{{\"version\":4,\"tipo\":\"saludo\",\"emisor\":\"sidecar\",\"id_celula\":\"{id_celula}\"}}\n"
        );
        escritura
            .write_all(saludo_sidecar.as_bytes())
            .await
            .unwrap();
        escritura.flush().await.unwrap();

        self.conexion = Some((lector, escritura));
    }

    async fn leer_linea(&mut self) -> String {
        let con = self.conexion.as_mut().expect("sin conexion");
        let mut linea = String::new();
        con.0.read_line(&mut linea).await.unwrap();
        linea.trim_end().to_string()
    }

    async fn enviar_linea(&mut self, linea: &str) {
        let con = self.conexion.as_mut().expect("sin conexion");
        con.1.write_all(linea.as_bytes()).await.unwrap();
        con.1.write_all(b"\n").await.unwrap();
        con.1.flush().await.unwrap();
    }
}

impl Drop for FakeSidecar {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ruta_socket);
    }
}

#[tokio::test]
async fn ejecutar_emparejamiento_codigo_de_vinculacion_exitoso() {
    let mut sidecar = FakeSidecar::nuevo();
    let ruta_socket = sidecar.ruta_socket().clone();

    let codigos_capturados: Arc<Mutex<Vec<CodigoEmparejamiento>>> =
        Arc::new(Mutex::new(Vec::new()));
    let codigos_ref = Arc::clone(&codigos_capturados);

    let tarea = tokio::spawn(async move {
        ejecutar(
            &ruta_socket,
            "celula-test-1",
            "codigo_de_vinculacion",
            Duration::from_secs(5),
            move |c| {
                codigos_ref.lock().unwrap().push(c.clone());
            },
        )
        .await
    });

    sidecar.aceptar_y_saludar("celula-test-1").await;

    let linea_orden = sidecar.leer_linea().await;
    assert!(linea_orden.contains("\"tipo\":\"orden_emparejar\""));
    assert!(linea_orden.contains("\"metodo\":\"codigo_de_vinculacion\""));

    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"codigo_emparejamiento\",\"metodo\":\"codigo_de_vinculacion\",\"valor\":\"ABC1-2345\",\"expira_en_ms\":0}",
        )
        .await;

    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"acuse_emparejamiento\",\"resultado\":\"completado\",\"motivo\":\"\"}",
        )
        .await;

    let resultado = tarea.await.unwrap().expect("ejecutar debe tener éxito");
    assert_eq!(resultado, ResultadoEmparejamiento::Completado);

    let capturados = codigos_capturados.lock().unwrap();
    assert_eq!(capturados.len(), 1);
    assert_eq!(capturados[0].valor, "ABC1-2345");
    assert_eq!(capturados[0].expira_en_ms, 0);
}

#[tokio::test]
async fn ejecutar_emparejamiento_qr_rotativo_exitoso() {
    let mut sidecar = FakeSidecar::nuevo();
    let ruta_socket = sidecar.ruta_socket().clone();

    let codigos_capturados: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let codigos_ref = Arc::clone(&codigos_capturados);

    let tarea = tokio::spawn(async move {
        ejecutar(
            &ruta_socket,
            "celula-test-qr",
            "qr",
            Duration::from_secs(5),
            move |c| {
                codigos_ref.lock().unwrap().push(c.valor.clone());
            },
        )
        .await
    });

    sidecar.aceptar_y_saludar("celula-test-qr").await;

    let linea_orden = sidecar.leer_linea().await;
    assert!(linea_orden.contains("\"tipo\":\"orden_emparejar\""));
    assert!(linea_orden.contains("\"metodo\":\"qr\""));

    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"codigo_emparejamiento\",\"metodo\":\"qr\",\"valor\":\"qr_code_frame_1\",\"expira_en_ms\":1000}",
        )
        .await;
    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"codigo_emparejamiento\",\"metodo\":\"qr\",\"valor\":\"qr_code_frame_2\",\"expira_en_ms\":2000}",
        )
        .await;

    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"acuse_emparejamiento\",\"resultado\":\"completado\",\"motivo\":\"\"}",
        )
        .await;

    let resultado = tarea.await.unwrap().expect("ejecutar debe tener éxito");
    assert_eq!(resultado, ResultadoEmparejamiento::Completado);

    let capturados = codigos_capturados.lock().unwrap();
    assert_eq!(*capturados, vec!["qr_code_frame_1", "qr_code_frame_2"]);
}

#[tokio::test]
async fn ordenar_emparejamiento_ipc_expirado() {
    let mut sidecar = FakeSidecar::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();
    sidecar.aceptar_y_saludar("celula-1").await;

    let tarea_exp = tokio::spawn(async move {
        ordenar_emparejamiento(&adaptador, "qr", Duration::from_secs(5), |_| {}).await
    });
    let _ = sidecar.leer_linea().await;
    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"acuse_emparejamiento\",\"resultado\":\"expirado\",\"motivo\":\"\"}",
        )
        .await;
    let res_exp = tarea_exp.await.unwrap().unwrap();
    assert_eq!(res_exp, ResultadoEmparejamiento::Expirado);
}

#[tokio::test]
async fn ordenar_emparejamiento_ipc_fallido_con_motivo() {
    let mut sidecar = FakeSidecar::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();
    sidecar.aceptar_y_saludar("celula-1").await;

    let tarea_fallo = tokio::spawn(async move {
        ordenar_emparejamiento(
            &adaptador,
            "codigo_de_vinculacion",
            Duration::from_secs(5),
            |_| {},
        )
        .await
    });
    let _ = sidecar.leer_linea().await;
    sidecar
        .enviar_linea(
            "{\"version\":4,\"tipo\":\"acuse_emparejamiento\",\"resultado\":\"fallido\",\"motivo\":\"sesion cerrada por usuario remoto\"}",
        )
        .await;
    let res_fallo = tarea_fallo.await.unwrap().unwrap();
    assert_eq!(
        res_fallo,
        ResultadoEmparejamiento::Fallido {
            motivo: "sesion cerrada por usuario remoto".to_string(),
        }
    );
}

#[tokio::test]
async fn ejecutar_emparejamiento_timeout_sin_respuesta() {
    let mut sidecar = FakeSidecar::nuevo();
    let ruta_socket = sidecar.ruta_socket().clone();

    let tarea = tokio::spawn(async move {
        ejecutar(
            &ruta_socket,
            "celula-timeout",
            "codigo_de_vinculacion",
            Duration::from_millis(50),
            |_| {},
        )
        .await
    });

    sidecar.aceptar_y_saludar("celula-timeout").await;
    let _orden = sidecar.leer_linea().await;

    let res = tarea.await.unwrap();
    match res {
        Err(ErrorModoEmparejar::Canal(ErrorCanalWhatsmeow::EmparejamientoSinAcuse))
        | Err(ErrorModoEmparejar::ConexionNoEstablecida) => {}
        _ => panic!("se esperaba error de plazo agotado, obtenido: {res:?}"),
    }
}
