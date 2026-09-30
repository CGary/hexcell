//! Tests del barrido de reservas huérfanas de presupuesto (HEX-092, AC-1 a AC-4 y AC-8 a AC-10).
//!
//! El barrido libera en **una** transacción toda reserva en estado `'activa'` cuyo instante de
//! creación sea estrictamente anterior a `ahora - antiguedad_maxima`, reutilizando la contabilidad
//! de `liberar_presupuesto` (devolución a `disponible` y movimiento `'liberacion'`).

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::DirectorioTemporal;
use hexcell_core::identidad::{IdConversacion, IdRemitente};
use hexcell_storage::presupuesto::ResumenDeBarridoDeReservas;
use hexcell_storage::{
    GestorDePools, NOMBRE_DE_ARCHIVO_DE_SESIONES, RepositorioDeSesiones, VeredictoDeReserva,
};
use rusqlite::Connection;

/// Antigüedad máxima del barrido en estos tests: 20 segundos, el límite de drenaje por defecto.
const ANTIGUEDAD_MAXIMA: Duration = Duration::from_secs(20);

/// Instante base para las reservas «antiguas»: el propio epoch Unix, siempre anterior al umbral.
const T0: SystemTime = SystemTime::UNIX_EPOCH;

fn repositorio(directorio: &DirectorioTemporal) -> RepositorioDeSesiones {
    let pools = Arc::new(GestorDePools::abrir(directorio.ruta()).expect("abrir los pools"));
    RepositorioDeSesiones::nuevo(pools)
}

fn crear_conversacion(repositorio: &RepositorioDeSesiones, conversacion: &IdConversacion) {
    let remitente = IdRemitente::nuevo("remitente-prueba");
    repositorio
        .anotar_entrante(
            conversacion,
            &remitente,
            "mensaje inicial",
            SystemTime::UNIX_EPOCH,
        )
        .expect("anotar mensaje entrante para crear la conversación");
}

/// Abre una conexión directa a `sessions.db` para inspeccionar filas, como ya hace
/// `tests/presupuesto.rs`: los tests de integración no ven `pools` (visibilidad de crate) y el
/// archivo de la base es la interfaz pública que sí pueden inspeccionar.
fn conexion_directa(directorio: &DirectorioTemporal) -> Connection {
    Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir sessions.db para inspeccionar el barrido")
}

#[test]
fn el_barrido_libera_una_reserva_activa_antigua_y_restaura_el_saldo() {
    let directorio = DirectorioTemporal::nuevo("barrido-libera-antigua");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-barrido-libera");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 3, T0)
    else {
        panic!("la reserva antigua debe concederse");
    };

    // ahora = t0 + 21 s con antigüedad máxima de 20 s: la reserva queda dentro del barrido.
    let resumen = repo
        .liberar_reservas_huerfanas(T0 + Duration::from_secs(21), ANTIGUEDAD_MAXIMA)
        .expect("el barrido debe completarse");

    assert_eq!(
        resumen,
        ResumenDeBarridoDeReservas {
            reservas_liberadas: 1,
            monto_liberado: 3,
        }
    );

    let saldo = repo.saldo().expect("consultar el saldo");
    assert_eq!(
        saldo.disponible, 10,
        "el monto liberado vuelve a disponible"
    );
    assert_eq!(saldo.reservado, 0);

    let conexion = conexion_directa(&directorio);
    let estado: String = conexion
        .query_row(
            "SELECT estado FROM reservas WHERE id = ?1",
            rusqlite::params![id_reserva],
            |fila| fila.get(0),
        )
        .expect("consultar el estado de la reserva");
    assert_eq!(estado, "liberada");

    let (clase, monto, saldo_resultante, id_reserva_mov, id_conv_mov): (
        String,
        i64,
        i64,
        i64,
        Option<String>,
    ) = conexion
        .query_row(
            "SELECT clase, monto, saldo_resultante, id_reserva, id_conversacion \
             FROM movimientos WHERE id_reserva = ?1 AND clase = 'liberacion'",
            rusqlite::params![id_reserva],
            |fila| {
                Ok((
                    fila.get(0)?,
                    fila.get(1)?,
                    fila.get(2)?,
                    fila.get(3)?,
                    fila.get(4)?,
                ))
            },
        )
        .expect("consultar el movimiento de liberación");
    assert_eq!(clase, "liberacion");
    assert_eq!(monto, 3);
    assert_eq!(saldo_resultante, 10);
    assert_eq!(id_reserva_mov, id_reserva);
    assert_eq!(id_conv_mov.as_deref(), Some("conv-barrido-libera"));
}

