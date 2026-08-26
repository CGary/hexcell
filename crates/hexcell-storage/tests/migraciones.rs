//! Tests del corredor de migraciones sobre `PRAGMA user_version` (AC-1..AC-5).

mod comun;

use comun::DirectorioTemporal;
use hexcell_storage::{
    GestorDePools, NOMBRE_DE_ARCHIVO_DE_SESIONES, VERSION_DE_ESQUEMA_DE_CONOCIMIENTO,
    VERSION_DE_ESQUEMA_DE_SESIONES, aplicar_migraciones_de_sesiones,
};
use rusqlite::Connection;

/// Tablas e índices que la versión 2 del esquema de `sessions.db` debe dejar creados.
const OBJETOS_ESPERADOS: [(&str, &str); 13] = [
    ("table", "contactos"),
    ("table", "conversaciones"),
    ("table", "mensajes"),
    ("table", "parametros_de_plantilla"),
    ("table", "deduplicacion"),
    ("table", "estado_del_motor"),
    ("table", "saldo"),
    ("table", "reservas"),
    ("table", "movimientos"),
    ("index", "idx_mensajes_conversacion"),
    ("index", "idx_deduplicacion_marca"),
    ("index", "idx_reservas_activas"),
    ("index", "idx_movimientos_conversacion"),
];

#[test]
fn migrar_una_base_vacia_crea_el_esquema_completo_y_fija_la_version() {
    let directorio = DirectorioTemporal::nuevo("migraciones-vacia");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES);
    let conexion = Connection::open(&ruta).expect("abrir una base nueva");

    aplicar_migraciones_de_sesiones(&conexion).expect("migrar una base vacía debe funcionar");

    for (tipo, nombre) in OBJETOS_ESPERADOS {
        let encontrados: i64 = conexion
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = ?1 AND name = ?2",
                rusqlite::params![tipo, nombre],
                |fila| fila.get(0),
            )
            .expect("consultar el esquema almacenado");
        assert_eq!(encontrados, 1, "falta el objeto {tipo} {nombre}");
    }

    let version: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .expect("leer user_version");
    assert_eq!(version, VERSION_DE_ESQUEMA_DE_SESIONES);
}

#[test]
fn todas_las_tablas_de_sesiones_se_declaran_strict() {
    let directorio = DirectorioTemporal::nuevo("migraciones-strict");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES);
    let conexion = Connection::open(&ruta).expect("abrir una base nueva");
    aplicar_migraciones_de_sesiones(&conexion).expect("migrar una base vacía debe funcionar");

    let mut sentencia = conexion
        .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'table'")
        .expect("preparar la lectura del esquema");
    let tablas: Vec<(String, String)> = sentencia
        .query_map([], |fila| Ok((fila.get(0)?, fila.get(1)?)))
        .expect("leer el esquema")
        .map(|fila| fila.expect("una fila del esquema"))
        .collect();

    assert!(!tablas.is_empty());
    for (nombre, sql) in tablas {
        assert!(
            sql.to_uppercase().contains("STRICT"),
            "la tabla {nombre} no se declaró STRICT"
        );
    }
}

#[test]
fn volver_a_migrar_una_base_ya_migrada_es_una_operacion_nula() {
    let directorio = DirectorioTemporal::nuevo("migraciones-idempotente");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES);
    let conexion = Connection::open(&ruta).expect("abrir una base nueva");

    aplicar_migraciones_de_sesiones(&conexion).expect("primera migración");
    conexion
        .execute(
            "INSERT INTO estado_del_motor (clave, valor) VALUES ('centinela', 7)",
            [],
        )
        .expect("escribir un dato centinela");

    // Si la segunda pasada volviera a ejecutar el guion, el `CREATE TABLE` fallaría; y si lo
    // ejecutara borrando antes, el centinela desaparecería. Ninguna de las dos cosas ocurre.
    aplicar_migraciones_de_sesiones(&conexion).expect("segunda migración: operación nula");

    let centinela: i64 = conexion
        .query_row(
            "SELECT valor FROM estado_del_motor WHERE clave = 'centinela'",
            [],
            |fila| fila.get(0),
        )
        .expect("el dato centinela debe seguir ahí");
    assert_eq!(centinela, 7);
}

#[test]
fn reabrir_el_gestor_sobre_la_misma_ruta_no_vuelve_a_migrar_nada() {
    let directorio = DirectorioTemporal::nuevo("migraciones-reapertura");

    {
        let gestor = GestorDePools::abrir(directorio.ruta()).expect("primera apertura");
        gestor
            .sesiones()
            .con_escritura(|conexion| {
                conexion
                    .execute(
                        "INSERT INTO estado_del_motor (clave, valor) VALUES ('centinela', 42)",
                        [],
                    )
                    .expect("escribir el centinela");
                Ok(())
            })
            .expect("la escritura debe funcionar");
    }

    let gestor = GestorDePools::abrir(directorio.ruta()).expect("segunda apertura");
    let centinela = gestor
        .sesiones()
        .con_lectura(|conexion| {
            let valor: i64 = conexion
                .query_row(
                    "SELECT valor FROM estado_del_motor WHERE clave = 'centinela'",
                    [],
                    |fila| fila.get(0),
                )
                .expect("leer el centinela");
            Ok(valor)
        })
        .expect("la lectura debe funcionar");
    assert_eq!(centinela, 42);

    let version = gestor
        .conocimiento()
        .con_lectura(|conexion| {
            let version: i64 = conexion
                .query_row("PRAGMA user_version", [], |fila| fila.get(0))
                .expect("leer user_version del conocimiento");
            Ok(version)
        })
        .expect("la lectura de conocimiento debe funcionar");
    assert_eq!(version, VERSION_DE_ESQUEMA_DE_CONOCIMIENTO);
}

