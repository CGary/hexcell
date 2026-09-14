//! Tests de aceptación de las siete condiciones de alerta (HEX-077-b).
//!
//! Cada test provoca una condición contra el sumidero simulado y verifica que produce exactamente
//! una notificación con el código esperado. Los tests de exactly-one comprueban que la segunda
//! observación de la misma condición no produce una segunda notificación.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hexcell::alertas::{
    CODIGO_BANEO_TEMPORAL, CODIGO_CAIDA_ACUSES, CODIGO_DESCARTES_GCRA, CODIGO_ENVIO_NO_SOLICITADO,
    CODIGO_SALDO_AGOTADO, CODIGO_SESION_DESVINCULADA, CODIGO_SIN_RECONECTAR, EmisorDeAlertas,
    EvaluadorDeAlertas, UmbralesDeAlerta,
};
use hexcell::metricas::InstantaneaDeMetricas;
use hexcell::notificacion::SumideroDeCelula;
use hexcell_core::canal::EstadoSesion;
use hexcell_core::identidad::IdConversacion;

fn umbrales_prueba() -> UmbralesDeAlerta {
    UmbralesDeAlerta {
        ventana_reconexion: Duration::from_secs(300),
        suelo_balance_disponible: 0,
        limite_tasa_descartes: 0.5,
        limite_caida_ratio_acuses: 0.5,
        minimo_envios_para_evaluar_acuse: 5,
    }
}

fn instantanea_base() -> InstantaneaDeMetricas {
    InstantaneaDeMetricas {
        admitidos: 100,
        descartados_admision: 10,
        descartados_concurrencia: 0,
        en_vuelo: 0,
        disponible: 500,
        reservado: 0,
        desviacion: 0,
    }
}

// ---------------------------------------------------------------------------
// AC-2: Baneo temporal detectado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac2_baneo_temporal_produce_exactamente_una_notificacion_con_expiracion() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());
    let expira = UNIX_EPOCH + Duration::from_secs(1_735_689_600);
    let ahora = SystemTime::now();

    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Pausada, Some(expira), ahora)
        .await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
    let notif = &simulado.notificaciones()[0];
    assert_eq!(notif.codigo.como_str(), CODIGO_BANEO_TEMPORAL);
    assert!(notif.datos.iter().any(|d| d.clave == "expira_en"));
}

#[tokio::test]
async fn ac2_baneo_temporal_exactly_one_la_segunda_observacion_no_emite() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());
    let expira = UNIX_EPOCH + Duration::from_secs(1_735_689_600);
    let ahora = SystemTime::now();

    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Pausada, Some(expira), ahora)
        .await;
    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Pausada, Some(expira), ahora)
        .await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(
        simulado.cantidad(),
        1,
        "exactly-one: la segunda observación no debe emitir"
    );
}

// ---------------------------------------------------------------------------
// AC-3: Sesión desvinculada
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac3_sesion_desvinculada_produce_exactamente_una_notificacion() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Desvinculada, None, SystemTime::now())
        .await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
    assert_eq!(
        simulado.notificaciones()[0].codigo.como_str(),
        CODIGO_SESION_DESVINCULADA
    );
}

#[tokio::test]
async fn ac3_sesion_desvinculada_exactly_one() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Desvinculada, None, SystemTime::now())
        .await;
    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Desvinculada, None, SystemTime::now())
        .await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
}

// ---------------------------------------------------------------------------
// AC-4: Sidecar sin reconectar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac4_sin_reconectar_debajo_de_la_ventana_no_emite() {
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());
    let ahora = SystemTime::now();

    let notifs = evaluador.evaluar_estado_de_sesion(EstadoSesion::Reconectando, None, ahora);
    assert!(notifs.is_empty(), "por debajo de la ventana no debe emitir");
}

#[tokio::test]
async fn ac4_sin_reconectar_por_encima_de_la_ventana_emite_una_vez() {
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());
    let inicio = SystemTime::now();
    let despues = inicio + Duration::from_secs(301);

    evaluador.evaluar_estado_de_sesion(EstadoSesion::Reconectando, None, inicio);
    let notifs = evaluador.evaluar_estado_de_sesion(EstadoSesion::Reconectando, None, despues);
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0].codigo.como_str(), CODIGO_SIN_RECONECTAR);

    let notifs2 = evaluador.evaluar_estado_de_sesion(EstadoSesion::Reconectando, None, despues);
    assert!(
        notifs2.is_empty(),
        "exactly-one: la segunda evaluación no debe emitir"
    );
}

// ---------------------------------------------------------------------------
// AC-6: Saldo LLM agotado o modo degradado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac6_saldo_agotado_produce_exactamente_una_notificacion() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let mut instantanea = instantanea_base();
    instantanea.disponible = 0;

    emisor.evaluar_y_emitir_instantanea(&instantanea, 0).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
    assert_eq!(
        simulado.notificaciones()[0].codigo.como_str(),
        CODIGO_SALDO_AGOTADO
    );
}

