//! Ayudas compartidas por los tests de esta capa.
//!
//! Cada test que necesita bases de datos crea **su propio** directorio temporal y lo borra al
//! salir de alcance. Ninguna ruta es fija ni compartida: `cargo test` corre los tests de un mismo
//! binario en hilos distintos del mismo proceso, y dos tests que abrieran la misma `sessions.db`
//! se pisarían de una forma que depende del orden de planificación.
//!
//! No se usa ningún crate de directorios temporales a propósito: `crates/hexcell/tests/` ya
//! construía los suyos con `temp_dir()` y `process::id()` desde HEX-004, y esta ayuda extiende ese
//! patrón en vez de introducir una segunda manera de hacer lo mismo.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use rusqlite::Connection;

/// Distingue dos directorios creados por el mismo proceso: `process::id()` solo separa procesos.
static SECUENCIA: AtomicUsize = AtomicUsize::new(0);

/// Directorio temporal propio de un test, borrado al salir de alcance.
pub struct DirectorioTemporal {
    ruta: PathBuf,
}

impl DirectorioTemporal {
    /// Crea un directorio temporal único para este test.
    pub fn nuevo(etiqueta: &str) -> Self {
        let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
        let ruta = std::env::temp_dir().join(format!(
            "hexcell-storage-{etiqueta}-{}-{secuencia}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&ruta);
        std::fs::create_dir_all(&ruta).expect("crear el directorio temporal del test");
        Self { ruta }
    }

    /// Ruta del directorio, para pasársela a `GestorDePools::abrir`.
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }
}

impl Drop for DirectorioTemporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.ruta);
    }
}

/// Prepara un archivo de base de datos en staging válido con un único fragmento para conmutaciones.
///
/// Esta función centraliza el patrón repetido en `tests/promocion.rs` y `tests/drenaje.rs` para
/// fabricar un staging que supera todas las compuertas de integridad. La variante de texto del
/// fragmento es genérica en español para que ningún test dependa de su contenido concreto: la
/// validación semántica solo necesita que el vector del fragmento coincida con el vector de la
/// sonda (similitud coseno = 1.0), no que el texto diga algo específico.
pub fn preparar_staging_valido(ruta_datos: &Path, dimension: usize) -> ConfiguracionDeFragmentacion {
    let ruta_staging = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir base de staging");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    // La ingesta real siempre abre staging con `abrir_lectura_escritura`, que fija el modo WAL
    // desde la primera conexión de escritura. Replicarlo aquí evita que un test que sostiene un
    // lector concurrente choque con un cambio de modo de diario (delete -> wal) que sí exige
    // exclusividad, en vez de con el punto de control que es lo que ese test quiere ejercitar.
    conexion
        .query_row("PRAGMA journal_mode = WAL", [], |fila| {
            fila.get::<_, String>(0)
        })
        .unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging");

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = ?1 WHERE id = 1",
            rusqlite::params![dimension as i64],
        )
        .unwrap();

    let texto = "Fragmento de conocimiento para pruebas de conmutación de época.";
    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_1', 'Titulo 1', ?1, 1000)",
            rusqlite::params![texto],
        )
        .unwrap();

    let vector = vec![1.0f32; dimension];
    let vector_bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, ?1)",
            rusqlite::params![texto],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, ?1)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO sonda_semantica (id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms) VALUES (1, 'consulta', ?1, 0.5, 1000)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();

    drop(conexion);

    ConfiguracionDeFragmentacion {
        tamano_de_fragmento: texto.chars().count(),
        solapamiento: 0,
    }
}
