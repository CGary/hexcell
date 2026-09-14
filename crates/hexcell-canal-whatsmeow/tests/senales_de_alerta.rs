//! Tests de las señales de alerta que el adaptador de whatsmeow eleva desde el cable IPC.
//!
//! AC-2: la expiración del baneo temporal (`expira_en_ms`) viaja **junto** al estado de sesión, en
//! un único envío, para que ningún consumidor pueda observar `Pausada` sin su fecha.
//! AC-9: los contadores de acuse por conversación se alimentan desde `send()` y `AcuseEnvio`,
//! contando solo los acuses de entrega y nunca el `enviado` que el sidecar emite en cada envío.

mod comun;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::{
    ChannelAdapter, EstadoSesion, EventoEntrante, MensajeSaliente, TestigoDeEntrante,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

fn retroceso_de_prueba() -> Retroceso {
    Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10))
}

/// Construye un `MensajeSaliente` legítimo para una conversación, con su testigo de entrante.
fn saliente_para(id_conversacion: &str, id_deduplicacion: &str) -> MensajeSaliente {
    let evento = EventoEntrante {
        remitente: IdRemitente::nuevo("rem-1"),
        conversacion: IdConversacion::nuevo(id_conversacion),
        contenido: "hola".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo(id_deduplicacion),
    };
    let testigo = TestigoDeEntrante::observar(&evento);
    MensajeSaliente::respuesta_libre(
        &testigo,
        &IdConversacion::nuevo(id_conversacion),
        "respuesta".to_string(),
    )
    .unwrap()
}

#[tokio::test]
async fn la_expiracion_del_baneo_viaja_junto_al_estado_en_un_unico_envio() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-alertas",
        8,
        retroceso_de_prueba(),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-alertas").await;

    let mut receptor = adaptador.suscribir_estado_con_expiracion();

    // Baneo temporal con expiración declarada.
    let expira_ms: i64 = 1_735_689_600_000;
    sidecar
        .enviar_estado_sesion("pausada", "temp_ban", 403, expira_ms)
        .await;

    // Consumir cambios hasta ver el estado Pausada. Lo que se afirma es que, en el MISMO valor
    // observado, el estado y la expiración llegan juntos: no existe ninguna observación de
    // `Pausada` con expiración ausente. Si el adaptador volviera a publicarlos por dos canales
    // independientes, esta afirmación podría fallar bajo entrelazado real.
    loop {
        receptor.changed().await.unwrap();
        let (estado, expira) = *receptor.borrow();
        if estado == EstadoSesion::Pausada {
            assert_eq!(
                expira,
                Some(UNIX_EPOCH + Duration::from_millis(expira_ms as u64)),
                "el estado Pausada nunca debe observarse sin su fecha de expiración"
            );
            break;
        }
    }

    // Al salir del baneo (vuelta a Activa), la expiración se limpia en el mismo envío.
    sidecar.enviar_estado_sesion("activa", "", 0, 0).await;
    loop {
        receptor.changed().await.unwrap();
        let (estado, expira) = *receptor.borrow();
        if estado == EstadoSesion::Activa {
            assert!(
                expira.is_none(),
                "la expiración se limpia al salir de Pausada"
            );
            break;
        }
    }
}

#[tokio::test]
async fn el_acuse_enviado_no_cuenta_y_solo_la_entrega_mueve_el_contador() {
    // Secuencia REAL del sidecar: `enviado` en cada envío aceptado por el servidor, y solo después
    // `entregado` por la ruta de Receipt. Contar `enviado` igualaría acusados a enviados para todo
    // mensaje que sale, el ratio quedaría clavado en 1.0 y AC-9 sería inalcanzable en producción.
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-acuses",
        8,
        retroceso_de_prueba(),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-acuses").await;

    sidecar
        .enviar_evento("dedup-1", "conv-acuse-1", "rem-1", "hola", 1_000)
        .await;
    let _evento = receptor_eventos.recv().await.unwrap();
    let _ = sidecar.leer_confirmacion().await;

    let resultado = adaptador
        .send(
            &IdConversacion::nuevo("conv-acuse-1"),
            saliente_para("conv-acuse-1", "dedup-1"),
        )
        .await;
    assert!(resultado.is_ok());

    let saliente = sidecar.leer_mensaje_saliente().await;
    let id_mensaje = saliente.id_mensaje.clone();

    let leer = || async {
        adaptador
            .contadores_de_acuse()
            .instantanea()
            .await
            .into_iter()
            .find(|(id, _, _)| id.como_str() == "conv-acuse-1")
            .expect("la conversación debe estar en los contadores")
    };

    let (_, enviados, acusados) = leer().await;
    assert_eq!(enviados, 1, "un envío registrado");
    assert_eq!(acusados, 0, "ningún acuse todavía");

    // Primer acuse del sidecar: `enviado`. NO cuenta como acuse.
    sidecar
        .enviar_acuse_envio(&id_mensaje, "enviado", "corr-1", "", 2_000)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (_, enviados, acusados) = leer().await;
    assert_eq!(enviados, 1);
    assert_eq!(
        acusados, 0,
        "el acuse 'enviado' es salida al servidor, no entrega: no debe contar"
    );

    // Segundo acuse: `entregado`, por la ruta de Receipt. Ahora sí cuenta.
    sidecar
        .enviar_acuse_envio(&id_mensaje, "entregado", "corr-1", "", 3_000)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (_, enviados, acusados) = leer().await;
    assert_eq!(enviados, 1);
    assert_eq!(acusados, 1, "la entrega sí cuenta como acuse");

    // Un `leido` posterior sobre el mismo mensaje no vuelve a contar.
    sidecar
        .enviar_acuse_envio(&id_mensaje, "leido", "corr-1", "", 4_000)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (_, _, acusados) = leer().await;
    assert_eq!(acusados, 1, "un mismo mensaje no se acusa dos veces");
}

