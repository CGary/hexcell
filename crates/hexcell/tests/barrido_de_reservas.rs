//! Tests de proceso real del barrido de reservas huérfanas en el arranque (HEX-092, AC-5, AC-6, AC-9).
//!
//! Siguen el patrón de `tests/apagado_ordenado.rs`: se siembra `sessions.db` con la reserva
//! deseada, se sueltan los pools de siembra, se lanza el binario compilado (el lanzador ya espera
//! las líneas `salud_vinculada`/`admin_vinculada`, así que el barrido, que ocurre antes, ya corrió)
//! y se relee el saldo con una persistencia nueva mientras el binario sigue vivo (WAL,
//! multiproceso). No se usa ningún `sleep`.

mod comun;

use std::time::SystemTime;

use comun::{DirectorioTemporal, abrir_persistencia, lanzar_binario_con_ruta_de_datos};
use hexcell_core::identidad::{IdConversacion, IdRemitente};
use hexcell_storage::{RepositorioDeSesiones, VeredictoDeReserva};

const CONVERSACION_DE_SIEMBRA: &str = "conversacion-del-barrido";
const REMITENTE_DE_SIEMBRA: &str = "remitente-del-barrido";

/// Crea la conversación de siembra y devuelve su identificador.
fn sembrar_conversacion(repositorio: &RepositorioDeSesiones) -> IdConversacion {
    let conversacion = IdConversacion::nuevo(CONVERSACION_DE_SIEMBRA);
    let remitente = IdRemitente::nuevo(REMITENTE_DE_SIEMBRA);
    repositorio
        .anotar_entrante(
            &conversacion,
            &remitente,
            "mensaje de siembra",
            SystemTime::UNIX_EPOCH,
        )
        .expect("crear la conversación de siembra");
    conversacion
}

#[test]
fn el_arranque_libera_una_reserva_antigua_y_emite_el_evento_antes_de_servir() {
    let directorio = DirectorioTemporal::nuevo("barrido-binario-antigua");

    {
        let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
        let conversacion = sembrar_conversacion(&repositorio);
        repositorio
            .aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
            .expect("aportar 10 unidades");
        let Ok(VeredictoDeReserva::Concedida { .. }) =
            repositorio.reservar_presupuesto(&conversacion, 3, SystemTime::UNIX_EPOCH)
        else {
            panic!("la reserva antigua debe concederse");
        };
        // Los pools de siembra se sueltan aquí: el binario debe abrir la base por su cuenta.
    }

    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let salida = binario.salida_capturada();
    let posicion_evento = salida
        .find("reservas_huerfanas_liberadas")
        .unwrap_or_else(|| panic!("el barrido debe emitir su evento de arranque: {salida}"));
    let detalle = &salida[posicion_evento..];
    assert!(
        detalle.contains("recuento=1 monto=3"),
        "el evento debe llevar el detalle recuento=1 monto=3: {salida}"
    );
    let posicion_salud = salida
        .find("salud_vinculada")
        .expect("el binario debe llegar a vincular el servidor de salud");
    assert!(
        posicion_evento < posicion_salud,
        "el barrido debe ocurrir antes de que el HTTP acepte tráfico: {salida}"
    );

    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let saldo = repositorio
        .saldo()
        .expect("consultar el saldo tras el arranque");
    assert_eq!(saldo.reservado, 0, "la reserva huérfana debe liberarse");
    assert_eq!(saldo.disponible, 10, "el monto debe volver a disponible");
}

#[test]
fn el_arranque_sin_reservas_emite_sin_cambios() {
    let directorio = DirectorioTemporal::nuevo("barrido-binario-sin-cambios");

    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let salida = binario.salida_capturada();
    let posicion_evento = salida
        .find("reservas_huerfanas_liberadas")
        .unwrap_or_else(|| panic!("el barrido debe emitir su evento de arranque: {salida}"));
    assert!(
        salida[posicion_evento..].contains("sin cambios"),
        "sin reservas el detalle debe ser 'sin cambios': {salida}"
    );
    assert!(
        !salida[posicion_evento..].contains("recuento="),
        "sin reservas no debe haber detalle de recuento: {salida}"
    );
}

#[test]
fn una_reserva_joven_sobrevive_al_arranque() {
    let directorio = DirectorioTemporal::nuevo("barrido-binario-joven");

    {
        let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
        let conversacion = sembrar_conversacion(&repositorio);
        repositorio
            .aportar_presupuesto(10, SystemTime::now())
            .expect("aportar 10 unidades");
        let Ok(VeredictoDeReserva::Concedida { .. }) =
            repositorio.reservar_presupuesto(&conversacion, 3, SystemTime::now())
        else {
            panic!("la reserva joven debe concederse");
        };
    }

    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let salida = binario.salida_capturada();
    assert!(
        salida.contains("reservas_huerfanas_liberadas"),
        "el barrido debe emitir su evento de arranque: {salida}"
    );

    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let saldo = repositorio
        .saldo()
        .expect("consultar el saldo tras el arranque");
    assert_eq!(
        saldo.reservado, 3,
        "la reserva joven (muy por debajo del límite de drenaje) debe sobrevivir"
    );
    assert_eq!(saldo.disponible, 7);
}
