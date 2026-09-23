mod comun;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::{
    AdaptadorWhatsmeow, AsaDeSesion, InicioDeEmparejamiento, MetodoDeEmparejamiento,
};
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::mensajes::CodigoEmparejamiento;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::{CicloDeVidaSesion, EstadoSesion};

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

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
    sidecar.enviar_saludo(6, "celula-1").await;

    // Esperar notificación de cambio de estado
    while *receptor.borrow() != EstadoSesion::Activa {
        receptor.changed().await.unwrap();
    }
    assert_eq!(*receptor.borrow(), EstadoSesion::Activa);
}

// ---------------------------------------------------------------------------
// Pruebas de iniciar_emparejamiento_con (AC-5 / AC-16)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn iniciar_emparejamiento_con_codigo_de_vinculacion_envia_orden_y_devuelve_codigo() {
    // Camino feliz: iniciar_emparejamiento_con(CodigoDeVinculacion, 5 s) escribe orden_emparejar
    // con metodo "codigo_de_vinculacion" y version 6, y resuelve con InicioDeEmparejamiento::Codigo
    // que lleva el valor y expira_en_ms exactos.
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
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move {
        adaptador
            .iniciar_emparejamiento_con(
                MetodoDeEmparejamiento::CodigoDeVinculacion,
                Duration::from_secs(5),
            )
            .await
    });

    let orden = sidecar.leer_orden_emparejar().await;
    assert_eq!(orden.tipo, "orden_emparejar");
    assert_eq!(orden.metodo, "codigo_de_vinculacion");
    assert_eq!(orden.version, 6);

    let valor_fixture = "WXYZ-1234";
    let expira_fixture: i64 = 1700000100;
    sidecar
        .enviar_codigo_emparejamiento("codigo_de_vinculacion", valor_fixture, expira_fixture)
        .await;

    let res = tarea
        .await
        .unwrap()
        .expect("debe resolver con InicioDeEmparejamiento::Codigo");
    match res {
        InicioDeEmparejamiento::Codigo(codigo) => {
            assert_eq!(codigo.metodo, "codigo_de_vinculacion");
            assert_eq!(codigo.valor, valor_fixture);
            assert_eq!(codigo.expira_en_ms, expira_fixture);
        }
        InicioDeEmparejamiento::Acuse(_) => panic!("se esperaba Codigo, no Acuse"),
    }
}

#[tokio::test]
async fn iniciar_emparejamiento_con_mismo_resultado_a_traves_de_asa() {
    // El mismo camino feliz a través de AsaDeSesion tomada del adaptador: demuestra que el asa
    // comparte la implementación y que las rutas pueden usarlo sin un segundo adaptador.
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
    sidecar.enviar_saludo(6, "celula-1").await;

    let asa: AsaDeSesion = adaptador.asa_de_sesion("cell rebind");

    let tarea = tokio::spawn(async move {
        asa.iniciar_emparejamiento_con(
            MetodoDeEmparejamiento::CodigoDeVinculacion,
            Duration::from_secs(5),
        )
        .await
    });

    let orden = sidecar.leer_orden_emparejar().await;
    assert_eq!(orden.metodo, "codigo_de_vinculacion");
    assert_eq!(orden.version, 6);

    sidecar
        .enviar_codigo_emparejamiento("codigo_de_vinculacion", "ASA-TEST", 1700000200)
        .await;

    let res = tarea
        .await
        .unwrap()
        .expect("el asa debe resolver con Codigo");
    match res {
        InicioDeEmparejamiento::Codigo(codigo) => {
            assert_eq!(codigo.valor, "ASA-TEST");
            assert_eq!(codigo.expira_en_ms, 1700000200);
        }
        InicioDeEmparejamiento::Acuse(_) => panic!("se esperaba Codigo a través del asa"),
    }
}