#[tokio::test]
async fn un_contacto_que_deja_de_acusar_hace_caer_su_ratio() {
    // La prueba de que AC-9 es alcanzable en producción: con la secuencia real del sidecar
    // (`enviado` en cada envío, `entregado` solo mientras el contacto acusa), un contacto que deja
    // de entregar ve caer su ratio por debajo de 0,5. Si `enviado` volviera a contarse, el ratio
    // se quedaría en 1,0 y esta afirmación se pondría roja.
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-caida",
        8,
        retroceso_de_prueba(),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-caida").await;

    sidecar
        .enviar_evento("dedup-caida", "conv-bloqueada", "rem-1", "hola", 1_000)
        .await;
    let _evento = receptor_eventos.recv().await.unwrap();
    let _ = sidecar.leer_confirmacion().await;

    // Diez envíos: el sidecar acusa `enviado` en los diez, pero solo entrega los dos primeros.
    for i in 0..10_u32 {
        adaptador
            .send(
                &IdConversacion::nuevo("conv-bloqueada"),
                saliente_para("conv-bloqueada", "dedup-caida"),
            )
            .await
            .unwrap();
        let saliente = sidecar.leer_mensaje_saliente().await;
        sidecar
            .enviar_acuse_envio(&saliente.id_mensaje, "enviado", "corr", "", 2_000)
            .await;
        if i < 2 {
            sidecar
                .enviar_acuse_envio(&saliente.id_mensaje, "entregado", "corr", "", 3_000)
                .await;
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (_, enviados, acusados) = adaptador
        .contadores_de_acuse()
        .instantanea()
        .await
        .into_iter()
        .find(|(id, _, _)| id.como_str() == "conv-bloqueada")
        .expect("la conversación debe estar en los contadores");

    assert_eq!(enviados, 10);
    assert_eq!(acusados, 2, "solo dos entregas reales");
    let ratio = acusados as f64 / enviados as f64;
    assert!(
        ratio < 0.5,
        "el ratio del contacto que dejó de acusar debe caer ({ratio}); \
         si el acuse 'enviado' contara, quedaría clavado en 1,0"
    );
}

#[tokio::test]
async fn los_contadores_de_acuse_usan_id_conversacion_nunca_jid() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-no-jid",
        8,
        retroceso_de_prueba(),
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

    // Los contadores solo se pueblan al enviar: sin este `send()` la instantánea estaría vacía y
    // el bucle de afirmación no se ejecutaría ni una vez, dejando la guarda vacua.
    adaptador
        .send(
            &IdConversacion::nuevo("conv-opaca-123"),
            saliente_para("conv-opaca-123", "dedup-jid"),
        )
        .await
        .unwrap();
    let saliente = sidecar.leer_mensaje_saliente().await;
    sidecar
        .enviar_acuse_envio(&saliente.id_mensaje, "entregado", "corr", "", 2_000)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let contadores = adaptador.contadores_de_acuse().instantanea().await;
    assert!(
        !contadores.is_empty(),
        "los contadores deben estar poblados: si no, esta guarda sería vacua"
    );
    assert!(
        contadores
            .iter()
            .any(|(id, _, _)| id.como_str() == "conv-opaca-123"),
        "la conversación enviada debe aparecer en los contadores"
    );
    for (id, _, _) in &contadores {
        assert!(
            !id.como_str().contains("@"),
            "el identificador de conversación nunca debe ser un JID: {}",
            id.como_str()
        );
    }
}