#[tokio::test]
async fn ac6_saldo_agotado_exactly_one() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let mut instantanea = instantanea_base();
    instantanea.disponible = 0;

    emisor.evaluar_y_emitir_instantanea(&instantanea, 0).await;
    emisor.evaluar_y_emitir_instantanea(&instantanea, 0).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
}

// ---------------------------------------------------------------------------
// AC-7: Tasa anómala de descartes GCRA
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac7_tasa_descartes_anomala_produce_exactamente_una_notificacion() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let mut instantanea = instantanea_base();
    instantanea.admitidos = 10;
    instantanea.descartados_admision = 20;

    emisor.evaluar_y_emitir_instantanea(&instantanea, 0).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
    assert_eq!(
        simulado.notificaciones()[0].codigo.como_str(),
        CODIGO_DESCARTES_GCRA
    );
}

#[tokio::test]
async fn ac7_tasa_descartes_normal_no_emite() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let instantanea = instantanea_base();
    emisor.evaluar_y_emitir_instantanea(&instantanea, 0).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 0);
}

// ---------------------------------------------------------------------------
// AC-8: Descarte de envío no solicitado (proxy: rechazos_de_construccion)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac8_rechazos_de_construccion_delta_produce_exactamente_una_notificacion() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let instantanea = instantanea_base();
    emisor.evaluar_y_emitir_instantanea(&instantanea, 5).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
    assert_eq!(
        simulado.notificaciones()[0].codigo.como_str(),
        CODIGO_ENVIO_NO_SOLICITADO
    );
}

#[tokio::test]
async fn ac8_sin_delta_no_emite() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let instantanea = instantanea_base();
    emisor.evaluar_y_emitir_instantanea(&instantanea, 0).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 0);
}

// ---------------------------------------------------------------------------
// AC-9: Caída anómala del ratio de acuses por contacto
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac9_caída_ratio_por_contacto_produce_notificacion_con_id_conversacion() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    let conv_afectada = IdConversacion::nuevo("conv-afectada");
    let conv_sana_1 = IdConversacion::nuevo("conv-sana-1");
    let conv_sana_2 = IdConversacion::nuevo("conv-sana-2");

    let contadores = vec![
        (conv_afectada.clone(), 10, 1),
        (conv_sana_1.clone(), 10, 9),
        (conv_sana_2.clone(), 10, 8),
    ];

    emisor.evaluar_y_emitir_acuses(&contadores).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(
        simulado.cantidad(),
        1,
        "solo el contacto afectado debe emitir"
    );
    let notif = &simulado.notificaciones()[0];
    assert_eq!(notif.codigo.como_str(), CODIGO_CAIDA_ACUSES);
    assert!(
        notif
            .datos
            .iter()
            .any(|d| d.clave == "conversacion_afectada")
    );
}

// ---------------------------------------------------------------------------
// AC-10: El cómputo agregado NO basta
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac10_el_agregado_no_detecta_la_caida_por_contacto() {
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());

    let conv_afectada = IdConversacion::nuevo("conv-afectada");
    let conv_sana_1 = IdConversacion::nuevo("conv-sana-1");
    let conv_sana_2 = IdConversacion::nuevo("conv-sana-2");

    let contadores = vec![
        (conv_afectada.clone(), 10, 1),
        (conv_sana_1.clone(), 10, 9),
        (conv_sana_2.clone(), 10, 8),
    ];

    let notifs = evaluador.evaluar_acuses_por_contacto(&contadores);
    assert_eq!(
        notifs.len(),
        1,
        "la evaluación por contacto detecta la caída"
    );

    let total_enviados: u64 = contadores.iter().map(|(_, e, _)| e).sum();
    let total_acusados: u64 = contadores.iter().map(|(_, _, a)| a).sum();
    let ratio_agregado = total_acusados as f64 / total_enviados as f64;
    assert!(
        ratio_agregado >= 0.5,
        "el ratio agregado ({ratio_agregado}) está por encima del umbral, \
         demostrando que el agregado no detectaría la caída"
    );
}

// ---------------------------------------------------------------------------
// AC-14: Ningún artifact menciona reportes de usuarios
// ---------------------------------------------------------------------------

#[test]
fn ac14_ningun_codigo_de_alerta_menciona_reportes_de_usuarios() {
    let codigos = [
        CODIGO_BANEO_TEMPORAL,
        CODIGO_SESION_DESVINCULADA,
        CODIGO_SIN_RECONECTAR,
        CODIGO_SALDO_AGOTADO,
        CODIGO_DESCARTES_GCRA,
        CODIGO_ENVIO_NO_SOLICITADO,
        CODIGO_CAIDA_ACUSES,
    ];
    let terminos_proscritos = ["report", "reporte", "usuario", "user", "denuncia"];
    for codigo in &codigos {
        for termino in &terminos_proscritos {
            assert!(
                !codigo.to_lowercase().contains(termino),
                "el código '{codigo}' no debe mencionar '{termino}'"
            );
        }
    }
}
