//! Copia de respaldo en caliente de una base SQLite, con `VACUUM INTO`.
//!
//! # Por qué `VACUUM INTO` y no la API de respaldo en línea de `rusqlite`
//!
//! La API de respaldo en línea de `rusqlite` reinicia su copia cada vez que un escritor confirma
//! una transacción, así que bajo un escritor activo puede no llegar nunca a terminar. `VACUUM
//! INTO` toma una única instantánea de lectura y no necesita activar ninguna característica
//! adicional de `rusqlite`; el descarte razonado vive en `docs/bitacora-de-descartes.md` (D-19).
//!
//! # Tres hechos de `VACUUM INTO`, comprobados el 2026-07-30 contra `sqlite3` 3.53.4
//!
//! * **Funciona sobre una conexión de solo lectura** y la copia resultante supera
//!   `integrity_check`; es una lectura, al contrario que `PRAGMA wal_checkpoint`, que HEX-007 ya
//!   comprobó que falla con un error de E/S sobre ese mismo tipo de conexión.
//! * **Rechaza un destino que ya existe** (`output file already exists`) y **rechaza un destino
//!   cuyo directorio padre no existe** (`unable to open database`). El primero es una ventaja, no
//!   un obstáculo: hace imposible sobrescribir por accidente una ronda de respaldo anterior.
//! * **No puede ejecutarse dentro de una transacción abierta**, así que esta función la lanza
//!   siempre en modo `autocommit`, nunca dentro de una transacción explícita de `rusqlite`.
//!
//! `PRAGMA user_version` se conserva en la copia (comprobado el mismo día), lo que permite que
//! [`verificar_copia`] compare la versión de la copia contra la que el llamante espera.
//!
//! # Por qué la ruta va como parámetro ligado
//!
//! Comprobado el 2026-07-30: `VACUUM INTO ?1` acepta un parámetro ligado. Interpolar la ruta de
//! destino con `format!` sería el único punto de este crate donde un valor externo llegaría a una
//! sentencia como texto —`crates/hexcell-storage/src/migraciones.rs` solo interpola una constante
//! entera del propio crate—, así que aquí se liga.
//!
//! # Por qué la copia sale en `journal_mode = delete` y no es un problema
//!
//! Comprobado el mismo día: el archivo que produce `VACUUM INTO` queda en modo `delete` aunque el
//! origen esté en WAL. Se autocura al restaurar, porque
//! [`crate::pools::abrir_lectura_escritura`] (usada tanto por `GestorDePools::abrir` como por
//! [`crate::almacen_de_identidad::AlmacenDeIdentidad::abrir`]) fija `PRAGMA journal_mode = WAL` en
//! cada apertura de lectura y escritura. Ningún código de este módulo compara el modo de diario de
//! una copia recién hecha, y ninguno debería tratarlo como señal de corrupción.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, params};

use crate::error::ErrorDeAlmacen;

/// Copia de respaldo ya verificada de una base.
#[derive(Clone, Debug)]
pub struct CopiaVerificada {
    /// Nombre lógico de la base copiada (su nombre de archivo canónico), para que quien agregue
    /// varias copias sepa cuál es cuál sin volver a abrir ningún archivo.
    pub nombre_logico: &'static str,
    /// Ruta completa de la copia ya escrita y verificada.
    pub ruta: PathBuf,
    /// Tamaño en bytes de la copia.
    pub bytes: u64,
    /// Número ordinal de la época copiada para las bases de conocimiento, `None` cuando la base
    /// no modela épocas (`sessions.db`, `adapter_identity.db`) o cuando la base de conocimiento
    /// nunca fue promovida (`metadatos_de_epoca.numero_de_epoca` es NULL, situación documentada
    /// en `conocimiento.rs:313`). El número se lee de la copia producida, **no** del pool vivo:
    /// para `knowledge_live.db` la ruta del pool es el symlink `<datos>/knowledge_live.db`, que
    /// repunta a la época nueva en el instante de la conmutación y haría mentir a cualquier
    /// etiqueta derivada de la ruta.
    pub numero_de_epoca: Option<i64>,
}

/// Comprueba que un destino de respaldo está disponible **antes** de ejecutar ningún `VACUUM
/// INTO`: ni el archivo existe ya, ni falta su directorio padre.
///
/// Se expone aparte de [`respaldar_base`] para que quien orqueste varias copias en una misma
/// ronda —[`crate::pools::GestorDePools::respaldar_en`], y el binario de la célula sobre las tres
/// bases— pueda comprobar **todos** los destinos antes de tomar la primera copia, y así no dejar
/// ninguna a medias si el segundo o el tercero ya estaban ocupados.
pub fn verificar_destino_disponible(destino: &Path) -> Result<(), ErrorDeAlmacen> {
    if destino.exists() {
        return Err(ErrorDeAlmacen::DestinoDeRespaldoOcupado {
            ruta: destino.to_path_buf(),
        });
    }
    let directorio_padre_valido = destino.parent().is_some_and(Path::is_dir);
    if !directorio_padre_valido {
        return Err(ErrorDeAlmacen::DirectorioDeRespaldoInaccesible {
            ruta: destino.to_path_buf(),
        });
    }
    Ok(())
}

