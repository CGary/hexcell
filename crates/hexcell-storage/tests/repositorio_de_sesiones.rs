//! Tests del repositorio de `sessions.db`: ida y vuelta del historial y veredicto de duplicado.
//!
//! Ninguno duerme ni consulta un reloj: cada instante se le pasa explícitamente al repositorio,
//! igual que hace el motor con la marca temporal que le entrega el puerto de canal.

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::DirectorioTemporal;
use hexcell_core::canal::MensajeSaliente;
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use hexcell_storage::{
    EventoDeHistorial, GestorDePools, RepositorioDeSesiones, VeredictoDeDeduplicacion,
    a_milisegundos, desde_milisegundos,
};

const VENTANA: Duration = Duration::from_secs(3600);

fn repositorio(directorio: &DirectorioTemporal) -> RepositorioDeSesiones {
    let pools = Arc::new(GestorDePools::abrir(directorio.ruta()).expect("abrir los pools"));
    RepositorioDeSesiones::nuevo(pools)
}

#[test]
fn el_historial_de_una_conversacion_sin_registros_llega_vacio() {
    let directorio = DirectorioTemporal::nuevo("repositorio-vacio");
    let repositorio = repositorio(&directorio);

    let historial = repositorio
        .historial(&IdConversacion::nuevo("conversacion-inexistente"))
        .expect("leer el historial de una conversación desconocida no es un error");
    assert!(historial.is_empty());
}

#[test]
fn una_respuesta_libre_y_una_plantilla_sobreviven_a_la_ida_y_vuelta_por_sqlite() {
    let directorio = DirectorioTemporal::nuevo("repositorio-ida-y-vuelta");
    let repositorio = repositorio(&directorio);

    let conversacion = IdConversacion::nuevo("conversacion-ida-y-vuelta");
    let remitente = IdRemitente::nuevo("remitente-ida-y-vuelta");
    let instante = SystemTime::UNIX_EPOCH + Duration::from_secs(10);

    repositorio
        .anotar_entrante(&conversacion, &remitente, "hola", instante)
        .expect("anotar el evento entrante");
    repositorio
        .anotar_saliente(
            &conversacion,
            &MensajeSaliente::RespuestaLibre("hola".to_string()),
            instante,
        )
        .expect("anotar la respuesta libre");
    repositorio
        .anotar_saliente(
            &conversacion,
            &MensajeSaliente::Plantilla {
                id: "recordatorio_de_cita".to_string(),
                parametros: vec!["martes".to_string(), "10:30".to_string()],
            },
            instante,
        )
        .expect("anotar la plantilla");

    let historial = repositorio
        .historial(&conversacion)
        .expect("leer historial");
    assert_eq!(
        historial,
        vec![
            EventoDeHistorial::Entrante("hola".to_string()),
            EventoDeHistorial::Saliente(MensajeSaliente::RespuestaLibre("hola".to_string())),
            EventoDeHistorial::Saliente(MensajeSaliente::Plantilla {
                id: "recordatorio_de_cita".to_string(),
                parametros: vec!["martes".to_string(), "10:30".to_string()],
            }),
        ]
    );
}

#[test]
fn el_historial_de_una_conversacion_no_arrastra_el_de_otra() {
    let directorio = DirectorioTemporal::nuevo("repositorio-aislamiento");
    let repositorio = repositorio(&directorio);

    let primera = IdConversacion::nuevo("conversacion-primera");
    let segunda = IdConversacion::nuevo("conversacion-segunda");
    let remitente = IdRemitente::nuevo("remitente-compartido");
    let instante = SystemTime::UNIX_EPOCH;

    repositorio
        .anotar_entrante(&primera, &remitente, "de la primera", instante)
        .expect("anotar en la primera");
    repositorio
        .anotar_entrante(&segunda, &remitente, "de la segunda", instante)
        .expect("anotar en la segunda");

    assert_eq!(
        repositorio.historial(&primera).expect("leer la primera"),
        vec![EventoDeHistorial::Entrante("de la primera".to_string())]
    );
    assert_eq!(
        repositorio.historial(&segunda).expect("leer la segunda"),
        vec![EventoDeHistorial::Entrante("de la segunda".to_string())]
    );
}

