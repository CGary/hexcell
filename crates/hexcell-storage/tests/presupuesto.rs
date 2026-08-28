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

#[test]
fn ac_4_saldo_disponible_y_reservado_coincide() {
    let directorio = DirectorioTemporal::nuevo("saldo-coincide");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-saldo-coincide");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(15, SystemTime::UNIX_EPOCH)
        .expect("aportar 15");

    let Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) =
        repo.reservar_presupuesto(&conv, 5, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva concedida");
    };

    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 10);
    assert_eq!(saldo.reservado, 5);

    repo.conciliar_presupuesto(id_reserva, 3, SystemTime::UNIX_EPOCH)
        .expect("conciliar");

    let saldo_final = repo.saldo().expect("obtener saldo final");
    assert_eq!(saldo_final.disponible, 12);
    assert_eq!(saldo_final.reservado, 0);
}

#[test]
fn ac_5_desviacion_de_conciliacion_acumulada() {
    let directorio = DirectorioTemporal::nuevo("desviacion-conciliacion");
    let repo = repositorio(&directorio);
    let conv = IdConversacion::nuevo("conv-desviacion");
    crear_conversacion(&repo, &conv);

    assert_eq!(
        repo.desviacion_de_conciliacion()
            .expect("desviación inicial"),
        0
    );

    repo.aportar_presupuesto(30, SystemTime::UNIX_EPOCH)
        .expect("aportar 30");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: id_reserva_1,
        ..
    }) = repo.reservar_presupuesto(&conv, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva 1 concedida");
    };
    assert_eq!(
        repo.desviacion_de_conciliacion()
            .expect("desviación tras reserva"),
        0
    );

    repo.liberar_presupuesto(id_reserva_1, SystemTime::UNIX_EPOCH)
        .expect("liberar");
    assert_eq!(
        repo.desviacion_de_conciliacion()
            .expect("desviación tras liberación"),
        0
    );

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: id_reserva_2,
        ..
    }) = repo.reservar_presupuesto(&conv, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva 2 concedida");
    };
    repo.conciliar_presupuesto(id_reserva_2, 4, SystemTime::UNIX_EPOCH)
        .expect("conciliar 2");
    assert_eq!(
        repo.desviacion_de_conciliacion()
            .expect("desviación tras conciliación 2"),
        6
    );

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: id_reserva_3,
        ..
    }) = repo.reservar_presupuesto(&conv, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva 3 concedida");
    };
    repo.conciliar_presupuesto(id_reserva_3, 12, SystemTime::UNIX_EPOCH)
        .expect("conciliar 3");
    assert_eq!(
        repo.desviacion_de_conciliacion()
            .expect("desviación tras conciliación 3"),
        4
    );
}

