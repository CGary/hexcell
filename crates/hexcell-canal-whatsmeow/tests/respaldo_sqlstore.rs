mod comun;

use std::time::Duration;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;

#[tokio::test]
async fn ordenar_respaldo_sqlstore_completado_retorna_acuse() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(4, "celula-1").await;

    // Disparamos la orden en una tarea concurrente
    let tarea_orden = {
        let adaptador = adaptador;
        tokio::spawn(async move {
            adaptador
                .ordenar_respaldo_sqlstore(
                    "/tmp/backup_dest",
                    "ronda-exito-1",
                    Duration::from_secs(5),
                )
                .await
        })
    };

    let orden_ipc = sidecar.leer_orden_respaldo_sqlstore().await;
    assert_eq!(orden_ipc.tipo, "orden_respaldo_sqlstore");
    assert_eq!(orden_ipc.orden, "respaldar_sqlstore");
    assert_eq!(orden_ipc.destino, "/tmp/backup_dest");
    assert_eq!(orden_ipc.identificador_de_ronda, "ronda-exito-1");

    sidecar
        .enviar_acuse_respaldo_sqlstore(
            "ronda-exito-1",
            "completado",
            "/tmp/backup_dest/sqlstore.db",
            2048,
            "",
        )
        .await;

    let resultado = tarea_orden
        .await
        .unwrap()
        .expect("el respaldo debe completarse");
    assert_eq!(resultado.resultado, "completado");
    assert_eq!(resultado.identificador_de_ronda, "ronda-exito-1");
    assert_eq!(resultado.ruta_de_la_copia, "/tmp/backup_dest/sqlstore.db");
    assert_eq!(resultado.bytes, 2048);
    assert_eq!(resultado.motivo, "");
}

#[tokio::test]
async fn ordenar_respaldo_sqlstore_fallido_retorna_acuse_con_motivo() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(4, "celula-1").await;

    let tarea_orden = {
        let adaptador = adaptador;
        tokio::spawn(async move {
            adaptador
                .ordenar_respaldo_sqlstore(
                    "/tmp/backup_invalido",
                    "ronda-fallo-1",
                    Duration::from_secs(5),
                )
                .await
        })
    };

    let orden_ipc = sidecar.leer_orden_respaldo_sqlstore().await;
    assert_eq!(orden_ipc.identificador_de_ronda, "ronda-fallo-1");

    sidecar
        .enviar_acuse_respaldo_sqlstore(
            "ronda-fallo-1",
            "fallido",
            "",
            0,
            "directorio de destino no existe",
        )
        .await;

    let resultado = tarea_orden
        .await
        .unwrap()
        .expect("el acuse debe devolverse");
    assert_eq!(resultado.resultado, "fallido");
    assert_eq!(resultado.identificador_de_ronda, "ronda-fallo-1");
    assert_eq!(resultado.ruta_de_la_copia, "");
    assert_eq!(resultado.bytes, 0);
    assert_eq!(resultado.motivo, "directorio de destino no existe");
}

#[tokio::test]
async fn acuse_respaldo_huerfano_no_cierra_conexion() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(4, "celula-1").await;

    // Enviamos un acuse con identificador de ronda no registrado
    sidecar
        .enviar_acuse_respaldo_sqlstore(
            "ronda-huerfana-desconocida",
            "completado",
            "/tmp/sqlstore.db",
            1024,
            "",
        )
        .await;

    // Verificamos que la conexión sigue abierta enviando un evento normal
    sidecar
        .enviar_evento("dedup-respaldo-1", "conv-1", "rem-1", "hola", 12345)
        .await;
    let conf = sidecar.leer_confirmacion().await;
    assert_eq!(conf.id_deduplicacion, "dedup-respaldo-1");
}

#[tokio::test]
async fn ordenar_respaldo_sqlstore_timeout_devuelve_error() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(4, "celula-1").await;

    // Plazo muy corto y el sidecar nunca responde
    let err = adaptador
        .ordenar_respaldo_sqlstore("/tmp/dest", "ronda-timeout", Duration::from_millis(50))
        .await
        .unwrap_err();

    match err {
        ErrorCanalWhatsmeow::RespaldoSinAcuse => {}
        _ => panic!("se esperaba RespaldoSinAcuse, obtenido: {err:?}"),
    }
}
