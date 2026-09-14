//! Tests de aceptación de las siete condiciones de alerta (HEX-077-b).
//!
//! Cada test provoca una condición contra el sumidero simulado y verifica que produce exactamente
//! una notificación con el código esperado. Los tests de «exactamente una» comprueban que la
//! segunda observación de la misma condición no produce una segunda notificación.
//!
//! Las afirmaciones de AC-9 y AC-10 se expresan como el predicado [`cumple_criterio_ac9`], que se
//! afirma en **verde** sobre el evaluador real y en **rojo** sobre tres mutantes. Una guarda que
//! nadie vio ponerse roja no es todavía una guarda: si alguien debilitara el criterio a «la clave
//! existe», el mutante «denunciar siempre el primer contacto» pasaría y el test de mutación se
//! pondría rojo.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hexcell::alertas::{
    CODIGO_BANEO_TEMPORAL, CODIGO_CAIDA_ACUSES, CODIGO_DESCARTES_GCRA, CODIGO_ENVIO_NO_SOLICITADO,
    CODIGO_SALDO_AGOTADO, CODIGO_SESION_DESVINCULADA, CODIGO_SIN_RECONECTAR,
    EXPIRACION_DESCONOCIDA, EmisorDeAlertas, EvaluadorDeAlertas, UmbralesDeAlerta,
};
use hexcell::metricas::InstantaneaDeMetricas;
use hexcell::notificacion::SumideroDeCelula;
use hexcell_core::canal::EstadoSesion;
use hexcell_core::identidad::IdConversacion;
use hexcell_core::notificacion::{CodigoDeNotificacion, Notificacion, ValorDeDato};

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
    // El VALOR, no solo la presencia de la clave: una mutación que llevara `ahora` en lugar de la
    // expiración real pasaría una afirmación de mera presencia.
    let dato = notif
        .datos
        .iter()
        .find(|d| d.clave == "expira_en")
        .expect("la alerta de baneo lleva siempre la clave expira_en");
    assert_eq!(
        dato.valor,
        ValorDeDato::Instante(expira),
        "la alerta debe llevar la expiración inyectada, no otro instante"
    );
    assert_ne!(
        dato.valor,
        ValorDeDato::Instante(ahora),
        "llevar el instante actual en lugar de la expiración es precisamente la mutación a atrapar"
    );
}

#[tokio::test]
async fn ac2_baneo_sin_expiracion_declarada_emite_la_clave_marcada_como_desconocida() {
    // Invariante 1 de la especificación: la alerta de baneo lleva SIEMPRE el dato `expira_en`.
    // El adaptador publica estado y expiración en un único envío, así que no hay carrera que
    // pueda perderla; el único caso en que falta es que el sidecar no la declare, y entonces la
    // clave se emite marcada como desconocida en vez de desaparecer del payload.
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());

    emisor
        .evaluar_y_emitir_estado(EstadoSesion::Pausada, None, SystemTime::now())
        .await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    assert_eq!(simulado.cantidad(), 1);
    let notif = &simulado.notificaciones()[0];
    assert_eq!(notif.codigo.como_str(), CODIGO_BANEO_TEMPORAL);
    let dato = notif
        .datos
        .iter()
        .find(|d| d.clave == "expira_en")
        .expect("la clave expira_en nunca se omite");
    assert_eq!(
        dato.valor,
        ValorDeDato::Texto(EXPIRACION_DESCONOCIDA.to_string())
    );
}

#[tokio::test]
async fn ac2_baneo_temporal_exactamente_una_la_segunda_observacion_no_emite() {
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
        "exactamente una: la segunda observación no debe emitir"
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
async fn ac3_sesion_desvinculada_exactamente_una() {
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
        "exactamente una: la segunda evaluación no debe emitir"
    );
}

