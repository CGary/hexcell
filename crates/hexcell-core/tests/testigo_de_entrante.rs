//! Test de integración del testigo de entrante (HEX-016, 2026-08-09).
//!
//! Archivo propio para que Cargo lo compile en su propio proceso: el contador de rechazos es un
//! `AtomicU64` estático de proceso. Asertar deltas protege contra la ACUMULACIÓN entre tests,
//! pero no contra incrementos CONCURRENTES: dos tests de este mismo binario corriendo en hilos
//! paralelos pueden incrementar el contador entre las dos lecturas de un delta y romperlo
//! (~1 % de las corridas, medido). Por eso los tests que tocan el contador se serializan con
//! [`EN_SERIE`] además de asertar deltas.

use std::sync::Mutex;
use std::time::SystemTime;

use hexcell_core::canal::{
    EventoEntrante, MensajeSaliente, TestigoDeEntrante, rechazos_de_construccion,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

/// Serializa los tests de este binario que observan el contador estático de proceso.
static EN_SERIE: Mutex<()> = Mutex::new(());

fn evento_para(conversacion: &str) -> EventoEntrante {
    EventoEntrante {
        remitente: IdRemitente::nuevo("rem-test"),
        conversacion: IdConversacion::nuevo(conversacion),
        contenido: "contenido".to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo("dedup-test"),
    }
}

#[test]
fn un_intento_cruzado_incrementa_el_contador_y_el_camino_correcto_no() {
    let _guardia = EN_SERIE
        .lock()
        .unwrap_or_else(|envenenado| envenenado.into_inner());
    let antes = rechazos_de_construccion();

    // Intento cruzado: testigo de "conv-a", destino "conv-b".
    let evento_a = evento_para("conv-a");
    let testigo = TestigoDeEntrante::observar(&evento_a);
    let resultado = MensajeSaliente::respuesta_libre(
        &testigo,
        &IdConversacion::nuevo("conv-b"),
        "texto".to_string(),
    );
    assert!(resultado.is_err(), "el intento cruzado debe ser rechazado");

    let despues_del_rechazo = rechazos_de_construccion();
    assert_eq!(
        despues_del_rechazo - antes,
        1,
        "el contador debe incrementarse exactamente en uno tras un rechazo"
    );

    // Camino correcto: misma conversación.
    let resultado_ok = MensajeSaliente::respuesta_libre(
        &testigo,
        &IdConversacion::nuevo("conv-a"),
        "texto".to_string(),
    );
    assert!(
        resultado_ok.is_ok(),
        "la construcción con la misma conversación debe tener éxito"
    );

    let despues_del_exito = rechazos_de_construccion();
    assert_eq!(
        despues_del_exito, despues_del_rechazo,
        "el contador no debe cambiar tras una construcción exitosa"
    );
}

#[test]
fn la_plantilla_tambien_valida_la_conversacion() {
    let _guardia = EN_SERIE
        .lock()
        .unwrap_or_else(|envenenado| envenenado.into_inner());
    let antes = rechazos_de_construccion();

    let evento = evento_para("conv-plantilla");
    let testigo = TestigoDeEntrante::observar(&evento);

    // Intento cruzado con plantilla.
    let resultado = MensajeSaliente::plantilla(
        &testigo,
        &IdConversacion::nuevo("conv-otra"),
        "plantilla-1".to_string(),
        vec![],
    );
    assert!(resultado.is_err());
    assert_eq!(rechazos_de_construccion() - antes, 1);

    // Camino correcto.
    let resultado_ok = MensajeSaliente::plantilla(
        &testigo,
        &IdConversacion::nuevo("conv-plantilla"),
        "plantilla-1".to_string(),
        vec!["param".to_string()],
    );
    assert!(resultado_ok.is_ok());
    assert_eq!(
        rechazos_de_construccion() - antes,
        1,
        "sin incremento adicional"
    );
}
