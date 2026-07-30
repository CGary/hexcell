//! Batería de tests de contrato: un `pub async fn` por escenario, genérico sobre [`BancoDeCanal`].
//!
//! Cada escenario es una función independiente para que, si uno falla, el nombre de la función
//! que aparece en el reporte de test identifique el escenario concreto, en vez de un único
//! `assert` opaco dentro de una macro. Los dos escenarios temporales piden además
//! [`BancoConRelojControlable`], porque necesitan expirar la ventana de servicio de verdad.
//!
//! [`verificar_contrato_del_puerto`] es el punto de entrada agregado: awaitar una sola función
//! basta para correr la batería completa contra un banco que sí controla su reloj.

use std::time::Duration;

use hexcell_core::canal::{
    ChannelAdapter, DURACION_VENTANA_SERVICIO, EstadoVentanaServicio, MensajeSaliente,
    ResultadoEnvio,
};
use hexcell_core::identidad::IdConversacion;

use crate::banco::{BancoConRelojControlable, BancoDeCanal};

/// Los cuatro rechazos de FR-12 se fuerzan, se producen y se observan de forma distinguible, y
/// las cuatro variantes resultantes son distintas entre sí (AC-2).
pub async fn verifica_los_cuatro_rechazos_son_forzables_y_distinguibles<B: BancoDeCanal>(
    banco: &B,
) {
    let rechazos = [
        ResultadoEnvio::FueraDeVentana,
        ResultadoEnvio::PlantillaRequerida,
        ResultadoEnvio::LimiteDeTasa,
        ResultadoEnvio::DestinatarioInvalido,
    ];

    let conversaciones: Vec<IdConversacion> = (0..rechazos.len())
        .map(|indice| IdConversacion::nuevo(format!("contrato-rechazo-{indice}")))
        .collect();

    for (conversacion, rechazo) in conversaciones.iter().zip(rechazos.iter()) {
        banco.forzar_resultado(conversacion, *rechazo);
    }

    let mut obtenidos = Vec::with_capacity(rechazos.len());
    for conversacion in &conversaciones {
        let resultado = banco
            .adaptador()
            .send(
                conversacion,
                MensajeSaliente::RespuestaLibre("contrato".to_string()),
            )
            .await
            .expect("el banco no debe producir una avería de transporte en este escenario");
        obtenidos.push(resultado);
    }

    assert_eq!(obtenidos, rechazos.to_vec());
    for i in 0..obtenidos.len() {
        for j in (i + 1)..obtenidos.len() {
            assert_ne!(
                obtenidos[i], obtenidos[j],
                "los cuatro rechazos de FR-12 deben ser distinguibles entre sí"
            );
        }
    }
}

/// Un rechazo forzado se consume en la llamada para la que se forzó y no tiñe la siguiente
/// llamada sobre la misma conversación, que vuelve a su comportamiento natural (AC-2).
pub async fn verifica_un_rechazo_forzado_se_aplica_a_una_sola_llamada<B: BancoDeCanal>(banco: &B) {
    let conversacion = IdConversacion::nuevo("contrato-rechazo-unico");
    banco.inyectar_evento(&conversacion).await;
    banco.forzar_resultado(&conversacion, ResultadoEnvio::LimiteDeTasa);

    let primero = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("primero".to_string()),
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(primero, ResultadoEnvio::LimiteDeTasa);

    let segundo = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("segundo".to_string()),
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(
        segundo,
        ResultadoEnvio::Aceptado,
        "el rechazo forzado no debe aplicarse más que a una sola llamada"
    );
}