#[test]
fn el_mismo_identificador_es_nuevo_la_primera_vez_y_duplicado_la_segunda() {
    let directorio = DirectorioTemporal::nuevo("repositorio-duplicado");
    let repositorio = repositorio(&directorio);
    let id = IdDeduplicacion::nuevo("id-repetido");
    let primera_llegada = SystemTime::UNIX_EPOCH;

    assert_eq!(
        repositorio
            .procesar_deduplicacion(&id, primera_llegada, VENTANA)
            .expect("primera llegada"),
        VeredictoDeDeduplicacion::Nuevo
    );

    let justo_antes_del_borde = primera_llegada + VENTANA - Duration::from_secs(1);
    assert_eq!(
        repositorio
            .procesar_deduplicacion(&id, justo_antes_del_borde, VENTANA)
            .expect("segunda llegada dentro de la ventana"),
        VeredictoDeDeduplicacion::Duplicado
    );
}

#[test]
fn una_reentrega_mas_alla_de_la_ventana_vuelve_a_parecer_nueva() {
    let directorio = DirectorioTemporal::nuevo("repositorio-poda");
    let repositorio = repositorio(&directorio);
    let id = IdDeduplicacion::nuevo("id-tardio");
    let primera_llegada = SystemTime::UNIX_EPOCH;

    assert_eq!(
        repositorio
            .procesar_deduplicacion(&id, primera_llegada, VENTANA)
            .expect("primera llegada"),
        VeredictoDeDeduplicacion::Nuevo
    );

    // Un evento ajeno adelanta el horizonte monótono y poda la entrada original.
    let mas_alla = primera_llegada + VENTANA + Duration::from_secs(1);
    repositorio
        .procesar_deduplicacion(
            &IdDeduplicacion::nuevo("evento-que-adelanta-el-horizonte"),
            mas_alla,
            VENTANA,
        )
        .expect("evento que adelanta el horizonte");

    assert_eq!(
        repositorio
            .procesar_deduplicacion(&id, mas_alla, VENTANA)
            .expect("reentrega tardía"),
        VeredictoDeDeduplicacion::Nuevo
    );
}

#[test]
fn el_horizonte_no_retrocede_ante_una_marca_temporal_atrasada() {
    let directorio = DirectorioTemporal::nuevo("repositorio-horizonte");
    let repositorio = repositorio(&directorio);

    let adelantado = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    repositorio
        .procesar_deduplicacion(&IdDeduplicacion::nuevo("adelantado"), adelantado, VENTANA)
        .expect("evento adelantado");

    // Un evento muy atrasado no debe hacer retroceder el horizonte: si lo hiciera, un
    // identificador ya podado volvería a considerarse retenido y la poda sería reversible.
    let atrasado = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    assert_eq!(
        repositorio
            .procesar_deduplicacion(&IdDeduplicacion::nuevo("atrasado"), atrasado, VENTANA)
            .expect("evento atrasado"),
        VeredictoDeDeduplicacion::Nuevo
    );
    assert_eq!(
        repositorio
            .procesar_deduplicacion(&IdDeduplicacion::nuevo("adelantado"), adelantado, VENTANA)
            .expect("repetición del adelantado"),
        VeredictoDeDeduplicacion::Duplicado
    );
}

#[test]
fn la_conversion_de_instantes_a_milisegundos_es_reversible_y_satura_en_los_extremos() {
    let instante = SystemTime::UNIX_EPOCH + Duration::from_millis(1_234_567);
    assert_eq!(a_milisegundos(instante), 1_234_567);
    assert_eq!(desde_milisegundos(1_234_567), instante);

    // Anterior al epoch: satura en el suelo del orden en vez de fallar.
    let anterior_al_epoch = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
    assert_eq!(a_milisegundos(anterior_al_epoch), 0);
    assert_eq!(desde_milisegundos(-1), SystemTime::UNIX_EPOCH);
}
