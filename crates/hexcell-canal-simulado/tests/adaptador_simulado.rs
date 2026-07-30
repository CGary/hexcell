//! Tests del adaptador simulado, concretos a `AdaptadorSimulado`.
//!
//! Una batería genérica sobre cualquier `ChannelAdapter` es la etapa siguiente (HEX-005); estos
//! tests nombran el tipo concreto a propósito, sin adelantar ese alcance.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hexcell_canal_simulado::{AdaptadorSimulado, Reloj, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, DURACION_VENTANA_SERVICIO, EstadoVentanaServicio, EventoEntrante,
    MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

fn evento_de_prueba(conversacion: &IdConversacion, marca_temporal: SystemTime) -> EventoEntrante {
    EventoEntrante {
        remitente: IdRemitente::nuevo("remitente-de-prueba"),
        conversacion: conversacion.clone(),
        contenido: "hola".to_string(),
        marca_temporal,
        deduplicacion: IdDeduplicacion::nuevo("dedup-de-prueba"),
    }
}

#[tokio::test]
async fn la_ventana_expira_de_verdad_contra_el_reloj_de_prueba() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let conversacion = IdConversacion::nuevo("conversacion-ventana");

    adaptador
        .inyectar(evento_de_prueba(&conversacion, reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");

    // Antes de que expire la ventana: se acepta.
    reloj.avanzar(DURACION_VENTANA_SERVICIO - Duration::from_secs(1));
    let resultado_dentro = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("hola de vuelta".to_string()),
        )
        .await
        .expect("el envío simulado no falla en este test");
    assert_eq!(resultado_dentro, ResultadoEnvio::Aceptado);
    let estado_dentro = adaptador
        .estado_ventana(&conversacion)
        .await
        .expect("consultar el estado no falla en este test");
    assert!(matches!(
        estado_dentro,
        EstadoVentanaServicio::Abierta { .. }
    ));

    // Tras cruzar la duración completa desde el evento original: fuera de ventana.
    reloj.avanzar(Duration::from_secs(2));
    let resultado_fuera = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("hola tarde".to_string()),
        )
        .await
        .expect("el envío simulado no falla en este test");
    assert_eq!(resultado_fuera, ResultadoEnvio::FueraDeVentana);
    let estado_fuera = adaptador
        .estado_ventana(&conversacion)
        .await
        .expect("consultar el estado no falla en este test");
    assert_eq!(estado_fuera, EstadoVentanaServicio::Cerrada);
}

#[tokio::test]
async fn la_expiracion_es_instantanea_y_reproducible_sin_depender_del_reloj_de_pared() {
    let inicio_de_verdad = std::time::Instant::now();

    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let conversacion = IdConversacion::nuevo("conversacion-reproducible");

    adaptador
        .inyectar(evento_de_prueba(&conversacion, reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");
    reloj.avanzar(DURACION_VENTANA_SERVICIO + Duration::from_secs(1));

    let resultado = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("hola".to_string()),
        )
        .await
        .expect("el envío simulado no falla en este test");
    assert_eq!(resultado, ResultadoEnvio::FueraDeVentana);

    // Nada de esto debería haber tardado un tiempo comparable a 24 horas de reloj de pared.
    assert!(inicio_de_verdad.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn los_cuatro_rechazos_son_disparables_y_distinguibles() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);

    let conversaciones: Vec<IdConversacion> = (0..4)
        .map(|indice| IdConversacion::nuevo(format!("conversacion-rechazo-{indice}")))
        .collect();
    let rechazos = [
        ResultadoEnvio::FueraDeVentana,
        ResultadoEnvio::PlantillaRequerida,
        ResultadoEnvio::LimiteDeTasa,
        ResultadoEnvio::DestinatarioInvalido,
    ];

    for (conversacion, rechazo) in conversaciones.iter().zip(rechazos.iter()) {
        adaptador.forzar(conversacion, *rechazo);
    }

    let mut obtenidos = Vec::new();
    for conversacion in &conversaciones {
        let resultado = adaptador
            .send(
                conversacion,
                MensajeSaliente::RespuestaLibre("mensaje".to_string()),
            )
            .await
            .expect("el envío simulado no falla en este test");
        obtenidos.push(resultado);
    }

    assert_eq!(obtenidos, rechazos.to_vec());
    for i in 0..obtenidos.len() {
        for j in (i + 1)..obtenidos.len() {
            assert_ne!(
                obtenidos[i], obtenidos[j],
                "los cuatro rechazos deben ser distinguibles"
            );
        }
    }
}

#[tokio::test]
async fn un_rechazo_forzado_solo_se_aplica_a_una_llamada() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let conversacion = IdConversacion::nuevo("conversacion-forzado-unico");

    adaptador
        .inyectar(evento_de_prueba(&conversacion, reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");
    adaptador.forzar(&conversacion, ResultadoEnvio::LimiteDeTasa);

    let primero = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("mensaje uno".to_string()),
        )
        .await
        .expect("el envío simulado no falla en este test");
    assert_eq!(primero, ResultadoEnvio::LimiteDeTasa);

    let segundo = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("mensaje dos".to_string()),
        )
        .await
        .expect("el envío simulado no falla en este test");
    assert_eq!(segundo, ResultadoEnvio::Aceptado);
}

