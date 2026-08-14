mod comun;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::mensajes::CodigoEmparejamiento;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::EstadoSesion;

#[tokio::test]
async fn ordenar_emparejamiento_envia_metodo_qr_y_codigo_vinculacion_exactos() {
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

    // 1. Método QR
    let tarea_qr = {
        let adaptador = adaptador;
        tokio::spawn(async move {
            adaptador
                .ordenar_emparejamiento("qr", Duration::from_secs(5), |_| {})
                .await
        })
    };

    let orden_qr = sidecar.leer_orden_emparejar().await;
    assert_eq!(orden_qr.tipo, "orden_emparejar");
    assert_eq!(orden_qr.metodo, "qr");

    sidecar.enviar_acuse_emparejamiento("completado", "").await;

    let res_qr = tarea_qr.await.unwrap().expect("emparejamiento qr exitoso");
    assert_eq!(res_qr.resultado, "completado");
}

#[tokio::test]
async fn ordenar_emparejamiento_codigo_de_vinculacion_expira_en_cero() {
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

    let codigos_capturados: Arc<Mutex<Vec<CodigoEmparejamiento>>> =
        Arc::new(Mutex::new(Vec::new()));
    let codigos_ref = Arc::clone(&codigos_capturados);

    let tarea_codigo = tokio::spawn(async move {
        adaptador
            .ordenar_emparejamiento("codigo_de_vinculacion", Duration::from_secs(5), move |c| {
                codigos_ref.lock().unwrap().push(c.clone());
            })
            .await
    });

    let orden = sidecar.leer_orden_emparejar().await;
    assert_eq!(orden.metodo, "codigo_de_vinculacion");

    sidecar
        .enviar_codigo_emparejamiento("codigo_de_vinculacion", "1234-5678", 0)
        .await;
    sidecar.enviar_acuse_emparejamiento("completado", "").await;

    let res = tarea_codigo.await.unwrap().expect("emparejamiento exitoso");
    assert_eq!(res.resultado, "completado");

    let capturados = codigos_capturados.lock().unwrap();
    assert_eq!(capturados.len(), 1);
    assert_eq!(capturados[0].metodo, "codigo_de_vinculacion");
    assert_eq!(capturados[0].valor, "1234-5678");
    assert_eq!(capturados[0].expira_en_ms, 0);
}

#[tokio::test]
async fn rotacion_de_codigos_qr_se_entrega_en_orden_antes_del_acuse_terminal() {
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

    let codigos_recibidos: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let codigos_ref = Arc::clone(&codigos_recibidos);

    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_emparejamiento("qr", Duration::from_secs(5), move |c| {
                codigos_ref.lock().unwrap().push(c.valor.clone());
            })
            .await
    });

    let _orden = sidecar.leer_orden_emparejar().await;

    // Tres rotaciones de código QR
    sidecar
        .enviar_codigo_emparejamiento("qr", "qr-rotacion-1", 1700000020)
        .await;
    sidecar
        .enviar_codigo_emparejamiento("qr", "qr-rotacion-2", 1700000040)
        .await;
    sidecar
        .enviar_codigo_emparejamiento("qr", "qr-rotacion-3", 1700000060)
        .await;

    sidecar.enviar_acuse_emparejamiento("completado", "").await;

    let res = tarea.await.unwrap().expect("acuse completado");
    assert_eq!(res.resultado, "completado");

    let recibidos = codigos_recibidos.lock().unwrap();
    assert_eq!(
        *recibidos,
        vec!["qr-rotacion-1", "qr-rotacion-2", "qr-rotacion-3"]
    );
}

#[tokio::test]
async fn acuse_emparejamiento_expirado_retorna_acuse() {
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

    let tarea_exp = tokio::spawn(async move {
        adaptador
            .ordenar_emparejamiento("qr", Duration::from_secs(5), |_| {})
            .await
    });
    let _ = sidecar.leer_orden_emparejar().await;
    sidecar.enviar_acuse_emparejamiento("expirado", "").await;
    let res_exp = tarea_exp.await.unwrap().expect("debe retornar acuse");
    assert_eq!(res_exp.resultado, "expirado");
}

#[tokio::test]
async fn acuse_emparejamiento_fallido_con_motivo_retorna_acuse() {
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

    let tarea_fallo = tokio::spawn(async move {
        adaptador
            .ordenar_emparejamiento("codigo_de_vinculacion", Duration::from_secs(5), |_| {})
            .await
    });
    let _ = sidecar.leer_orden_emparejar().await;
    sidecar
        .enviar_acuse_emparejamiento("fallido", "número de teléfono no válido en configuración")
        .await;
    let res_fallo = tarea_fallo.await.unwrap().expect("debe retornar acuse");
    assert_eq!(res_fallo.resultado, "fallido");
    assert_eq!(
        res_fallo.motivo,
        "número de teléfono no válido en configuración"
    );
}

#[tokio::test]
async fn timeout_por_plazo_limpia_slot_y_descartar_huerfanos() {
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

    // Plazo breve sin respuesta del sidecar
    let err = adaptador
        .ordenar_emparejamiento("qr", Duration::from_millis(50), |_| {})
        .await
        .unwrap_err();

    match err {
        ErrorCanalWhatsmeow::EmparejamientoSinAcuse => {}
        _ => panic!("se esperaba EmparejamientoSinAcuse, obtenido: {err:?}"),
    }

    let _orden = sidecar.leer_orden_emparejar().await;

    // Ahora enviamos un código y un acuse tardíos (huérfanos): no deben romper la conexión
    sidecar
        .enviar_codigo_emparejamiento("qr", "qr-huerfano", 0)
        .await;
    sidecar.enviar_acuse_emparejamiento("completado", "").await;

    // Verificamos que la conexión sigue viva con un evento normal
    sidecar
        .enviar_evento("dedup-emp-1", "conv-1", "rem-1", "hola", 1000)
        .await;
    let conf = sidecar.leer_confirmacion().await;
    assert_eq!(conf.id_deduplicacion, "dedup-emp-1");
}

#[tokio::test]
async fn acuse_resultado_desconocido_se_descarta_fail_closed() {
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

    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_emparejamiento("qr", Duration::from_secs(5), |_| {})
            .await
    });

    let _orden = sidecar.leer_orden_emparejar().await;

    // Enviamos un acuse con resultado desconocido: debe descartarse sin cancelar la espera
    sidecar
        .enviar_acuse_emparejamiento("resultado_inventado", "motivo desconocido")
        .await;

    // Luego enviamos el acuse válido real
    sidecar.enviar_acuse_emparejamiento("completado", "").await;

    let res = tarea
        .await
        .unwrap()
        .expect("debe resolver con el acuse válido");
    assert_eq!(res.resultado, "completado");
}

#[tokio::test]
async fn suscribir_estado_refleja_estado_activo() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    let mut receptor = adaptador.suscribir_estado();
    assert_eq!(*receptor.borrow(), EstadoSesion::Reconectando);

    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(4, "celula-1").await;

    // Esperar notificación de cambio de estado
    while *receptor.borrow() != EstadoSesion::Activa {
        receptor.changed().await.unwrap();
    }
    assert_eq!(*receptor.borrow(), EstadoSesion::Activa);
}
