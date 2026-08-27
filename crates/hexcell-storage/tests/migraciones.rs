//! Tests del corredor de migraciones sobre `PRAGMA user_version` (AC-1..AC-5).

mod comun;

use comun::DirectorioTemporal;
use hexcell_storage::{
    GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_SESIONES,
    VERSION_DE_ESQUEMA_DE_CONOCIMIENTO, VERSION_DE_ESQUEMA_DE_SESIONES,
    aplicar_migraciones_de_conocimiento, aplicar_migraciones_de_sesiones,
};
use rusqlite::Connection;

/// Tablas, índices y vistas que el esquema de `sessions.db` debe dejar creados.
const OBJETOS_ESPERADOS: [(&str, &str); 14] = [
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
    ("view", "consumo_por_conversacion"),
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

    aplicar_migraciones_de_sesiones(&conexion).expect("upgrade v1->v3");

    let version: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .expect("user_version");
    assert_eq!(version, VERSION_DE_ESQUEMA_DE_SESIONES);

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

#[test]
fn upgrade_de_version_2_a_version_3_preserva_datos_preexistentes() {
    let directorio = DirectorioTemporal::nuevo("migraciones-upgrade-v2-v3");
    let conexion = Connection::open(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("abrir base");
    conexion
        .execute_batch(include_str!(
            "../migraciones/sesiones/0001-esquema-inicial.sql"
        ))
        .expect("aplicar v1");
    conexion
        .execute_batch(include_str!(
            "../migraciones/sesiones/0002-saldo-y-movimientos.sql"
        ))
        .expect("aplicar v2");
    conexion
        .execute_batch("PRAGMA user_version = 2;")
        .expect("fijar v2");

    conexion
        .execute(
            "INSERT INTO conversaciones (id_conversacion, creada_ms, ultima_actividad_ms) VALUES ('conv1', 100, 200)",
            [],
        )
        .expect("insertar conversacion");
    conexion
        .execute(
            "INSERT INTO reservas (id, id_conversacion, monto_reservado, estado, creada_ms, resuelta_ms) VALUES (1, 'conv1', 10, 'conciliada', 100, 150)",
            [],
        )
        .expect("insertar reserva");
    conexion
        .execute(
            "INSERT INTO movimientos (id, id_reserva, id_conversacion, clase, monto, saldo_resultante, registrado_ms) VALUES (1, 1, 'conv1', 'conciliacion', -10, 0, 150)",
            [],
        )
        .expect("insertar movimiento");

    aplicar_migraciones_de_sesiones(&conexion).expect("upgrade v2->v3");

    let version: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .expect("user_version");
    assert_eq!(version, VERSION_DE_ESQUEMA_DE_SESIONES);

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

    let monto_reservado: i64 = conexion
        .query_row(
            "SELECT monto_reservado FROM reservas WHERE id = 1",
            [],
            |fila| fila.get(0),
        )
        .expect("consultar reserva");
    assert_eq!(monto_reservado, 10);

    let monto_movimiento: i64 = conexion
        .query_row("SELECT monto FROM movimientos WHERE id = 1", [], |fila| {
            fila.get(0)
        })
        .expect("consultar movimiento");
    assert_eq!(monto_movimiento, -10);

    aplicar_migraciones_de_sesiones(&conexion).expect("segundo upgrade v2->v3: no-op");
}

/// Tablas que el esquema de conocimiento en versión 2 debe dejar creadas.
const OBJETOS_ESPERADOS_DE_CONOCIMIENTO: [(&str, &str); 5] = [
    ("table", "metadatos_de_conocimiento"),
    ("table", "documentos"),
    ("table", "fragmentos"),
    ("table", "vectores_de_fragmento"),
    ("table", "metadatos_de_epoca"),
];

#[test]
fn migrar_una_base_de_conocimiento_vacia_crea_el_esquema_completo_y_fija_la_version() {
    let directorio = DirectorioTemporal::nuevo("migcono-vacia");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir una base nueva de conocimiento");

    aplicar_migraciones_de_conocimiento(&conexion)
        .expect("migrar una base de conocimiento vacía debe funcionar");

    for (tipo, nombre) in OBJETOS_ESPERADOS_DE_CONOCIMIENTO {
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
    assert_eq!(version, VERSION_DE_ESQUEMA_DE_CONOCIMIENTO);
}

#[test]
fn todas_las_tablas_de_conocimiento_se_declaran_strict() {
    let directorio = DirectorioTemporal::nuevo("migcono-strict");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir una base nueva de conocimiento");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    let mut sentencia = conexion
        .prepare("SELECT name, strict FROM pragma_table_list WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .expect("preparar la lectura de pragma_table_list");
    let tablas: Vec<(String, i64)> = sentencia
        .query_map([], |fila| Ok((fila.get(0)?, fila.get(1)?)))
        .expect("leer el esquema")
        .map(|fila| fila.expect("una fila del esquema"))
        .collect();

    assert!(
        !tablas.is_empty(),
        "la lista de tablas no puede estar vacía"
    );
    for (nombre, strict) in tablas {
        assert_eq!(strict, 1, "la tabla {nombre} no se declaró STRICT");
    }
}

#[test]
fn upgrade_de_conocimiento_v1_a_v2_preserva_datos_preexistentes_y_reaplica_es_un_noop() {
    let directorio = DirectorioTemporal::nuevo("migcono-upgrade-v1-v2");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");

    conexion
        .execute_batch(include_str!(
            "../migraciones/conocimiento/0001-esquema-minimo.sql"
        ))
        .expect("aplicar v1 de conocimiento");
    conexion
        .execute_batch("PRAGMA user_version = 1;")
        .expect("fijar user_version a 1");

    conexion
        .execute(
            "INSERT INTO metadatos_de_conocimiento (clave, valor) VALUES ('clave_centinela', 'valor_centinela')",
            [],
        )
        .expect("insertar fila preexistente en metadatos_de_conocimiento");

    aplicar_migraciones_de_conocimiento(&conexion).expect("upgrade v1->v2 debe funcionar");

    let version: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .expect("leer user_version tras upgrade");
    assert_eq!(version, VERSION_DE_ESQUEMA_DE_CONOCIMIENTO);

    for (tipo, nombre) in OBJETOS_ESPERADOS_DE_CONOCIMIENTO {
        let encontrados: i64 = conexion
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = ?1 AND name = ?2",
                rusqlite::params![tipo, nombre],
                |fila| fila.get(0),
            )
            .expect("consultar el esquema almacenado");
        assert_eq!(encontrados, 1, "falta objeto {tipo} {nombre} tras upgrade");
    }

    let valor: String = conexion
        .query_row(
            "SELECT valor FROM metadatos_de_conocimiento WHERE clave = 'clave_centinela'",
            [],
            |fila| fila.get(0),
        )
        .expect("la fila preexistente debe seguir ahí tras el upgrade");
    assert_eq!(valor, "valor_centinela");

    aplicar_migraciones_de_conocimiento(&conexion).expect("reaplicar sobre v2 debe ser un no-op");

    let filas_de_epoca: i64 = conexion
        .query_row("SELECT count(*) FROM metadatos_de_epoca", [], |fila| {
            fila.get(0)
        })
        .expect("contar filas de metadatos_de_epoca");
    assert_eq!(filas_de_epoca, 1, "no-op no debe duplicar la fila semilla");
}

#[test]
fn metadatos_de_epoca_contiene_exactamente_una_fila_semilla_con_dimension_768() {
    let directorio = DirectorioTemporal::nuevo("migcono-epoca-semilla");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    let (numero_de_epoca, dimension, sellada_ms): (Option<i64>, i64, Option<i64>) = conexion
        .query_row(
            "SELECT numero_de_epoca, dimension_de_embedding, sellada_ms FROM metadatos_de_epoca WHERE id = 1",
            [],
            |fila| Ok((fila.get(0)?, fila.get(1)?, fila.get(2)?)),
        )
        .expect("leer la fila semilla de metadatos_de_epoca");

    assert!(
        numero_de_epoca.is_none(),
        "numero_de_epoca debe ser NULL en staging: el archivo no ha sido promovido"
    );
    assert_eq!(
        dimension, 768,
        "la dimensión sembrada debe ser 768 valores f32"
    );
    assert!(
        sellada_ms.is_none(),
        "sellada_ms debe ser NULL mientras el archivo esté en staging"
    );

    // Intentar insertar una segunda fila debe fallar por CHECK (id = 1).
    let resultado = conexion.execute(
        "INSERT INTO metadatos_de_epoca (id, numero_de_epoca, dimension_de_embedding, construida_ms, sellada_ms) VALUES (2, NULL, 512, 1000, NULL)",
        [],
    );
    assert!(
        resultado.is_err(),
        "insertar una segunda fila en metadatos_de_epoca debe fallar por CHECK (id = 1)"
    );
}

#[test]
fn check_de_longitud_de_vector_rechaza_blobs_no_multiplos_de_4_y_acepta_los_correctos() {
    let directorio = DirectorioTemporal::nuevo("migcono-blob-check");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'doc-blob', 'Título', 'Contenido de prueba', 1000)",
            [],
        )
        .expect("insertar documento de apoyo");
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, 'Fragmento de prueba')",
            [],
        )
        .expect("insertar fragmento de apoyo");

    // Un BLOB de longitud no múltiplo de 4 (por ejemplo, 5 bytes) debe ser rechazado.
    let blob_invalido = vec![0u8; 5];
    let resultado = conexion.execute(
        "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, ?1)",
        rusqlite::params![blob_invalido],
    );
    assert!(
        resultado.is_err(),
        "un BLOB de longitud 5 (no múltiplo de 4) debe ser rechazado por el CHECK"
    );

    // Un BLOB de 3072 bytes (768 valores f32) debe ser aceptado.
    let blob_valido: Vec<u8> = (0u32..768).flat_map(|i| (i as f32).to_le_bytes()).collect();
    assert_eq!(blob_valido.len(), 3072);
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, ?1)",
            rusqlite::params![blob_valido],
        )
        .expect("un BLOB de 3072 bytes debe ser aceptado");
}