#[tokio::test]
async fn forzar_averia_hace_fallar_exactamente_al_siguiente_envio() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);
    let conversacion = IdConversacion::nuevo("conversacion-averia");

    adaptador.forzar_averia();
    let resultado = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("mensaje".to_string()),
        )
        .await;
    assert!(resultado.is_err());

    let siguiente = adaptador
        .send(
            &conversacion,
            MensajeSaliente::RespuestaLibre("mensaje".to_string()),
        )
        .await
        .expect("la avería ya se consumió, este envío no debe fallar");
    assert_eq!(siguiente, ResultadoEnvio::Aceptado);
}

#[tokio::test]
async fn envios_capturados_registra_cada_envio_realizado() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let conversacion = IdConversacion::nuevo("conversacion-captura");

    adaptador
        .inyectar(evento_de_prueba(&conversacion, reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");
    let mensaje = MensajeSaliente::RespuestaLibre("capturado".to_string());
    adaptador
        .send(&conversacion, mensaje.clone())
        .await
        .expect("el envío simulado no falla en este test");

    let capturas = adaptador.envios_capturados();
    assert_eq!(capturas.len(), 1);
    assert_eq!(capturas[0].0, conversacion);
    assert_eq!(capturas[0].1, mensaje);
    assert_eq!(capturas[0].2, ResultadoEnvio::Aceptado);
}

#[tokio::test]
async fn un_contacto_resuelve_a_la_misma_conversacion_antes_y_despues_de_re_emparejar() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);

    let conversacion_antes = adaptador
        .inyectar_desde_contacto(
            "contacto-estable",
            "hola",
            hexcell_core::identidad::IdDeduplicacion::nuevo("dedup-antes"),
        )
        .await
        .expect("el canal recién creado debe aceptar el evento");

    let dispositivo_antes = adaptador.dispositivo_actual();
    adaptador.re_emparejar("dispositivo-nuevo");
    assert_ne!(
        adaptador.dispositivo_actual(),
        dispositivo_antes,
        "re_emparejar debe cambiar el dispositivo actual"
    );

    let conversacion_despues = adaptador
        .inyectar_desde_contacto(
            "contacto-estable",
            "hola de nuevo",
            hexcell_core::identidad::IdDeduplicacion::nuevo("dedup-despues"),
        )
        .await
        .expect("el canal recién creado debe aceptar el evento");

    assert_eq!(
        conversacion_antes, conversacion_despues,
        "el mismo contacto debe resolver siempre a la misma conversación, pase lo que pase con \
         el dispositivo emparejado"
    );
}

#[tokio::test]
async fn dos_contactos_distintos_resuelven_a_conversaciones_distintas() {
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, _receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);

    let conversacion_uno = adaptador
        .inyectar_desde_contacto(
            "contacto-uno",
            "hola",
            hexcell_core::identidad::IdDeduplicacion::nuevo("dedup-uno"),
        )
        .await
        .expect("el canal recién creado debe aceptar el evento");
    let conversacion_dos = adaptador
        .inyectar_desde_contacto(
            "contacto-dos",
            "hola",
            hexcell_core::identidad::IdDeduplicacion::nuevo("dedup-dos"),
        )
        .await
        .expect("el canal recién creado debe aceptar el evento");

    assert_ne!(conversacion_uno, conversacion_dos);
}
