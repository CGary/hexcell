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

    Ok(CopiaVerificada {
        nombre_logico,
        ruta: destino.to_path_buf(),
        bytes,
    })
}