#[test]
fn ida_y_vuelta_de_valores_f32_little_endian_produce_bits_identicos() {
    let directorio = DirectorioTemporal::nuevo("migcono-roundtrip");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'doc-rt', 'Título', 'Contenido', 1000)",
            [],
        )
        .expect("insertar documento");
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, 'Fragmento')",
            [],
        )
        .expect("insertar fragmento");

    // Serie de valores f32 conocidos, incluidos casos de borde: cero, uno, negativos, NaN canónico.
    let originales: Vec<f32> = vec![0.0, 1.0, -1.0, 3.14159, f32::MAX, f32::MIN_POSITIVE];
    let blob: Vec<u8> = originales.iter().flat_map(|v| v.to_le_bytes()).collect();

    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, ?1)",
            rusqlite::params![blob],
        )
        .expect("insertar vector con valores conocidos");

    let blob_recuperado: Vec<u8> = conexion
        .query_row(
            "SELECT vector FROM vectores_de_fragmento WHERE id_fragmento = 1",
            [],
            |fila| fila.get(0),
        )
        .expect("leer el vector almacenado");

    let recuperados: Vec<f32> = blob_recuperado
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("trozo de 4 bytes")))
        .collect();

    assert_eq!(recuperados.len(), originales.len());
    for (indice, (original, recuperado)) in originales.iter().zip(recuperados.iter()).enumerate() {
        assert_eq!(
            original.to_bits(),
            recuperado.to_bits(),
            "el valor f32 en posición {indice} no sobrevivió la ida y vuelta bit a bit"
        );
    }
}