#[test]
fn consumo_por_conversacion_agrega_unidades_de_multiples_conversaciones() {
    let directorio = DirectorioTemporal::nuevo("consumo-completo");

    {
        let repo = repositorio(&directorio);
        let consumo_inicial = repo.consumo_por_conversacion().expect("consumo inicial");
        assert!(consumo_inicial.is_empty());
    }

    let repo = repositorio(&directorio);
    let conv1 = IdConversacion::nuevo("conv-1");
    let conv2 = IdConversacion::nuevo("conv-2");
    let conv3 = IdConversacion::nuevo("conv-3");
    let conv4 = IdConversacion::nuevo("conv-4");

    crear_conversacion(&repo, &conv1);
    crear_conversacion(&repo, &conv2);
    crear_conversacion(&repo, &conv3);
    crear_conversacion(&repo, &conv4);

    repo.aportar_presupuesto(100, SystemTime::UNIX_EPOCH)
        .expect("aportar 100");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r1_1, ..
    }) = repo.reservar_presupuesto(&conv1, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva")
    };
    repo.conciliar_presupuesto(r1_1, 4, SystemTime::UNIX_EPOCH)
        .expect("conciliar");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r1_2, ..
    }) = repo.reservar_presupuesto(&conv1, 15, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva")
    };
    repo.conciliar_presupuesto(r1_2, 18, SystemTime::UNIX_EPOCH)
        .expect("conciliar");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r2_1, ..
    }) = repo.reservar_presupuesto(&conv2, 20, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva")
    };
    repo.conciliar_presupuesto(r2_1, 15, SystemTime::UNIX_EPOCH)
        .expect("conciliar");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r2_2, ..
    }) = repo.reservar_presupuesto(&conv2, 10, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva")
    };
    repo.liberar_presupuesto(r2_2, SystemTime::UNIX_EPOCH)
        .expect("liberar");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r3_1, ..
    }) = repo.reservar_presupuesto(&conv3, 8, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva")
    };
    repo.liberar_presupuesto(r3_1, SystemTime::UNIX_EPOCH)
        .expect("liberar");

    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r4_1, ..
    }) = repo.reservar_presupuesto(&conv4, 12, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva")
    };
    repo.conciliar_presupuesto(r4_1, 12, SystemTime::UNIX_EPOCH)
        .expect("conciliar");

    let consumo_antes_reinicio = repo
        .consumo_por_conversacion()
        .expect("consultar antes de reiniciar");

    assert_eq!(consumo_antes_reinicio.len(), 4);
    assert_eq!(
        consumo_antes_reinicio[0].id_conversacion.como_str(),
        "conv-1"
    );
    assert_eq!(consumo_antes_reinicio[0].unidades_consumidas, 22);

    assert_eq!(
        consumo_antes_reinicio[1].id_conversacion.como_str(),
        "conv-2"
    );
    assert_eq!(consumo_antes_reinicio[1].unidades_consumidas, 15);

    assert_eq!(
        consumo_antes_reinicio[2].id_conversacion.como_str(),
        "conv-3"
    );
    assert_eq!(consumo_antes_reinicio[2].unidades_consumidas, 0);

    assert_eq!(
        consumo_antes_reinicio[3].id_conversacion.como_str(),
        "conv-4"
    );
    assert_eq!(consumo_antes_reinicio[3].unidades_consumidas, 12);

    drop(repo);
    let repo_nuevo = repositorio(&directorio);
    let consumo_despues_reinicio = repo_nuevo
        .consumo_por_conversacion()
        .expect("consultar despues de reiniciar");

    assert_eq!(consumo_antes_reinicio, consumo_despues_reinicio);

    let ruta_db = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES);
    let conexion_directa = Connection::open(ruta_db).expect("abrir conexion directa");
    let mut sentencia = conexion_directa
        .prepare("SELECT id_conversacion, unidades_consumidas FROM consumo_por_conversacion ORDER BY id_conversacion")
        .expect("preparar consulta directa");
    let filas: Vec<(String, i64)> = sentencia
        .query_map([], |fila| Ok((fila.get(0)?, fila.get(1)?)))
        .expect("ejecutar consulta directa")
        .map(|r| r.expect("leer fila"))
        .collect();

    assert_eq!(filas.len(), 4);
    assert_eq!(filas[0], ("conv-1".to_string(), 22));
    assert_eq!(filas[1], ("conv-2".to_string(), 15));
    assert_eq!(filas[2], ("conv-3".to_string(), 0));
    assert_eq!(filas[3], ("conv-4".to_string(), 12));
}