#[test]
fn una_reserva_joven_no_es_tocada_por_el_barrido() {
    let directorio = DirectorioTemporal::nuevo("barrido-joven");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-barrido-joven");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 3, T0 + Duration::from_secs(30))
    else {
        panic!("la reserva joven debe concederse");
    };

    // ahora = t0 + 19 s con antigüedad máxima de 20 s: la reserva (creada en t0 + 30 s) queda
    // fuera del barrido.
    let resumen = repo
        .liberar_reservas_huerfanas(T0 + Duration::from_secs(19), ANTIGUEDAD_MAXIMA)
        .expect("el barrido debe completarse");

    assert_eq!(
        resumen,
        ResumenDeBarridoDeReservas {
            reservas_liberadas: 0,
            monto_liberado: 0,
        }
    );

    let saldo = repo.saldo().expect("consultar el saldo");
    assert_eq!(saldo.disponible, 7);
    assert_eq!(saldo.reservado, 3);

    let conexion = conexion_directa(&directorio);
    let estado: String = conexion
        .query_row(
            "SELECT estado FROM reservas WHERE id = ?1",
            rusqlite::params![id_reserva],
            |fila| fila.get(0),
        )
        .expect("consultar el estado de la reserva");
    assert_eq!(estado, "activa");

    // El libro de movimientos no cambia: solo el aporte y la reserva.
    let numero_de_movimientos: i64 = conexion
        .query_row("SELECT COUNT(*) FROM movimientos", [], |fila| fila.get(0))
        .expect("contar los movimientos");
    assert_eq!(numero_de_movimientos, 2);
}

#[test]
fn una_reserva_en_el_umbral_exacto_no_se_libera() {
    let directorio = DirectorioTemporal::nuevo("barrido-umbral");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-barrido-umbral");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");

    // creada_ms == ahora - antiguedad: el umbral se compara con estricto menor.
    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 3, T0 + Duration::from_secs(1))
    else {
        panic!("la reserva en el umbral debe concederse");
    };

    let resumen = repo
        .liberar_reservas_huerfanas(T0 + Duration::from_secs(21), ANTIGUEDAD_MAXIMA)
        .expect("el barrido debe completarse");

    assert_eq!(
        resumen,
        ResumenDeBarridoDeReservas {
            reservas_liberadas: 0,
            monto_liberado: 0,
        }
    );

    let saldo = repo.saldo().expect("consultar el saldo");
    assert_eq!(saldo.disponible, 7);
    assert_eq!(saldo.reservado, 3);

    let conexion = conexion_directa(&directorio);
    let estado: String = conexion
        .query_row(
            "SELECT estado FROM reservas WHERE id = ?1",
            rusqlite::params![id_reserva],
            |fila| fila.get(0),
        )
        .expect("consultar el estado de la reserva");
    assert_eq!(estado, "activa");
}

#[test]
fn el_barrido_solo_toca_reservas_activas_antiguas() {
    let directorio = DirectorioTemporal::nuevo("barrido-mixto");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-barrido-mixto");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");

    // Antigua y activa: la única que el barrido debe liberar.
    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: antigua,
        ..
    }) = repo.reservar_presupuesto(&conv, 3, T0)
    else {
        panic!("la reserva antigua debe concederse");
    };

    // Joven y activa: debe sobrevivir.
    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: joven, ..
    }) = repo.reservar_presupuesto(&conv, 2, T0 + Duration::from_secs(30))
    else {
        panic!("la reserva joven debe concederse");
    };

    // Antigua pero conciliada: debe sobrevivir con su resuelta_ms.
    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: conciliada,
        ..
    }) = repo.reservar_presupuesto(&conv, 1, T0)
    else {
        panic!("la reserva a conciliar debe concederse");
    };
    repo.conciliar_presupuesto(conciliada, 1, T0 + Duration::from_secs(5))
        .expect("conciliar la reserva antigua");

    let resumen = repo
        .liberar_reservas_huerfanas(T0 + Duration::from_secs(21), ANTIGUEDAD_MAXIMA)
        .expect("el barrido debe completarse");

    assert_eq!(
        resumen,
        ResumenDeBarridoDeReservas {
            reservas_liberadas: 1,
            monto_liberado: 3,
        }
    );

    // 10 de aporte, -3 de la antigua, -2 de la joven, -1 de la conciliada, +3 de la liberada;
    // la conciliación exacta no mueve el saldo disponible.
    let saldo = repo.saldo().expect("consultar el saldo");
    assert_eq!(saldo.disponible, 7);
    assert_eq!(saldo.reservado, 2);

    let conexion = conexion_directa(&directorio);
    let (estado_antigua, resuelta_antigua): (String, Option<i64>) = conexion
        .query_row(
            "SELECT estado, resuelta_ms FROM reservas WHERE id = ?1",
            rusqlite::params![antigua],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        )
        .expect("consultar la reserva antigua");
    assert_eq!(estado_antigua, "liberada");
    assert_eq!(resuelta_antigua, Some(21_000));

    let (estado_joven, resuelta_joven): (String, Option<i64>) = conexion
        .query_row(
            "SELECT estado, resuelta_ms FROM reservas WHERE id = ?1",
            rusqlite::params![joven],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        )
        .expect("consultar la reserva joven");
    assert_eq!(estado_joven, "activa");
    assert!(resuelta_joven.is_none());

    let (estado_conciliada, resuelta_conciliada): (String, Option<i64>) = conexion
        .query_row(
            "SELECT estado, resuelta_ms FROM reservas WHERE id = ?1",
            rusqlite::params![conciliada],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        )
        .expect("consultar la reserva conciliada");
    assert_eq!(estado_conciliada, "conciliada");
    assert_eq!(resuelta_conciliada, Some(5_000));
}