#[tokio::test]
async fn iniciar_emparejamiento_con_acuse_primero_devuelve_acuse_y_no_bloquea_bucle() {
    // Camino expirado: el sidecar responde con acuse_emparejamiento expirado ANTES de cualquier
    // código. La llamada resuelve con InicioDeEmparejamiento::Acuse(resultado "expirado"). Después
    // se envía un código huérfano y un estado_sesion activa: el bucle de lectura no se bloquea y
    // el estado llega al receptor.
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    let mut receptor_estado = adaptador.suscribir_estado();
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move {
        adaptador
            .iniciar_emparejamiento_con(MetodoDeEmparejamiento::Qr, Duration::from_secs(5))
            .await
    });

    let _ = sidecar.leer_orden_emparejar().await;
    sidecar.enviar_acuse_emparejamiento("expirado", "").await;

    let res = tarea
        .await
        .unwrap()
        .expect("debe resolver con Acuse expirado");
    match res {
        InicioDeEmparejamiento::Acuse(acuse) => {
            assert_eq!(acuse.resultado, "expirado");
        }
        InicioDeEmparejamiento::Codigo(_) => panic!("se esperaba Acuse expirado"),
    }

    // Código huérfano posterior: no debe bloquear el bucle ni pánicar.
    sidecar
        .enviar_codigo_emparejamiento("qr", "qr-huerfano-post-expirado", 0)
        .await;

    // Estado de sesión posterior: debe llegar al receptor.
    sidecar.enviar_estado_sesion("activa", "", 0, 0).await;
    let limite = tokio::time::Instant::now() + Duration::from_secs(2);
    while *receptor_estado.borrow() != EstadoSesion::Activa && tokio::time::Instant::now() < limite
    {
        tokio::time::timeout(Duration::from_millis(100), receptor_estado.changed())
            .await
            .ok();
    }
    assert_eq!(
        *receptor_estado.borrow(),
        EstadoSesion::Activa,
        "el bucle de lectura no debe quedar bloqueado tras el retorno"
    );
}

#[tokio::test]
async fn iniciar_emparejamiento_con_plazo_agotado_devuelve_sin_acuse_y_no_bloquea() {
    // Sin ningún evento y un plazo corto (50 ms): resuelve Err(EmparejamientoSinAcuse). Un código
    // huérfano posterior no pánica ni bloquea el bucle.
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    let mut receptor_estado = adaptador.suscribir_estado();
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-1").await;

    let err = adaptador
        .iniciar_emparejamiento_con(MetodoDeEmparejamiento::Qr, Duration::from_millis(50))
        .await
        .unwrap_err();
    match err {
        ErrorCanalWhatsmeow::EmparejamientoSinAcuse => {}
        _ => panic!("se esperaba EmparejamientoSinAcuse, obtenido: {err:?}"),
    }

    // El plazo agotado debe limpiar el slot pendiente: si no lo limpia, el próximo código o
    // acuse huérfano se enruta al receptor ya soltado de este intento (falla el `send` en
    // silencio) en vez de descartarse con el aviso «huérfano recibido».
    assert!(
        !adaptador.emparejamiento_pendiente_ocupado().await,
        "el slot de emparejamiento pendiente debe quedar libre tras el plazo agotado"
    );

    // Código huérfano posterior: no debe pánicar.
    sidecar
        .enviar_codigo_emparejamiento("qr", "qr-huerfano-post-plazo", 0)
        .await;

    // El bucle sigue vivo: un estado_sesion llega al receptor.
    sidecar.enviar_estado_sesion("activa", "", 0, 0).await;
    let limite = tokio::time::Instant::now() + Duration::from_secs(2);
    while *receptor_estado.borrow() != EstadoSesion::Activa && tokio::time::Instant::now() < limite
    {
        tokio::time::timeout(Duration::from_millis(100), receptor_estado.changed())
            .await
            .ok();
    }
    assert_eq!(
        *receptor_estado.borrow(),
        EstadoSesion::Activa,
        "el bucle debe seguir vivo tras el plazo agotado"
    );
}

#[tokio::test]
async fn iniciar_emparejamiento_con_sin_conexion_devuelve_sin_conexion_sin_escribir() {
    // Sin conexión activa (adaptador no arrancado, escritor nunca establecido): devuelve
    // Err(SinConexion) sin escribir nada. La función comprueba el escritor antes de escribir.
    let ruta_fantasma = {
        let mut p = std::env::temp_dir();
        p.push(format!("hexcell-phantom-{}", std::process::id()));
        p
    };
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        &ruta_fantasma,
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(1), 2, Duration::from_millis(1)),
    );
    // No arrancamos el adaptador: el escritor_compartido sigue siendo None.
    let err = adaptador
        .iniciar_emparejamiento_con(MetodoDeEmparejamiento::Qr, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ErrorCanalWhatsmeow::SinConexion),
        "sin conexión debe devolver SinConexion sin escribir, se obtuvo: {err:?}"
    );
    let _ = std::fs::remove_file(&ruta_fantasma);
}

