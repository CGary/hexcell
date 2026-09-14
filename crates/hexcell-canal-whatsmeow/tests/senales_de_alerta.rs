//! Tests de las señales de alerta que el adaptador de whatsmeow eleva desde el cable IPC.
//!
//! AC-2: la expiración del baneo temporal (`expira_en_ms`) se publica por un watch separado.
//! AC-9: los contadores de acuse por conversación se alimentan desde `send()` y `AcuseEnvio`.

mod comun;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;

#[tokio::test]
async fn la_expiracion_del_baneo_se_publica_por_el_watch_separado() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-alertas",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-alertas").await;

    let mut receptor_expiracion = adaptador.suscribir_expiracion_de_baneo();

    // Sin baneo: la expiración es None.
    assert!(receptor_expiracion.borrow().is_none());

    // Baneo temporal con expiración.
    let expira_ms: i64 = 1_735_689_600_000;
    sidecar
        .enviar_estado_sesion("pausada", "temp_ban", 403, expira_ms)
        .await;
    receptor_expiracion.changed().await.unwrap();
    let expira = *receptor_expiracion.borrow();
    assert!(
        expira.is_some(),
        "la expiración debe publicarse al entrar en Pausada"
    );
    let esperado = UNIX_EPOCH + Duration::from_millis(expira_ms as u64);
    assert_eq!(expira.unwrap(), esperado);

    // Al salir del baneo (vuelta a Activa), la expiración se limpia.
    sidecar.enviar_estado_sesion("activa", "", 0, 0).await;
    receptor_expiracion.changed().await.unwrap();
    assert!(
        receptor_expiracion.borrow().is_none(),
        "la expiración se limpia al salir de Pausada"
    );
}

#[tokio::test]
async fn los_contadores_de_acuse_se_alimentan_desde_send_y_acuse_envio() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-acuses",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-acuses").await;

    // Primero, un evento entrante para crear la conversación y su marca de origen.
    sidecar
        .enviar_evento("dedup-1", "conv-acuse-1", "rem-1", "hola", 1_000)
        .await;
    let _evento = receptor_eventos.recv().await.unwrap();
    let _ = sidecar.leer_confirmacion().await;

    // Ahora enviar un mensaje saliente.
    use hexcell_core::canal::{ChannelAdapter, EventoEntrante, MensajeSaliente, TestigoDeEntrante};
    use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

    let evento = EventoEntrante {
        remitente: IdRemitente::nuevo("rem-1"),
        conversacion: IdConversacion::nuevo("conv-acuse-1"),
        contenido: "hola".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo("dedup-1"),
    };
    let testigo = TestigoDeEntrante::observar(&evento);
    let mensaje = MensajeSaliente::respuesta_libre(
        &testigo,
        &IdConversacion::nuevo("conv-acuse-1"),
        "respuesta".to_string(),
    )
    .unwrap();

    let resultado = adaptador
        .send(&IdConversacion::nuevo("conv-acuse-1"), mensaje)
        .await;
    assert!(resultado.is_ok());

    // Leer el mensaje saliente para obtener el id_mensaje.
    let saliente = sidecar.leer_mensaje_saliente().await;
    let id_mensaje = saliente.id_mensaje.clone();

    // Verificar que el contador de enviados se incrementó.
    let contadores = adaptador.contadores_de_acuse().instantanea().await;
    let conv = contadores
        .iter()
        .find(|(id, _, _)| id.como_str() == "conv-acuse-1");
    assert!(
        conv.is_some(),
        "la conversación debe estar en los contadores"
    );
    let (_, enviados, acusados) = conv.unwrap();
    assert_eq!(*enviados, 1, "un envío registrado");
    assert_eq!(*acusados, 0, "ningún acuse todavía");

    // Enviar un acuse de entrega.
    sidecar
        .enviar_acuse_envio(&id_mensaje, "entregado", "corr-1", "", 2_000)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let contadores = adaptador.contadores_de_acuse().instantanea().await;
    let conv = contadores
        .iter()
        .find(|(id, _, _)| id.como_str() == "conv-acuse-1");
    let (_, enviados, acusados) = conv.unwrap();
    assert_eq!(*enviados, 1);
    assert_eq!(*acusados, 1, "un acuse registrado");
}

#[tokio::test]
async fn los_contadores_de_acuse_usan_id_conversacion_nunca_jid() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-no-jid",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-no-jid").await;

    // El evento entrante lleva un id_conversacion opaco (no un JID).
    sidecar
        .enviar_evento("dedup-jid", "conv-opaca-123", "rem-1", "hola", 1_000)
        .await;
    let _evento = receptor_eventos.recv().await.unwrap();
    let _ = sidecar.leer_confirmacion().await;

    let contadores = adaptador.contadores_de_acuse().instantanea().await;
    for (id, _, _) in &contadores {
        assert!(
            !id.como_str().contains("@"),
            "el identificador de conversación nunca debe ser un JID: {}",
            id.como_str()
        );
    }
}
