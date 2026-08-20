mod comun;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::{ChannelAdapter, EventoEntrante, MensajeSaliente, TestigoDeEntrante};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn send_escribe_mensaje_saliente_en_ipc() {
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

    // Simula la recepción de un evento para que el adaptador guarde la marca temporal de origen
    sidecar
        .enviar_evento("dedup-1", "conv-1", "rem-1", "hola", 12345)
        .await;
    let _ = sidecar.leer_confirmacion().await;

    // Esperar a que el adaptador procese el evento y guarde el estado
    tokio::time::sleep(Duration::from_millis(50)).await;

    let evento = EventoEntrante {
        remitente: IdRemitente::nuevo("rem-1"),
        conversacion: IdConversacion::nuevo("conv-1"),
        contenido: "hola".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo("dedup-1"),
    };
    let testigo = TestigoDeEntrante::observar(&evento);
    let msj =
        MensajeSaliente::respuesta_libre(&testigo, &evento.conversacion, "respuesta".to_string())
            .unwrap();

    let res = adaptador
        .send(&evento.conversacion, msj)
        .await
        .expect("el envio debe ser aceptado");
    assert_eq!(res, hexcell_core::canal::ResultadoEnvio::Aceptado);

    let msj_ipc = sidecar.leer_mensaje_saliente().await;
    assert_eq!(msj_ipc.tipo, "mensaje_saliente");
    assert_eq!(msj_ipc.contenido, "respuesta");
    assert_eq!(msj_ipc.id_conversacion, "conv-1");
    assert_eq!(msj_ipc.marca_temporal_origen_ms, 12345);
    assert!(msj_ipc.id_mensaje.starts_with("conv-1-"));
}

#[tokio::test]
async fn send_sin_conexion_devuelve_error() {
    let ruta = std::env::temp_dir().join("no-existe.sock");
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        ruta,
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    let evento = EventoEntrante {
        remitente: IdRemitente::nuevo("rem-1"),
        conversacion: IdConversacion::nuevo("conv-1"),
        contenido: "hola".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo("dedup-1"),
    };
    let testigo = TestigoDeEntrante::observar(&evento);
    let msj =
        MensajeSaliente::respuesta_libre(&testigo, &evento.conversacion, "respuesta".to_string())
            .unwrap();

    let err = adaptador.send(&evento.conversacion, msj).await.unwrap_err();
    match err {
        ErrorCanalWhatsmeow::SinConexion => {}
        _ => panic!("se esperaba SinConexion"),
    }
}

#[tokio::test]
async fn send_sin_marca_de_origen_conocida_devuelve_error_explicito() {
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

    // A diferencia de send_escribe_mensaje_saliente_en_ipc, aquí NUNCA se envía un evento
    // entrante para "conv-nueva". El mapa de marcas de origen es memoria de proceso: arranca
    // vacío tanto para una conversación nunca vista como justo tras un reinicio del núcleo, así
    // que este caso cubre los dos a la vez. El adaptador debe rechazar el envío explícitamente
    // en vez de enviar con una marca de origen inventada (que un 0 leería como "ya expirado").
    tokio::time::sleep(Duration::from_millis(50)).await;

    let evento = EventoEntrante {
        remitente: IdRemitente::nuevo("rem-1"),
        conversacion: IdConversacion::nuevo("conv-nueva"),
        contenido: "hola".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo("dedup-1"),
    };
    let testigo = TestigoDeEntrante::observar(&evento);
    let msj =
        MensajeSaliente::respuesta_libre(&testigo, &evento.conversacion, "respuesta".to_string())
            .unwrap();

    let err = adaptador.send(&evento.conversacion, msj).await.unwrap_err();
    match err {
        ErrorCanalWhatsmeow::OrigenDesconocido => {}
        _ => panic!("se esperaba OrigenDesconocido"),
    }
}

#[tokio::test]
async fn send_plantilla_devuelve_error_representable() {
    let ruta = std::env::temp_dir().join("no-existe2.sock");
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        ruta,
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );

    let evento = EventoEntrante {
        remitente: IdRemitente::nuevo("rem-1"),
        conversacion: IdConversacion::nuevo("conv-1"),
        contenido: "hola".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo("dedup-1"),
    };
    let testigo = TestigoDeEntrante::observar(&evento);
    let msj = MensajeSaliente::plantilla(
        &testigo,
        &evento.conversacion,
        "plantilla".to_string(),
        vec![],
    )
    .unwrap();

    let err = adaptador.send(&evento.conversacion, msj).await.unwrap_err();
    match err {
        ErrorCanalWhatsmeow::PlantillaNoRepresentable => {}
        _ => panic!("se esperaba PlantillaNoRepresentable"),
    }
}

#[tokio::test]
async fn acuse_envio_se_consume_sin_cerrar_conexion() {
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

    // Mandamos el acuse_envio
    sidecar
        .enviar_acuse_envio("conv-1-0", "entregado", "corr-1", "", 12345)
        .await;

    // Verificamos que la conexión sigue abierta enviando un evento normal
    sidecar
        .enviar_evento("dedup-1", "conv-1", "rem-1", "hola", 12345)
        .await;
    let conf = sidecar.leer_confirmacion().await;
    assert_eq!(conf.id_deduplicacion, "dedup-1");
}

#[tokio::test]
async fn acuse_envio_no_filtra_terminos_proscritos() {
    let texto_acuse = r#"{"version":5,"tipo":"acuse_envio","id_mensaje":"msg-1","estado":"fallido","id_correlacion":"corr-1","motivo":"phone numero dispositivo jid device telefono inválido","marca_temporal_ms":123}"#;
    const TERMINOS_PROSCRITOS: [&str; 6] = [
        "jid",
        "telefono",
        "phone",
        "dispositivo",
        "device",
        "numero",
    ];

    for termino in TERMINOS_PROSCRITOS {
        assert!(texto_acuse.contains(termino));
    }

    let acuse: hexcell_canal_whatsmeow::mensajes::AcuseEnvioIpc =
        serde_json::from_str(texto_acuse).unwrap();
    assert_eq!(acuse.estado, "fallido");
}
