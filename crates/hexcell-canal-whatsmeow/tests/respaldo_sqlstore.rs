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
    sidecar.enviar_saludo(5, "celula-1").await;

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
    sidecar.enviar_saludo(5, "celula-1").await;

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
    sidecar.enviar_saludo(5, "celula-1").await;

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
async fn ordenar_respaldo_identidad_completado_retorna_acuse() {
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
    sidecar.enviar_saludo(5, "celula-1").await;

    let tarea_orden = {
        let adaptador = adaptador;
        tokio::spawn(async move {
            adaptador
                .ordenar_respaldo_identidad("/tmp/backup_id", "ronda-id-1", Duration::from_secs(5))
                .await
        })
    };

    let orden_ipc = sidecar.leer_orden_respaldo_identidad().await;
    assert_eq!(orden_ipc.tipo, "orden_respaldo_identidad");
    assert_eq!(orden_ipc.orden, "respaldar_identidad");
    assert_eq!(orden_ipc.destino, "/tmp/backup_id");
    assert_eq!(orden_ipc.identificador_de_ronda, "ronda-id-1");

    sidecar
        .enviar_acuse_respaldo_identidad(
            "ronda-id-1",
            "completado",
            "/tmp/backup_id/identidad.db",
            512,
            "",
        )
        .await;

    let resultado = tarea_orden
        .await
        .unwrap()
        .expect("el respaldo de identidad debe completarse");
    assert_eq!(resultado.resultado, "completado");
    assert_eq!(resultado.identificador_de_ronda, "ronda-id-1");
    assert_eq!(resultado.ruta_de_la_copia, "/tmp/backup_id/identidad.db");
    assert_eq!(resultado.bytes, 512);
}

// Prueba clave de adr-0022: los dos acuses de la MISMA ronda (sqlstore e identidad) se resuelven
// cada uno contra su propio mapa de pendientes, sin colisionar por clave de ronda. Si el adaptador
// correlacionara ambos por un solo mapa keyeado solo por ronda, uno de los dos se perdería o se
// entregaría al oneshot equivocado y este test fallaría (LES-036 discriminación).
#[tokio::test]
async fn acuses_de_sqlstore_e_identidad_de_la_misma_ronda_no_colisionan() {
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
    sidecar.enviar_saludo(5, "celula-1").await;

    let adaptador = std::sync::Arc::new(adaptador);
    let a_sql = std::sync::Arc::clone(&adaptador);
    let tarea_sql = tokio::spawn(async move {
        a_sql
            .ordenar_respaldo_sqlstore("/tmp/dst", "ronda-compartida", Duration::from_secs(5))
            .await
    });
    let _ = sidecar.leer_orden_respaldo_sqlstore().await;

    let a_id = std::sync::Arc::clone(&adaptador);
    let tarea_id = tokio::spawn(async move {
        a_id.ordenar_respaldo_identidad("/tmp/dst", "ronda-compartida", Duration::from_secs(5))
            .await
    });
    let _ = sidecar.leer_orden_respaldo_identidad().await;

    // Los acuses llegan en orden inverso al de las órdenes, para que un supuesto mapa único no
    // pudiera "acertar" por simple orden de llegada.
    sidecar
        .enviar_acuse_respaldo_identidad(
            "ronda-compartida",
            "completado",
            "/tmp/dst/identidad.db",
            64,
            "",
        )
        .await;
    sidecar
        .enviar_acuse_respaldo_sqlstore(
            "ronda-compartida",
            "completado",
            "/tmp/dst/sqlstore.db",
            128,
            "",
        )
        .await;

    let res_sql = tarea_sql.await.unwrap().expect("acuse sqlstore");
    let res_id = tarea_id.await.unwrap().expect("acuse identidad");
    assert_eq!(res_sql.ruta_de_la_copia, "/tmp/dst/sqlstore.db");
    assert_eq!(res_sql.bytes, 128);
    assert_eq!(res_id.ruta_de_la_copia, "/tmp/dst/identidad.db");
    assert_eq!(res_id.bytes, 64);
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
    sidecar.enviar_saludo(5, "celula-1").await;

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
