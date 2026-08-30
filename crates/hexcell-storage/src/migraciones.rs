//! Migraciones versionadas con `PRAGMA user_version`.
//!
//! # Por qué no hay ningún crate de migraciones
//!
//! SQLite ya guarda un entero de 32 bits en la cabecera del archivo, `user_version`, que ninguna
//! otra parte del motor usa y que cambia **dentro de la misma transacción** que el esquema. Un
//! crate de migraciones añadiría una tabla de versiones que duplica exactamente ese dato, con la
//! diferencia de que la tabla puede quedar desincronizada del esquema y la cabecera no.
//!
//! # Por qué el corredor es una escalera de pasos
//!
//! En lugar de aplicar un único guion monolítico que solo alcance una versión fija, el corredor
//! recorre una secuencia ordenada de pasos (`PasoDeMigracion`). Para cada paso cuya versión sea
//! estrictamente mayor que la `user_version` actual de la base de datos, se ejecuta su guion SQL y
//! se incrementa la `user_version` a la de dicho paso en la **misma** transacción. Esto permite que
//! bases de datos en versiones intermedias (por ejemplo, versión 1) se actualicen a versiones más
//! recientes (por ejemplo, versión 2) aplicando únicamente los pasos faltantes.
//!
//! # Por qué el guion SQL viaja dentro del binario
//!
//! Los `.sql` viven en `crates/hexcell-storage/migraciones/` y entran por `include_str!`: se leen
//! como SQL en el repositorio —revisables, con su propio historial— y a la vez no crean ninguna
//! dependencia de archivos en tiempo de ejecución, que en la imagen mínima de la etapa A-6 sería
//! un modo de fallo nuevo (el binario arrancaría y moriría al primer arranque por un archivo que
//! nadie copió).
//!
//! Volver a aplicar una migración sobre una base ya migrada es una operación **nula** que devuelve
//! `Ok`: es el caso normal, porque cada arranque de la célula la ejecuta.

use rusqlite::Connection;

use crate::error::ErrorDeAlmacen;

/// Versión de esquema que este binario espera encontrar en `sessions.db`.
///
/// La versión 4 flexibiliza la tabla `reservas` haciendo que `id_conversacion` sea nullable para
/// dar soporte a las reservas de presupuesto de ingestas de catálogo (las cuales no tienen una
/// conversación asociada). También filtra `consumo_por_conversacion` para omitir registros con
/// conversación nula, añade la vista `consumo_de_ingesta` para agruparlos y aplica una compuerta de
/// integridad en la migración `0004-reservas-sin-conversacion.sql`.
pub const VERSION_DE_ESQUEMA_DE_SESIONES: i64 = 4;

/// Versión de esquema que este binario espera encontrar en `knowledge_live.db`.
///
/// La versión 3 añade la tabla singleton `sonda_semantica` para almacenar el vector y el
/// umbral de aceptación requeridos para la validación fuera de línea de la época, conforme
/// a la migración `0003-sonda-semantica.sql`.
pub const VERSION_DE_ESQUEMA_DE_CONOCIMIENTO: i64 = 3;

/// Versión de esquema que este binario espera encontrar en `adapter_identity.db`.
///
/// Base propia del adaptador (`adr-0010`, puntos 5 y 6), no del núcleo: esta capa la abre y la
/// migra con el mismo mecanismo que las otras dos, pero no construye ni interpreta ningún
/// identificador de conversación al hacerlo.
pub const VERSION_DE_ESQUEMA_DE_IDENTIDAD: i64 = 1;

const ESQUEMA_INICIAL_DE_SESIONES: &str =
    include_str!("../migraciones/sesiones/0001-esquema-inicial.sql");

const ESQUEMA_SALDO_Y_MOVIMIENTOS_DE_SESIONES: &str =
    include_str!("../migraciones/sesiones/0002-saldo-y-movimientos.sql");

const ESQUEMA_CONSUMO_POR_CONVERSACION_DE_SESIONES: &str =
    include_str!("../migraciones/sesiones/0003-consumo-por-conversacion.sql");

const ESQUEMA_RESERVAS_SIN_CONVERSACION_DE_SESIONES: &str =
    include_str!("../migraciones/sesiones/0004-reservas-sin-conversacion.sql");

const ESQUEMA_MINIMO_DE_CONOCIMIENTO: &str =
    include_str!("../migraciones/conocimiento/0001-esquema-minimo.sql");

const ESQUEMA_DE_CONOCIMIENTO: &str =
    include_str!("../migraciones/conocimiento/0002-esquema-de-conocimiento.sql");

const ESQUEMA_DE_SONDA_SEMANTICA: &str =
    include_str!("../migraciones/conocimiento/0003-sonda-semantica.sql");

const ESQUEMA_INICIAL_DE_IDENTIDAD: &str =
    include_str!("../migraciones/identidad/0001-esquema-inicial.sql");

