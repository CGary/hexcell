mod comun;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use comun::DirectorioTemporal;
use hexcell::respaldo::{ResultadoRespaldoSqlstore, ordenar_respaldo_sqlstore};
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
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
            "hexcell-fake-sidecar-{}-{}",
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
            "{{\"version\":5,\"tipo\":\"saludo\",\"emisor\":\"sidecar\",\"id_celula\":\"{id_celula}\"}}\n"
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
async fn ordenar_respaldo_sqlstore_ipc_exitoso() {
    let mut sidecar = FakeSidecar::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_y_saludar("celula-1").await;

    let destino_temp = DirectorioTemporal::nuevo("respaldo-sqlstore-ipc-exito");
    let destino_path = destino_temp.ruta().to_path_buf();

    let tarea_orden = tokio::spawn(async move {
        ordenar_respaldo_sqlstore(
            &adaptador,
            &destino_path,
            "ronda-ipc-1",
            Duration::from_secs(5),
        )
        .await
    });

    let orden_linea = sidecar.leer_linea().await;
    assert!(orden_linea.contains("\"tipo\":\"orden_respaldo_sqlstore\""));
    assert!(orden_linea.contains("\"identificador_de_ronda\":\"ronda-ipc-1\""));

    let ruta_copia_simulada = destino_temp.ruta().join("sqlstore.db");
    let acuse = format!(
        "{{\"version\":5,\"tipo\":\"acuse_respaldo_sqlstore\",\"identificador_de_ronda\":\"ronda-ipc-1\",\"resultado\":\"completado\",\"ruta_de_la_copia\":\"{}\",\"bytes\":4096,\"motivo\":\"\"}}",
        ruta_copia_simulada.to_string_lossy()
    );
    sidecar.enviar_linea(&acuse).await;

    let res = tarea_orden
        .await
        .unwrap()
        .expect("ordenar_respaldo_sqlstore debe tener exito");
    match res {
        ResultadoRespaldoSqlstore::Completado(copia) => {
            assert_eq!(copia.nombre_logico, "sqlstore.db");
            assert_eq!(copia.ruta, ruta_copia_simulada);
            assert_eq!(copia.bytes, 4096);
        }
        ResultadoRespaldoSqlstore::Fallido { motivo } => {
            panic!("se esperaba Completado, obtenido Fallido: {motivo}");
        }
    }
}

#[tokio::test]
async fn ordenar_respaldo_sqlstore_ipc_fallido() {
    let mut sidecar = FakeSidecar::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_y_saludar("celula-1").await;

    let destino_temp = DirectorioTemporal::nuevo("respaldo-sqlstore-ipc-fallo");
    let destino_path = destino_temp.ruta().to_path_buf();

    let tarea_orden = tokio::spawn(async move {
        ordenar_respaldo_sqlstore(
            &adaptador,
            &destino_path,
            "ronda-ipc-fallo",
            Duration::from_secs(5),
        )
        .await
    });

    let orden_linea = sidecar.leer_linea().await;
    assert!(orden_linea.contains("\"tipo\":\"orden_respaldo_sqlstore\""));
    assert!(orden_linea.contains("\"identificador_de_ronda\":\"ronda-ipc-fallo\""));

    let acuse = "{\"version\":5,\"tipo\":\"acuse_respaldo_sqlstore\",\"identificador_de_ronda\":\"ronda-ipc-fallo\",\"resultado\":\"fallido\",\"ruta_de_la_copia\":\"\",\"bytes\":0,\"motivo\":\"error al verificar integridad de la copia\"}";
    sidecar.enviar_linea(acuse).await;

    let res = tarea_orden
        .await
        .unwrap()
        .expect("ordenar_respaldo_sqlstore debe responder");
    match res {
        ResultadoRespaldoSqlstore::Completado(copia) => {
            panic!("se esperaba Fallido, obtenido Completado: {copia:?}");
        }
        ResultadoRespaldoSqlstore::Fallido { motivo } => {
            assert_eq!(motivo, "error al verificar integridad de la copia");
        }
    }
}
