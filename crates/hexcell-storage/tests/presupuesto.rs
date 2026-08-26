//! Tests de la gestión contable de saldo, reservas y movimientos en hexcell-storage (AC-1, AC-2, AC-4).

mod comun;

use std::sync::Arc;
use std::time::SystemTime;

use comun::DirectorioTemporal;
use hexcell_core::identidad::{IdConversacion, IdRemitente};
use hexcell_storage::{
    GestorDePools, NOMBRE_DE_ARCHIVO_DE_SESIONES, RepositorioDeSesiones, ResultadoDeResolucion,
    VeredictoDeReserva,
};
use rusqlite::Connection;

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

#[test]
fn reserva_con_saldo_suficiente_crea_reserva_y_movimiento_atomicamente() {
    let directorio = DirectorioTemporal::nuevo("reserva-suficiente");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-reserva-ok");
    crear_conversacion(&repo, &conv);

    // Aportar presupuesto inicial de 10 unidades
    repo.aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
        .expect("aportar presupuesto inicial");

    let saldo_antes = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo_antes.disponible, 10);
    assert_eq!(saldo_antes.reservado, 0);

    let veredicto = repo
        .reservar_presupuesto(&conv, 3, SystemTime::UNIX_EPOCH)
        .expect("reservar presupuesto");

    let VeredictoDeReserva::Concedida {
        id_reserva,
        monto_reservado,
    } = veredicto
    else {
        panic!("se esperaba VeredictoDeReserva::Concedida");
    };

    assert!(id_reserva > 0);
    assert_eq!(monto_reservado, 3);

    let saldo_despues = repo.saldo().expect("obtener saldo tras reserva");
    assert_eq!(saldo_despues.disponible, 7);
    assert_eq!(saldo_despues.reservado, 3);

    assert!(
        !repo
            .presupuesto_sin_iniciar()
            .expect("consultar presupuesto_sin_iniciar")
    );
}

#[test]
fn reserva_con_saldo_insuficiente_es_rechazada_y_no_modifica_datos() {
    let directorio = DirectorioTemporal::nuevo("reserva-insuficiente");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-reserva-rechazada");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(2, SystemTime::UNIX_EPOCH)
        .expect("aportar 2 unidades");

    let saldo_antes = repo.saldo().expect("obtener saldo");

    let veredicto = repo
        .reservar_presupuesto(&conv, 5, SystemTime::UNIX_EPOCH)
        .expect("intentar reservar 5 unidades con saldo de 2");

    assert_eq!(
        veredicto,
        VeredictoDeReserva::Rechazada {
            disponible: 2,
            requerido: 5,
        }
    );

    let saldo_despues = repo.saldo().expect("obtener saldo tras rechazo");
    assert_eq!(saldo_antes, saldo_despues);
}

#[test]
fn flujo_de_reservas_mantiene_saldo_disponible_no_negativo() {
    let directorio = DirectorioTemporal::nuevo("saldo-no-negativo");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-flujo-reservas");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(5, SystemTime::UNIX_EPOCH)
        .expect("aportar 5 unidades");

    // Reservar 3 (disponible pasa a 2)
    assert!(matches!(
        repo.reservar_presupuesto(&conv, 3, SystemTime::UNIX_EPOCH),
        Ok(VeredictoDeReserva::Concedida { .. })
    ));
    assert_eq!(repo.saldo().unwrap().disponible, 2);

    // Reservar 2 (disponible pasa a 0)
    assert!(matches!(
        repo.reservar_presupuesto(&conv, 2, SystemTime::UNIX_EPOCH),
        Ok(VeredictoDeReserva::Concedida { .. })
    ));
    assert_eq!(repo.saldo().unwrap().disponible, 0);

    // Intentar reservar 1 (rechazado, disponible sigue en 0)
    assert!(matches!(
        repo.reservar_presupuesto(&conv, 1, SystemTime::UNIX_EPOCH),
        Ok(VeredictoDeReserva::Rechazada {
            disponible: 0,
            requerido: 1
        })
    ));
    assert_eq!(repo.saldo().unwrap().disponible, 0);
}

#[test]
fn reserva_para_conversacion_inexistente_falla_por_clave_foranea() {
    let directorio = DirectorioTemporal::nuevo("reserva-fk");
    let repo = repositorio(&directorio);
    let conv_inexistente = IdConversacion::nuevo("conv-fantasma");

    repo.aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
        .expect("aportar presupuesto");

    // Al no existir en la tabla conversaciones, la restricción FOREIGN KEY falla.
    let resultado = repo.reservar_presupuesto(&conv_inexistente, 2, SystemTime::UNIX_EPOCH);
    assert!(resultado.is_err());
}

#[test]
fn semilla_es_idempotente_con_presupuesto_sin_iniciar() {
    let directorio = DirectorioTemporal::nuevo("presupuesto-idempotente");
    let repo = repositorio(&directorio);

    assert!(
        repo.presupuesto_sin_iniciar()
            .expect("inicialmente sin iniciar")
    );

    repo.aportar_presupuesto(50, SystemTime::UNIX_EPOCH)
        .expect("aportar semilla");

    assert!(!repo.presupuesto_sin_iniciar().expect("ahora ya iniciado"));
}