/// La asimetría real de la Cloud API, y la razón de que `MensajeSaliente` sea un enum: una
/// `Plantilla` se acepta donde una `RespuestaLibre` sobre la misma conversación se rechazaría por
/// ventana cerrada (AC-2).
pub async fn verifica_plantilla_se_acepta_donde_respuesta_libre_se_rechaza<B: BancoDeCanal>(
    banco: &B,
) {
    let conversacion = IdConversacion::nuevo("contrato-asimetria-plantilla");
    banco.forzar_resultado(&conversacion, ResultadoEnvio::FueraDeVentana);

    let respuesta_libre = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("texto libre".to_string()),
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(respuesta_libre, ResultadoEnvio::FueraDeVentana);

    let plantilla = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::Plantilla {
                id: "plantilla-de-contrato".to_string(),
                parametros: Vec::new(),
            },
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(
        plantilla,
        ResultadoEnvio::Aceptado,
        "una plantilla debe aceptarse donde una respuesta libre se rechazaría"
    );
}

/// `estado_ventana` concuerda con lo que `send` acaba de hacer, en vez de contradecirlo (AC-2).
pub async fn verifica_estado_ventana_concuerda_con_send<B: BancoDeCanal>(banco: &B) {
    let conversacion = IdConversacion::nuevo("contrato-estado-concuerda");
    banco.inyectar_evento(&conversacion).await;

    let resultado = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("hola".to_string()),
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(resultado, ResultadoEnvio::Aceptado);

    let estado = banco
        .adaptador()
        .estado_ventana(&conversacion)
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert!(
        matches!(estado, EstadoVentanaServicio::Abierta { .. }),
        "un envío aceptado no debe convivir con una ventana ya cerrada"
    );
}

/// Con la ventana abierta, una `RespuestaLibre` se acepta y `estado_ventana` la reporta abierta
/// (AC-2).
pub async fn verifica_ventana_abierta_acepta_respuesta_libre<B: BancoConRelojControlable>(
    banco: &B,
) {
    let conversacion = IdConversacion::nuevo("contrato-ventana-abierta");
    banco.inyectar_evento(&conversacion).await;

    let resultado = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("dentro de ventana".to_string()),
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(resultado, ResultadoEnvio::Aceptado);

    let estado = banco
        .adaptador()
        .estado_ventana(&conversacion)
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert!(matches!(estado, EstadoVentanaServicio::Abierta { .. }));
}

/// Tras avanzar el reloj del banco más allá de `DURACION_VENTANA_SERVICIO`, la misma conversación
/// pasa a `FueraDeVentana` y `estado_ventana` la reporta cerrada (AC-2).
pub async fn verifica_ventana_expira_tras_duracion_de_servicio<B: BancoConRelojControlable>(
    banco: &B,
) {
    let conversacion = IdConversacion::nuevo("contrato-ventana-expira");
    banco.inyectar_evento(&conversacion).await;
    banco.avanzar(DURACION_VENTANA_SERVICIO + Duration::from_secs(1));

    let resultado = banco
        .adaptador()
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("tarde".to_string()),
        )
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(resultado, ResultadoEnvio::FueraDeVentana);

    let estado = banco
        .adaptador()
        .estado_ventana(&conversacion)
        .await
        .expect("el banco no debe producir una avería de transporte en este escenario");
    assert_eq!(estado, EstadoVentanaServicio::Cerrada);
}

/// Ejecuta la batería completa, en orden, contra un banco que controla su reloj.
///
/// Un consumidor —el test de `hexcell-canal-simulado` de esta misma tarea, y la etapa B-1 contra
/// la Cloud API real después— solo necesita awaitar esta función para correr todo el contrato.
pub async fn verificar_contrato_del_puerto<B: BancoConRelojControlable>(banco: &B) {
    verifica_los_cuatro_rechazos_son_forzables_y_distinguibles(banco).await;
    verifica_un_rechazo_forzado_se_aplica_a_una_sola_llamada(banco).await;
    verifica_plantilla_se_acepta_donde_respuesta_libre_se_rechaza(banco).await;
    verifica_estado_ventana_concuerda_con_send(banco).await;
    verifica_ventana_abierta_acepta_respuesta_libre(banco).await;
    verifica_ventana_expira_tras_duracion_de_servicio(banco).await;
}