#[test]
fn upgrade_de_version_1_a_version_2_preserva_datos_preexistentes() {
    let directorio = DirectorioTemporal::nuevo("migraciones-upgrade-v1-v2");
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir base");
    conexion
        .execute_batch(include_str!(
            "../migraciones/sesiones/0001-esquema-inicial.sql"
        ))
        .expect("aplicar v1");
    conexion
        .execute_batch("PRAGMA user_version = 1;")
        .expect("fijar v1");
    conexion
        .execute(
            "INSERT INTO contactos (id_remitente, primera_actividad_ms, ultima_actividad_ms) VALUES ('c1', 100, 200)",
            [],
        )
        .expect("insertar contacto");
    conexion
        .execute(
            "INSERT INTO conversaciones (id_conversacion, creada_ms, ultima_actividad_ms) VALUES ('conv1', 100, 200)",
            [],
        )
        .expect("insertar conversacion");
    conexion
        .execute(
            "INSERT INTO mensajes (id, id_conversacion, id_remitente, direccion, clase, contenido, marca_temporal_ms) VALUES (1, 'conv1', 'c1', 'entrante', 'texto', 'hola', 150)",
            [],
        )
        .expect("insertar mensaje");

    aplicar_migraciones_de_sesiones(&conexion).expect("upgrade v1->v2");

    let version: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .expect("user_version");
    assert_eq!(version, 2);

    for (tipo, nombre) in OBJETOS_ESPERADOS {
        let encontrados: i64 = conexion
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = ?1 AND name = ?2",
                rusqlite::params![tipo, nombre],
                |fila| fila.get(0),
            )
            .expect("objeto esquema");
        assert_eq!(encontrados, 1, "falta objeto {tipo} {nombre}");
    }

    let msg: String = conexion
        .query_row("SELECT contenido FROM mensajes WHERE id = 1", [], |fila| {
            fila.get(0)
        })
        .expect("mensaje");
    assert_eq!(msg, "hola");
}

#[test]
fn restricciones_de_clave_foranea_en_movimientos_y_reservas_rechazan_filas_invalidas() {
    let directorio = DirectorioTemporal::nuevo("migraciones-fk");
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir base");
    conexion.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    aplicar_migraciones_de_sesiones(&conexion).unwrap();

    assert!(conexion.execute("INSERT INTO reservas (id_conversacion, monto_reservado, estado, creada_ms) VALUES ('x', 10, 'activa', 1)", []).is_err());
    assert!(conexion.execute("INSERT INTO movimientos (id_conversacion, clase, monto, saldo_resultante, registrado_ms) VALUES ('x', 'aporte', 10, 10, 1)", []).is_err());

    conexion
        .execute(
            "INSERT INTO conversaciones (id_conversacion, creada_ms, ultima_actividad_ms) VALUES ('conv1', 1, 1)",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO reservas (id, id_conversacion, monto_reservado, estado, creada_ms) VALUES (1, 'conv1', 10, 'activa', 1)",
            [],
        )
        .unwrap();

    assert!(conexion.execute("INSERT INTO movimientos (id_reserva, id_conversacion, clase, monto, saldo_resultante, registrado_ms) VALUES (99, 'conv1', 'reserva', -10, 0, 1)", []).is_err());
}

#[test]
fn restricciones_check_y_strict_rechazan_valores_invalidos() {
    let directorio = DirectorioTemporal::nuevo("migraciones-check");
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir base");
    conexion.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    aplicar_migraciones_de_sesiones(&conexion).unwrap();

    // saldo checks
    assert!(conexion.execute("INSERT INTO saldo (id, disponible, reservado, actualizado_ms) VALUES (2, 10, 0, 1)", []).is_err());
    assert!(
        conexion
            .execute("UPDATE saldo SET disponible = -1 WHERE id = 1", [])
            .is_err()
    );

    // reservas checks
    conexion
        .execute(
            "INSERT INTO conversaciones (id_conversacion, creada_ms, ultima_actividad_ms) VALUES ('conv1', 1, 1)",
            [],
        )
        .unwrap();
    assert!(conexion.execute("INSERT INTO reservas (id_conversacion, monto_reservado, estado, creada_ms) VALUES ('conv1', 0, 'activa', 1)", []).is_err());
    assert!(conexion.execute("INSERT INTO reservas (id_conversacion, monto_reservado, estado, creada_ms) VALUES ('conv1', 5, 'invalida', 1)", []).is_err());
    assert!(conexion.execute("INSERT INTO reservas (id_conversacion, monto_reservado, estado, creada_ms, resuelta_ms) VALUES ('conv1', 5, 'activa', 1, 10)", []).is_err());
    assert!(conexion.execute("INSERT INTO reservas (id_conversacion, monto_reservado, estado, creada_ms) VALUES ('conv1', 5, 'conciliada', 1)", []).is_err());

    // movimientos checks
    assert!(conexion.execute("INSERT INTO movimientos (clase, monto, saldo_resultante, registrado_ms) VALUES ('invalida', 10, 10, 1)", []).is_err());
    assert!(conexion.execute("INSERT INTO movimientos (clase, monto, saldo_resultante, registrado_ms) VALUES ('aporte', 0, 10, 1)", []).is_err());
    assert!(conexion.execute("INSERT INTO movimientos (clase, monto, saldo_resultante, registrado_ms) VALUES ('aporte', 10, -1, 1)", []).is_err());

    // STRICT checks
    assert!(conexion.execute("INSERT INTO movimientos (clase, monto, saldo_resultante, registrado_ms) VALUES ('aporte', 'abc', 10, 1)", []).is_err());
    assert!(
        conexion
            .execute("UPDATE saldo SET disponible = 'abc' WHERE id = 1", [])
            .is_err()
    );
}