#[tokio::test]
async fn ac4_reconectando_sostenido_sin_cambio_de_estado_cruza_la_ventana() {
    // Forma exacta del cableado de producción: el sidecar emite `reconectando` UNA sola vez por
    // desconexión, así que el evaluador nunca vuelve a ver un cambio de estado. Solo una
    // reevaluación disparada por reloj, con el MISMO estado, puede cruzar la ventana. Si alguien
    // volviera a evaluar únicamente en el cambio de estado, este caso se pondría rojo.
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());
    let inicio = SystemTime::now();

    // Única observación del cambio de estado: silencio.
    assert!(
        evaluador
            .evaluar_estado_de_sesion(EstadoSesion::Reconectando, None, inicio)
            .is_empty()
    );

    // Reevaluaciones periódicas dentro de la ventana: siguen en silencio.
    for segundos in [60_u64, 120, 180, 240, 299] {
        let notifs = evaluador.evaluar_estado_de_sesion(
            EstadoSesion::Reconectando,
            None,
            inicio + Duration::from_secs(segundos),
        );
        assert!(
            notifs.is_empty(),
            "a los {segundos} s, dentro de la ventana, no debe emitir"
        );
    }

    // Reevaluación pasada la ventana: emite, y solo una vez.
    let notifs = evaluador.evaluar_estado_de_sesion(
        EstadoSesion::Reconectando,
        None,
        inicio + Duration::from_secs(360),
    );
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0].codigo.como_str(), CODIGO_SIN_RECONECTAR);

    let notifs = evaluador.evaluar_estado_de_sesion(
        EstadoSesion::Reconectando,
        None,
        inicio + Duration::from_secs(420),
    );
    assert!(
        notifs.is_empty(),
        "exactamente una: la reevaluación siguiente no debe emitir"
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
async fn ac6_saldo_agotado_exactamente_una() {
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
// AC-9 y AC-10: Caída anómala del ratio de acuses por contacto
// ---------------------------------------------------------------------------

/// Muestra de tres contactos en la que **el afectado no es el primero**.
///
/// Que no sea el primero es deliberado: es lo que permite distinguir la segmentación real de la
/// mutación «denunciar siempre el primer contacto». El agregado de la muestra (18/30 = 0,6) queda
/// por encima del umbral de prueba (0,5), de modo que un cómputo agregado no vería nada.
fn muestra_de_acuses() -> (Vec<(IdConversacion, u64, u64)>, IdConversacion) {
    let conv_afectada = IdConversacion::nuevo("conv-afectada");
    let contadores = vec![
        (IdConversacion::nuevo("conv-sana-1"), 10, 9),
        (conv_afectada.clone(), 10, 1),
        (IdConversacion::nuevo("conv-sana-2"), 10, 8),
    ];
    (contadores, conv_afectada)
}

/// El criterio de AC-9 como predicado, para poder afirmarlo en verde sobre el código real y en
/// rojo sobre cada mutante.
///
/// Exige tres cosas a la vez: exactamente una notificación, con el código de caída de acuses, y
/// que el dato `conversacion_afectada` **valga** el contacto afectado. La tercera es la que hace
/// viva la guarda: una afirmación de mera presencia de la clave dejaría pasar cualquier mutación
/// que nombre a otro contacto.
fn cumple_criterio_ac9(notificaciones: &[Notificacion], afectada: &IdConversacion) -> bool {
    notificaciones.len() == 1
        && notificaciones[0].codigo.como_str() == CODIGO_CAIDA_ACUSES
        && notificaciones[0].datos.iter().any(|d| {
            d.clave == "conversacion_afectada"
                && d.valor == ValorDeDato::Conversacion(afectada.clone())
        })
}

/// Mutante: denunciar siempre el primer contacto, sin mirar el ratio.
fn mutante_siempre_el_primer_contacto(
    contadores: &[(IdConversacion, u64, u64)],
) -> Vec<Notificacion> {
    contadores
        .first()
        .map(|(id, _, _)| {
            Notificacion::nueva(CodigoDeNotificacion::nuevo(CODIGO_CAIDA_ACUSES)).con_dato(
                "conversacion_afectada",
                ValorDeDato::Conversacion(id.clone()),
            )
        })
        .into_iter()
        .collect()
}

/// Mutante: colapsar la muestra a un único ratio agregado antes de evaluarla.
///
/// Recorre el MISMO evaluador y los MISMOS umbrales que el caso real; lo único que cambia es que
/// la segmentación por contacto desaparece.
fn mutante_agregado(contadores: &[(IdConversacion, u64, u64)]) -> Vec<Notificacion> {
    let enviados: u64 = contadores.iter().map(|(_, e, _)| e).sum();
    let acusados: u64 = contadores.iter().map(|(_, _, a)| a).sum();
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());
    evaluador.evaluar_acuses_por_contacto(&[(
        IdConversacion::nuevo("agregado-de-la-celula"),
        enviados,
        acusados,
    )])
}

/// Mutante: umbral inalcanzable, de modo que ningún ratio pueda caer por debajo.
fn mutante_umbral_inalcanzable(contadores: &[(IdConversacion, u64, u64)]) -> Vec<Notificacion> {
    let mut umbrales = umbrales_prueba();
    umbrales.limite_caida_ratio_acuses = 0.0;
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales);
    evaluador.evaluar_acuses_por_contacto(contadores)
}