struct PasoDeMigracion {
    version: i64,
    guion: &'static str,
}

const MIGRACIONES_DE_SESIONES: &[PasoDeMigracion] = &[
    PasoDeMigracion {
        version: 1,
        guion: ESQUEMA_INICIAL_DE_SESIONES,
    },
    PasoDeMigracion {
        version: 2,
        guion: ESQUEMA_SALDO_Y_MOVIMIENTOS_DE_SESIONES,
    },
    PasoDeMigracion {
        version: 3,
        guion: ESQUEMA_CONSUMO_POR_CONVERSACION_DE_SESIONES,
    },
    PasoDeMigracion {
        version: 4,
        guion: ESQUEMA_RESERVAS_SIN_CONVERSACION_DE_SESIONES,
    },
];

const MIGRACIONES_DE_CONOCIMIENTO: &[PasoDeMigracion] = &[
    PasoDeMigracion {
        version: 1,
        guion: ESQUEMA_MINIMO_DE_CONOCIMIENTO,
    },
    PasoDeMigracion {
        version: 2,
        guion: ESQUEMA_DE_CONOCIMIENTO,
    },
    PasoDeMigracion {
        version: 3,
        guion: ESQUEMA_DE_SONDA_SEMANTICA,
    },
];

const MIGRACIONES_DE_IDENTIDAD: &[PasoDeMigracion] = &[PasoDeMigracion {
    version: 1,
    guion: ESQUEMA_INICIAL_DE_IDENTIDAD,
}];

/// Lleva `sessions.db` hasta [`VERSION_DE_ESQUEMA_DE_SESIONES`].
///
/// La conexión debe estar abierta en lectura y escritura.
pub fn aplicar_migraciones_de_sesiones(conexion: &Connection) -> Result<(), ErrorDeAlmacen> {
    aplicar(
        conexion,
        MIGRACIONES_DE_SESIONES,
        "migrar el esquema de sessions.db",
    )
}

/// Lleva `knowledge_live.db` hasta [`VERSION_DE_ESQUEMA_DE_CONOCIMIENTO`].
///
/// La conexión debe estar abierta en lectura y escritura: es la única ocasión en que la célula
/// escribe esa base, justo antes de reabrirla en solo lectura para servir producción.
pub fn aplicar_migraciones_de_conocimiento(conexion: &Connection) -> Result<(), ErrorDeAlmacen> {
    aplicar(
        conexion,
        MIGRACIONES_DE_CONOCIMIENTO,
        "migrar el esquema de knowledge_live.db",
    )
}

/// Lleva `adapter_identity.db` hasta [`VERSION_DE_ESQUEMA_DE_IDENTIDAD`].
///
/// La conexión debe estar abierta en lectura y escritura. Vive en este módulo y no en
/// `almacen_de_identidad` para que las tres bases de la célula compartan el mismo mecanismo de
/// migración versionada, ya justificado arriba.
pub fn aplicar_migraciones_de_identidad(conexion: &Connection) -> Result<(), ErrorDeAlmacen> {
    aplicar(
        conexion,
        MIGRACIONES_DE_IDENTIDAD,
        "migrar el esquema de adapter_identity.db",
    )
}

/// Lee la versión de la cabecera y recorre la escalera de pasos. Para cada paso cuya versión
/// sea estrictamente mayor a la actual, aplica su guion y sube la versión en la **misma**
/// transacción: o quedan las dos cosas, o no queda ninguna. Si el archivo quedase con
/// el esquema aplicado y la versión antigua, el arranque siguiente reintentaría el `CREATE TABLE`
/// y fallaría para siempre.
fn aplicar(
    conexion: &Connection,
    pasos: &[PasoDeMigracion],
    operacion: &'static str,
) -> Result<(), ErrorDeAlmacen> {
    let version_actual: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .map_err(ErrorDeAlmacen::en(
            "leer la versión de esquema (user_version)",
        ))?;

    for paso in pasos {
        if version_actual >= paso.version {
            continue;
        }

        let transaccion = conexion
            .unchecked_transaction()
            .map_err(ErrorDeAlmacen::en(operacion))?;

        transaccion
            .execute_batch(paso.guion)
            .map_err(ErrorDeAlmacen::en(operacion))?;

        // `PRAGMA` no admite parámetros ligados, así que la versión se interpola con `format!`. El
        // valor interpolado es **siempre** una constante entera de este crate y nunca llega de fuera:
        // esa es la única razón por la que la interpolación es aceptable aquí.
        transaccion
            .execute_batch(&format!("PRAGMA user_version = {};", paso.version))
            .map_err(ErrorDeAlmacen::en("fijar la versión de esquema"))?;

        transaccion
            .commit()
            .map_err(ErrorDeAlmacen::en("confirmar la migración de esquema"))?;
    }

    Ok(())
}