#[test]
fn reservar_presupuesto_de_ingesta_permite_conciliar_y_liberar_sin_conversacion() {
    let directorio = DirectorioTemporal::nuevo("presupuesto-ingesta");
    let repo = repositorio(&directorio);

    // Aportar presupuesto inicial
    repo.aportar_presupuesto(100, SystemTime::UNIX_EPOCH)
        .expect("aportar 100 unidades");

    // 1. Reservar presupuesto de ingesta (conversación NULL)
    let veredicto = repo
        .reservar_presupuesto_de_ingesta(25, SystemTime::UNIX_EPOCH)
        .expect("reservar presupuesto de ingesta");

    let id_reserva = match veredicto {
        VeredictoDeReserva::Concedida {
            id_reserva,
            monto_reservado,
        } => {
            assert_eq!(monto_reservado, 25);
            id_reserva
        }
        _ => panic!("reserva de ingesta debió ser concedida"),
    };

    // Verificar el saldo
    let saldo = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo.disponible, 75);
    assert_eq!(saldo.reservado, 25);

    // Verificar en la base de datos que se haya insertado con id_conversacion NULL
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir sessions.db");
    let (id_conv_res, monto_res, estado_res): (Option<String>, i64, String) = conexion
        .query_row(
            "SELECT id_conversacion, monto_reservado, estado FROM reservas WHERE id = ?1",
            rusqlite::params![id_reserva],
            |fila| Ok((fila.get(0)?, fila.get(1)?, fila.get(2)?)),
        )
        .expect("consultar reserva");
    assert!(id_conv_res.is_none());
    assert_eq!(monto_res, 25);
    assert_eq!(estado_res, "activa");

    // Verificar que el movimiento correspondiente también tenga id_conversacion NULL
    let (id_conv_mov, clase_mov, monto_mov, saldo_resultante_mov): (Option<String>, String, i64, i64) = conexion
        .query_row(
            "SELECT id_conversacion, clase, monto, saldo_resultante FROM movimientos WHERE id_reserva = ?1",
            rusqlite::params![id_reserva],
            |fila| Ok((fila.get(0)?, fila.get(1)?, fila.get(2)?, fila.get(3)?)),
        )
        .expect("consultar movimiento");
    assert!(id_conv_mov.is_none());
    assert_eq!(clase_mov, "reserva");
    assert_eq!(monto_mov, -25);
    assert_eq!(saldo_resultante_mov, 75);

    // 2. Conciliar la reserva de ingesta con un consumo menor (excedente devuelto)
    let res_conciliacion = repo
        .conciliar_presupuesto(id_reserva, 20, SystemTime::UNIX_EPOCH)
        .expect("conciliar presupuesto de ingesta");

    assert_eq!(
        res_conciliacion,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: 5,
            deficit_no_cubierto: 0,
        }
    );

    let saldo_conciliado = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo_conciliado.disponible, 80);
    assert_eq!(saldo_conciliado.reservado, 0);

    // 3. Reservar de ingesta con liberación posterior
    let veredicto_liberacion = repo
        .reservar_presupuesto_de_ingesta(15, SystemTime::UNIX_EPOCH)
        .expect("reservar presupuesto de ingesta");

    let id_reserva_lib = match veredicto_liberacion {
        VeredictoDeReserva::Concedida { id_reserva, .. } => id_reserva,
        _ => panic!("reserva de ingesta para liberar debió ser concedida"),
    };

    let res_liberacion = repo
        .liberar_presupuesto(id_reserva_lib, SystemTime::UNIX_EPOCH)
        .expect("liberar presupuesto de ingesta");

    assert_eq!(
        res_liberacion,
        ResultadoDeResolucion::Resuelta {
            ajuste_aplicado: 15,
            deficit_no_cubierto: 0,
        }
    );

    let saldo_liberado = repo.saldo().expect("obtener saldo");
    assert_eq!(saldo_liberado.disponible, 80);
    assert_eq!(saldo_liberado.reservado, 0);
}

#[test]
fn vistas_consumo_por_conversacion_y_consumo_de_ingesta_no_se_mezclan() {
    let directorio = DirectorioTemporal::nuevo("vistas-consumo-separadas");
    let repo = repositorio(&directorio);

    let conv = IdConversacion::nuevo("conv-real");
    crear_conversacion(&repo, &conv);

    repo.aportar_presupuesto(200, SystemTime::UNIX_EPOCH)
        .expect("aportar 200");

    // Reserva con conversación real
    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r_conv, ..
    }) = repo.reservar_presupuesto(&conv, 50, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva conv");
    };
    repo.conciliar_presupuesto(r_conv, 40, SystemTime::UNIX_EPOCH)
        .unwrap();

    // Reserva de ingesta (conversación NULL)
    let Ok(VeredictoDeReserva::Concedida {
        id_reserva: r_ing, ..
    }) = repo.reservar_presupuesto_de_ingesta(100, SystemTime::UNIX_EPOCH)
    else {
        panic!("reserva ingesta");
    };
    repo.conciliar_presupuesto(r_ing, 85, SystemTime::UNIX_EPOCH)
        .unwrap();

    // Consultar consumo_por_conversacion a través del método público
    let consumos_conv = repo
        .consumo_por_conversacion()
        .expect("consumo por conversación");
    assert_eq!(consumos_conv.len(), 1);
    assert_eq!(consumos_conv[0].id_conversacion.como_str(), "conv-real");
    assert_eq!(consumos_conv[0].unidades_consumidas, 40);

    // Consultar consumo_de_ingesta directamente en SQLite
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir sessions.db");

    let consumo_ingesta_val: i64 = conexion
        .query_row(
            "SELECT unidades_consumidas FROM consumo_de_ingesta",
            [],
            |fila| fila.get(0),
        )
        .expect("consultar consumo_de_ingesta");
    assert_eq!(consumo_ingesta_val, 85);
}

#[test]
fn consumo_de_ingesta_sin_filas_devuelve_cero_por_coalesce() {
    let directorio = DirectorioTemporal::nuevo("consumo-ingesta-vacio");
    let repo = repositorio(&directorio);

    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir sessions.db");

    // Al no haber filas de ingesta (id_conversacion NULL), consumo_de_ingesta debe retornar exactamente una fila con 0.
    let consumo_ingesta_val: i64 = conexion
        .query_row(
            "SELECT unidades_consumidas FROM consumo_de_ingesta",
            [],
            |fila| fila.get(0),
        )
        .expect("consultar consumo_de_ingesta vacío");
    assert_eq!(consumo_ingesta_val, 0);
}