#[tokio::test]
async fn trait_iniciar_emparejamiento_envia_qr_y_mapea_codigo_a_codigo_qr() {
    // El trait CicloDeVidaSesion::iniciar_emparejamiento envía metodo "qr" y mapea un código
    // recibido a Emparejamiento::CodigoQr(valor).
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
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move { adaptador.iniciar_emparejamiento().await });

    let orden = sidecar.leer_orden_emparejar().await;
    assert_eq!(orden.metodo, "qr");

    sidecar
        .enviar_codigo_emparejamiento("qr", "cadena-qr-cruda", 1700000300)
        .await;

    let res = tarea
        .await
        .unwrap()
        .expect("el trait debe resolver con CodigoQr");
    assert_eq!(
        res,
        hexcell_core::canal::Emparejamiento::CodigoQr("cadena-qr-cruda".to_string())
    );
}

#[tokio::test]
async fn trait_iniciar_emparejamiento_con_acuse_devuelve_error_de_protocolo() {
    // El trait CicloDeVidaSesion::iniciar_emparejamiento con un acuse que llega antes que cualquier
    // código devuelve Err(ErrorDeProtocolo) con el motivo del acuse.
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
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move { adaptador.iniciar_emparejamiento().await });

    let _ = sidecar.leer_orden_emparejar().await;
    sidecar
        .enviar_acuse_emparejamiento("fallido", "canal: la sesión ya está emparejada")
        .await;

    let err = tarea
        .await
        .unwrap()
        .expect_err("el trait debe fallar con acuse");
    match err {
        ErrorCanalWhatsmeow::ErrorDeProtocolo(detalle) => {
            assert!(
                detalle.contains("canal: la sesión ya está emparejada"),
                "el motivo del acuse debe viajar en el error: {detalle}"
            );
        }
        otro => panic!("se esperaba ErrorDeProtocolo, obtenido: {otro:?}"),
    }
}

#[tokio::test]
async fn iniciar_emparejamiento_con_codigo_primero_luego_acuse_y_estado_actualiza_receptor() {
    // Después de que iniciar_emparejamiento_con devolvió el primer código, el sidecar envía un
    // segundo código (huérfano), un acuse terminal y un estado_sesion activa. El estado del
    // adaptador alcanza Activa en un plazo finito, probando que el bucle de lectura no está
    // bloqueado por el llamante retornado y que los eventos posteriores actualizan el receptor.
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    let mut receptor = adaptador.suscribir_estado();
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move {
        adaptador
            .iniciar_emparejamiento_con(
                MetodoDeEmparejamiento::CodigoDeVinculacion,
                Duration::from_secs(5),
            )
            .await
    });

    let _ = sidecar.leer_orden_emparejar().await;
    sidecar
        .enviar_codigo_emparejamiento("codigo_de_vinculacion", "PRIMER-COD", 1700000400)
        .await;

    let res = tarea
        .await
        .unwrap()
        .expect("debe resolver con el primer código");
    assert_eq!(
        match &res {
            InicioDeEmparejamiento::Codigo(c) => c.valor.as_str(),
            _ => panic!("se esperaba Codigo"),
        },
        "PRIMER-COD"
    );

    // Segundo código (huérfano), acuse terminal y estado activa: el bucle debe seguir vivo.
    sidecar
        .enviar_codigo_emparejamiento("codigo_de_vinculacion", "SEGUNDO-COD-HUERFANO", 0)
        .await;
    sidecar.enviar_acuse_emparejamiento("completado", "").await;
    sidecar.enviar_estado_sesion("activa", "", 0, 0).await;

    // El estado debe alcanzar Activa en un plazo finito.
    let limite = tokio::time::Instant::now() + Duration::from_secs(2);
    while *receptor.borrow() != EstadoSesion::Activa && tokio::time::Instant::now() < limite {
        tokio::time::timeout(Duration::from_millis(100), receptor.changed())
            .await
            .ok();
    }
    assert_eq!(
        *receptor.borrow(),
        EstadoSesion::Activa,
        "el estado debe actualizarse a Activa después del retorno"
    );
}