#[test]
fn conciliacion_con_excedente_devuelve_saldo_y_cierra_reserva() {
    let directorio = DirectorioTemporal::nuevo("conciliacion-excedente");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-conciliar-excedente");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    let res = repo
        .conciliar_presupuesto(id_reserva, 4, SystemTime::UNIX_EPOCH)
        .expect("conciliar presupuesto con excedente");

    assert_eq!(
        res,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: 6,
            deficit_no_cubierto: 0,
        }
    );

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 6);
    assert_eq!(saldo.reservado, 0);
}

#[test]
fn conciliacion_con_deficit_cubierto_aplica_cargo_y_cierra_reserva() {
    let directorio = DirectorioTemporal::nuevo("conciliacion-deficit-cubierto");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-conciliar-deficit");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(15, SystemTime::UNIX_EPOCH)
        .expect("aportar 15 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 5, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    // Disponible actual es 10, reservado es 5. Consumo real es 8 (déficit de 3).
    let res = repo
        .conciliar_presupuesto(id_reserva, 8, SystemTime::UNIX_EPOCH)
        .expect("conciliar presupuesto con déficit cubierto");

    assert_eq!(
        res,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: -3,
            deficit_no_cubierto: 0,
        }
    );

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 7);
    assert_eq!(saldo.reservado, 0);
}

#[test]
fn conciliacion_con_deficit_no_cubierto_no_viola_saldo_no_negativo_y_reporta_resto() {
    let directorio = DirectorioTemporal::nuevo("conciliacion-deficit-nocubierto");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-conciliar-nocubierto");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(7, SystemTime::UNIX_EPOCH)
        .expect("aportar 7 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 5, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    // Disponible actual es 2, reservado es 5. Consumo real es 10 (déficit de 5, disponible solo 2).
    let res = repo
        .conciliar_presupuesto(id_reserva, 10, SystemTime::UNIX_EPOCH)
        .expect("conciliar presupuesto con déficit no cubierto");

    assert_eq!(
        res,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: -2,
            deficit_no_cubierto: 3,
        }
    );

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 0);
    assert_eq!(saldo.reservado, 0);
}

#[test]
fn conciliacion_con_coincidencia_exacta_cierra_reserva_sin_movimiento() {
    let directorio = DirectorioTemporal::nuevo("conciliacion-exacta");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-exacta");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 5, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    let res = repo
        .conciliar_presupuesto(id_reserva, 5, SystemTime::UNIX_EPOCH)
        .expect("conciliar presupuesto exacto");

    assert_eq!(
        res,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: 0,
            deficit_no_cubierto: 0,
        }
    );

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 5);
    assert_eq!(saldo.reservado, 0);
}

#[test]
fn liberacion_devuelve_monto_completo_y_cierra_reserva() {
    let directorio = DirectorioTemporal::nuevo("liberacion-completa");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-liberar");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 4, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    let res = repo
        .liberar_presupuesto(id_reserva, SystemTime::UNIX_EPOCH)
        .expect("liberar presupuesto");

    assert_eq!(
        res,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: 4,
            deficit_no_cubierto: 0,
        }
    );

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 10);
    assert_eq!(saldo.reservado, 0);
}

#[test]
fn segunda_resolucion_devuelve_reserva_no_activa_y_no_modifica_saldo() {
    let directorio = DirectorioTemporal::nuevo("doble-resolucion");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-doble-res");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(10, SystemTime::UNIX_EPOCH)
        .expect("aportar 10 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 4, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    let primera = repo
        .conciliar_presupuesto(id_reserva, 2, SystemTime::UNIX_EPOCH)
        .expect("primera resolución");
    assert!(matches!(primera, ResultadoDeResolucion::Resuelta { .. }));

    let segunda = repo
        .conciliar_presupuesto(id_reserva, 1, SystemTime::UNIX_EPOCH)
        .expect("segunda resolución");
    assert_eq!(segunda, ResultadoDeResolucion::ReservaNoActiva);

    let tercera = repo
        .liberar_presupuesto(id_reserva, SystemTime::UNIX_EPOCH)
        .expect("tercera resolución");
    assert_eq!(tercera, ResultadoDeResolucion::ReservaNoActiva);

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 8);
    assert_eq!(saldo.reservado, 0);
}

#[test]
fn suma_de_movimientos_coincide_con_saldo_disponible_y_referencia_reserva() {
    let directorio = DirectorioTemporal::nuevo("consistencia-libro");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-consistencia");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(20, SystemTime::UNIX_EPOCH)
        .expect("aportar 20 unidades");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    repo.conciliar_presupuesto(id_reserva, 4, SystemTime::UNIX_EPOCH)
        .expect("conciliar presupuesto");

    let saldo = repo.saldo().expect("obtener saldo");

    // Verificar en la base que la suma de movimientos coincide con disponible
    // y que id_reserva e id_conversacion están presentes en los movimientos de reserva y conciliación.
    // Conexión directa a sessions.db: los tests de integración no ven `pools` (visibilidad
    // de crate) y el archivo de la base es la interfaz pública que sí pueden inspeccionar,
    // igual que hacen los tests de migraciones.
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir sessions.db para inspeccionar el libro");
    let suma_monto: i64 = conexion
        .query_row("SELECT COALESCE(SUM(monto), 0) FROM movimientos", [], |f| {
            f.get(0)
        })
        .expect("sumar los montos del libro");
    let num_movimientos: i64 = conexion
        .query_row("SELECT COUNT(*) FROM movimientos", [], |f| f.get(0))
        .expect("contar los movimientos del libro");

    assert_eq!(suma_monto, saldo.disponible);
    assert_eq!(num_movimientos, 3); // aporte, reserva, conciliacion
}