#[tokio::test]
async fn ac9_caida_ratio_por_contacto_nombra_al_contacto_afectado_y_solo_a_el() {
    let sumidero = SumideroDeCelula::desde_configuracion(None);
    let emisor = EmisorDeAlertas::nuevo(umbrales_prueba(), sumidero.clone());
    let (contadores, conv_afectada) = muestra_de_acuses();

    emisor.evaluar_y_emitir_acuses(&contadores).await;

    let simulado = match &sumidero {
        SumideroDeCelula::Simulado(s) => s,
        _ => panic!("se esperaba sumidero simulado"),
    };
    let notificaciones = simulado.notificaciones();
    assert!(
        cumple_criterio_ac9(&notificaciones, &conv_afectada),
        "se esperaba exactamente una notificación nombrando a {}, y llegaron {:?}",
        conv_afectada.como_str(),
        notificaciones
    );

    // Ningún contacto sano puede aparecer nombrado en ninguna notificación.
    for sano in ["conv-sana-1", "conv-sana-2"] {
        let nombrado = notificaciones.iter().any(|n| {
            n.datos
                .iter()
                .any(|d| d.valor == ValorDeDato::Conversacion(IdConversacion::nuevo(sano)))
        });
        assert!(!nombrado, "ninguna notificación debe nombrar a {sano}");
    }
}

#[tokio::test]
async fn ac10_el_computo_por_contacto_detecta_lo_que_el_agregado_no_ve() {
    let (contadores, conv_afectada) = muestra_de_acuses();

    // Cómputo por contacto: detecta la caída y nombra al afectado.
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());
    let segmentadas = evaluador.evaluar_acuses_por_contacto(&contadores);
    assert!(
        cumple_criterio_ac9(&segmentadas, &conv_afectada),
        "la evaluación por contacto debe detectar la caída"
    );

    // Cómputo agregado sobre la MISMA muestra y con los MISMOS umbrales: no ve nada. El contraste
    // entre las dos líneas es la prueba de que la segmentación es exigida, no incidental.
    let agregadas = mutante_agregado(&contadores);
    assert!(
        agregadas.is_empty(),
        "el agregado (18/30 = 0,6) queda por encima del umbral y no debe emitir nada, \
         pero emitió {agregadas:?}"
    );
}

#[tokio::test]
async fn ac9_ac10_la_guarda_del_ratio_de_acuses_se_pone_roja_bajo_mutacion() {
    let (contadores, conv_afectada) = muestra_de_acuses();

    // Verde sobre el código real.
    let mut evaluador = EvaluadorDeAlertas::nuevo(umbrales_prueba());
    let reales = evaluador.evaluar_acuses_por_contacto(&contadores);
    assert!(
        cumple_criterio_ac9(&reales, &conv_afectada),
        "el criterio debe cumplirse sobre el evaluador sin mutar"
    );

    // Rojo bajo cada mutación. Si el criterio se debilitara a «la clave existe», la primera de
    // estas tres afirmaciones se pondría roja.
    let mutaciones: [(&str, Vec<Notificacion>); 3] = [
        (
            "denunciar siempre el primer contacto",
            mutante_siempre_el_primer_contacto(&contadores),
        ),
        (
            "evaluar el agregado en vez de segmentar por contacto",
            mutante_agregado(&contadores),
        ),
        (
            "umbral de caída inalcanzable",
            mutante_umbral_inalcanzable(&contadores),
        ),
    ];
    for (nombre, notificaciones) in mutaciones {
        assert!(
            !cumple_criterio_ac9(&notificaciones, &conv_afectada),
            "la mutación «{nombre}» debería poner la guarda en rojo, pero la pasó con {notificaciones:?}"
        );
    }
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