#[test]
fn integridad_referencial_rechaza_fragmentos_sin_documento_y_cascadea_el_borrado() {
    let directorio = DirectorioTemporal::nuevo("migcono-fk");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");
    // Las claves foráneas deben activarse explícitamente: Connection::open las deja desactivadas.
    conexion
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("activar claves foráneas");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    let resultado = conexion.execute(
        "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 999, 0, 'Fragmento huérfano')",
        [],
    );
    assert!(
        resultado.is_err(),
        "un fragmento sin documento debe ser rechazado por la clave foránea"
    );

    let resultado = conexion.execute(
        "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (999, X'00000000')",
        [],
    );
    assert!(
        resultado.is_err(),
        "un vector sin fragmento debe ser rechazado por la clave foránea"
    );

    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'doc-fk', 'Título', 'Contenido', 1000)",
            [],
        )
        .expect("insertar documento para prueba de cascada");
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, 'Fragmento')",
            [],
        )
        .expect("insertar fragmento para prueba de cascada");
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, X'00000000')",
            [],
        )
        .expect("insertar vector para prueba de cascada");

    conexion
        .execute("DELETE FROM documentos WHERE id = 1", [])
        .expect("borrar el documento debe funcionar");

    let fragmentos_restantes: i64 = conexion
        .query_row(
            "SELECT count(*) FROM fragmentos WHERE id_documento = 1",
            [],
            |fila| fila.get(0),
        )
        .expect("contar fragmentos tras el borrado");
    assert_eq!(
        fragmentos_restantes, 0,
        "el borrado del documento debe cascadear a sus fragmentos"
    );

    let vectores_restantes: i64 = conexion
        .query_row(
            "SELECT count(*) FROM vectores_de_fragmento WHERE id_fragmento = 1",
            [],
            |fila| fila.get(0),
        )
        .expect("contar vectores tras el borrado");
    assert_eq!(
        vectores_restantes, 0,
        "el borrado del fragmento debe cascadear a su vector"
    );
}