/// Ejecuta `VACUUM INTO` sobre `conexion` hacia `destino` y verifica la copia resultante.
///
/// `conexion` debe ser una conexión que el proceso ya tiene abierta sobre la base de origen —de
/// lectura, nunca de escritura, ver la nota de [`crate::pools::GestorDePools::respaldar_en`]— y
/// `destino` debe apuntar a un archivo que todavía no existe, dentro de un directorio que sí. La
/// verificación comprueba, sobre una conexión de solo lectura recién abierta a la copia, que
/// `PRAGMA integrity_check` responde `ok` y que `PRAGMA user_version` coincide con
/// `version_esperada`; cualquiera de las dos cosas que falle es [`ErrorDeAlmacen::CopiaCorrupta`],
/// nunca un aviso.
pub fn respaldar_base(
    conexion: &Connection,
    destino: &Path,
    version_esperada: i64,
    nombre_logico: &'static str,
) -> Result<CopiaVerificada, ErrorDeAlmacen> {
    verificar_destino_disponible(destino)?;

    let destino_como_texto = destino.to_string_lossy().into_owned();
    conexion
        .execute("VACUUM INTO ?1", params![destino_como_texto])
        .map_err(ErrorDeAlmacen::en("ejecutar VACUUM INTO"))?;

    verificar_copia(destino, version_esperada, nombre_logico)
}

/// Lee el número ordinal de la época desde la fila singleton `metadatos_de_epoca`.
///
/// Devuelve `Ok(None)` en los dos casos legítimos en los que no hay número que reportar y que
/// distinguen las bases sin épocas (`sessions.db`, `adapter_identity.db`: la tabla no existe)
/// de una base de conocimiento que aún no fue promovida (`metadatos_de_epoca.numero_de_epoca`
/// es NULL, situación documentada en `conocimiento.rs:313`). Cualquier otro fallo de SQLite se
/// propaga como error: copiar una base cuyo metadato de época existe pero es ilegible no es
/// un caso normal y debe parar el respaldo.
fn leer_numero_de_epoca_de_la_copia(destino: &Path) -> Result<Option<i64>, ErrorDeAlmacen> {
    let conexion = Connection::open_with_flags(
        destino,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(ErrorDeAlmacen::en(
        "abrir la copia de respaldo para leer su número de época",
    ))?;

    // `prepare` falla con `SqliteFailure(SQLITE_ERROR, "no such table ...")` cuando la tabla
    // no existe; `query_row` con `QueryReturnedNoRows` cuando la fila no está. Ambos se traducen
    // a `Ok(None)` y son los dos casos documentados arriba: bases sin épocas y bases nunca
    // promovidas, respectivamente. `SQLITE_ERROR` se compara por código extendido porque es el
    // código que SQLite emite para «no such table», y `ErrorCode` en `rusqlite` 0.39 lo modela
    // como entero extendido sin variante con nombre para ese caso.
    let mut sentencia =
        match conexion.prepare("SELECT numero_de_epoca FROM metadatos_de_epoca WHERE id = 1") {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(causa, mensaje)) => {
                if causa.extended_code == rusqlite::ffi::SQLITE_ERROR {
                    return Ok(None);
                }
                let mensaje = mensaje.as_deref().unwrap_or("");
                if mensaje.to_lowercase().contains("no such table") {
                    return Ok(None);
                }
                return Err(ErrorDeAlmacen::en(
                    "preparar la lectura del número de época de la copia",
                )(rusqlite::Error::SqliteFailure(
                    causa,
                    Some(mensaje.to_string()),
                )));
            }
            Err(causa) => {
                return Err(ErrorDeAlmacen::en(
                    "preparar la lectura del número de época de la copia",
                )(causa));
            }
        };

    let resultado = sentencia
        .query_row([], |fila| fila.get::<_, Option<i64>>(0))
        .map_err(ErrorDeAlmacen::en(
            "leer el número de época de la copia de respaldo",
        ))?;

    Ok(resultado)
}

/// Abre la copia ya escrita en solo lectura y comprueba su integridad y su versión de esquema.
fn verificar_copia(
    destino: &Path,
    version_esperada: i64,
    nombre_logico: &'static str,
) -> Result<CopiaVerificada, ErrorDeAlmacen> {
    let conexion = Connection::open_with_flags(
        destino,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(ErrorDeAlmacen::en(
        "abrir la copia de respaldo para verificarla",
    ))?;

    let integridad: String = conexion
        .query_row("PRAGMA integrity_check", [], |fila| fila.get(0))
        .map_err(ErrorDeAlmacen::en(
            "ejecutar integrity_check sobre la copia",
        ))?;
    if integridad != "ok" {
        return Err(ErrorDeAlmacen::CopiaCorrupta {
            ruta: destino.to_path_buf(),
            motivo: format!("integrity_check devolvió «{integridad}» en vez de «ok»"),
        });
    }

    let version_real: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .map_err(ErrorDeAlmacen::en("leer user_version de la copia"))?;
    if version_real != version_esperada {
        return Err(ErrorDeAlmacen::CopiaCorrupta {
            ruta: destino.to_path_buf(),
            motivo: format!("user_version esperado {version_esperada}, encontrado {version_real}"),
        });
    }

    // La conexión de verificación se cierra al salir de alcance, antes de medir el archivo: así
    // el tamaño reportado es el definitivo, sin ninguna escritura de SQLite todavía pendiente.
    drop(conexion);

    let bytes = std::fs::metadata(destino)
        .map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: destino.to_path_buf(),
            causa,
        })?
        .len();

    // El número de época se lee **de la copia producida** y no del pool vivo. Para el pool vivo
    // la ruta es `<datos>/knowledge_live.db`, un symlink que `reasignar_enlace_de_la_epoca_viva`
    // repunta a la época nueva en el instante de la conmutación; una etiqueta derivada de la
    // ruta mentiría sobre el contenido físico que esta misma copia acaba de escribir.
    let numero_de_epoca = leer_numero_de_epoca_de_la_copia(destino)?;

    Ok(CopiaVerificada {
        nombre_logico,
        ruta: destino.to_path_buf(),
        bytes,
        numero_de_epoca,
    })
}