#[test]
fn un_barrido_sin_reservas_devuelve_un_resumen_vacio() {
    let directorio = DirectorioTemporal::nuevo("barrido-vacio");
    let repo = repositorio(&directorio);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");
    let saldo_antes = repo.saldo().expect("consultar el saldo antes");

    let resumen = repo
        .liberar_reservas_huerfanas(T0 + Duration::from_secs(21), ANTIGUEDAD_MAXIMA)
        .expect("el barrido debe completarse");

    assert_eq!(
        resumen,
        ResumenDeBarridoDeReservas {
            reservas_liberadas: 0,
            monto_liberado: 0,
        }
    );

    let saldo_despues = repo.saldo().expect("consultar el saldo después");
    assert_eq!(saldo_antes, saldo_despues);
    assert_eq!(saldo_despues.disponible, 10);
    assert_eq!(saldo_despues.reservado, 0);
}

#[test]
fn un_fallo_durante_el_barrido_revierte_todo_el_barrido() {
    let directorio = DirectorioTemporal::nuevo("barrido-reversion");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-barrido-reversion");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: reserva_1,
        ..
    }) = repo.reservar_presupuesto(&conv, 2, T0)
    else {
        panic!("la reserva 1 debe concederse");
    };
    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: reserva_2,
        ..
    }) = repo.reservar_presupuesto(&conv, 3, T0)
    else {
        panic!("la reserva 2 debe concederse");
    };

    // Disparador que aborta la actualización de la segunda reserva: el barrido debe revertir
    // entero, dejando ambas reservas y el saldo intactos.
    let conexion = conexion_directa(&directorio);
    conexion
        .execute_batch(&format!(
            "CREATE TRIGGER abortar_segunda_liberacion \
             BEFORE UPDATE OF estado ON reservas \
             WHEN NEW.id = {reserva_2} AND NEW.estado = 'liberada' \
             BEGIN \
               SELECT RAISE(ABORT, 'abortar la liberacion de la reserva {reserva_2}'); \
             END;"
        ))
        .expect("crear el disparador de aborto");
    drop(conexion);

    let resultado =
        repo.liberar_reservas_huerfanas(T0 + Duration::from_secs(21), ANTIGUEDAD_MAXIMA);
    assert!(
        resultado.is_err(),
        "el barrido debe fallar ante el disparador"
    );

    let saldo = repo.saldo().expect("consultar el saldo");
    assert_eq!(saldo.disponible, 5);
    assert_eq!(saldo.reservado, 5);

    let conexion = conexion_directa(&directorio);
    for id_reserva in [reserva_1, reserva_2] {
        let (estado, resuelta_ms): (String, Option<i64>) = conexion
            .query_row(
                "SELECT estado, resuelta_ms FROM reservas WHERE id = ?1",
                rusqlite::params![id_reserva],
                |fila| Ok((fila.get(0)?, fila.get(1)?)),
            )
            .expect("consultar la reserva tras el fallo");
        assert_eq!(
            estado, "activa",
            "la reserva {id_reserva} debe seguir activa"
        );
        assert!(resuelta_ms.is_none());
    }
    let numero_de_movimientos: i64 = conexion
        .query_row("SELECT COUNT(*) FROM movimientos", [], |fila| fila.get(0))
        .expect("contar los movimientos");
    assert_eq!(
        numero_de_movimientos, 3,
        "solo aporte y las dos reservas: ninguna liberación debe quedar registrada"
    );
}

#[test]
fn el_barrido_libera_reservas_de_ingesta_sin_conversacion() {
    let directorio = DirectorioTemporal::nuevo("barrido-ingesta");
    let repo = repositorio(&directorio);

    repo.aportar_presupuesto(10, T0)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto_de_ingesta(4, T0)
    else {
        panic!("la reserva de ingesta debe concederse");
    };

    let resumen = repo
        .liberar_reservas_huerfanas(T0 + Duration::from_secs(21), ANTIGUEDAD_MAXIMA)
        .expect("el barrido debe completarse");

    assert_eq!(
        resumen,
        ResumenDeBarridoDeReservas {
            reservas_liberadas: 1,
            monto_liberado: 4,
        }
    );

    let saldo = repo.saldo().expect("consultar el saldo");
    assert_eq!(saldo.disponible, 10);
    assert_eq!(saldo.reservado, 0);

    let conexion = conexion_directa(&directorio);
    let (estado, id_conv_mov): (String, Option<String>) = conexion
        .query_row(
            "SELECT r.estado, m.id_conversacion FROM reservas r \
             JOIN movimientos m ON m.id_reserva = r.id AND m.clase = 'liberacion' \
             WHERE r.id = ?1",
            rusqlite::params![id_reserva],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        )
        .expect("consultar la reserva y su movimiento de liberación");
    assert_eq!(estado, "liberada");
    assert!(
        id_conv_mov.is_none(),
        "el movimiento de liberación de una reserva de ingesta conserva id_conversacion NULL"
    );
}