#[test]
fn consulta_de_deteccion_de_dimension_inconsistente_identifica_el_fragmento_discrepante() {
    // Este test documenta la costura diferida: el esquema no impide BLOBs de longitudes
    // distintas en la misma época, pero la consulta que el validador de la tarea 5 usará
    // detecta el fragmento discrepante comparando length(vector) con 4 * dimension_de_embedding.
    let directorio = DirectorioTemporal::nuevo("migcono-deteccion-dim");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");
    conexion
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("activar claves foráneas");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'doc-dim', 'Título', 'Contenido', 1000)",
            [],
        )
        .expect("insertar documento");

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, 'Fragmento con dimensión correcta')",
            [],
        )
        .expect("insertar fragmento 1");
    let blob_correcto = vec![0u8; 768 * 4];
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, ?1)",
            rusqlite::params![blob_correcto],
        )
        .expect("insertar vector correcto");

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (2, 1, 1, 'Fragmento con dimensión discrepante')",
            [],
        )
        .expect("insertar fragmento 2");
    let blob_discrepante = vec![0u8; 256 * 4];
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (2, ?1)",
            rusqlite::params![blob_discrepante],
        )
        .expect("el esquema debe aceptar el BLOB aunque la dimensión sea distinta");

    let mut sentencia = conexion
        .prepare(
            "SELECT vf.id_fragmento
             FROM vectores_de_fragmento AS vf
             WHERE length(vf.vector) <> 4 * (SELECT dimension_de_embedding FROM metadatos_de_epoca)",
        )
        .expect("preparar la consulta de detección de dimensión inconsistente");
    let discrepantes: Vec<i64> = sentencia
        .query_map([], |fila| fila.get(0))
        .expect("ejecutar la consulta de detección")
        .map(|fila| fila.expect("una fila del resultado"))
        .collect();

    assert_eq!(
        discrepantes,
        vec![2i64],
        "la consulta de detección debe identificar exactamente el fragmento con dimensión discrepante"
    );
}

#[test]
fn consulta_de_vitalidad_de_conocimiento_sigue_funcionando_en_version_2() {
    // La sonda de vitalidad en pools.rs usa "SELECT count(*) FROM metadatos_de_conocimiento".
    // Esta tabla debe sobrevivir intacta a la migración de versión 2.
    let directorio = DirectorioTemporal::nuevo("migcono-vitalidad");
    let ruta = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let conexion = Connection::open(&ruta).expect("abrir base de conocimiento");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar debe funcionar");

    let cuenta: i64 = conexion
        .query_row(
            "SELECT count(*) FROM metadatos_de_conocimiento",
            [],
            |fila| fila.get(0),
        )
        .expect("la consulta de vitalidad debe funcionar en una base de versión 2");

    assert_eq!(
        cuenta, 0,
        "metadatos_de_conocimiento debe existir y estar vacía en una base recién migrada"
    );
}
