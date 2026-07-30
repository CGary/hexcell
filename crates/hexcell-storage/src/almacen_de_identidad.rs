//! Almacén de identidad del adaptador: mapa opaco entre un contacto y su identificador interno.
//!
//! `adr-0010`, puntos 5 y 6: el mapeo entre el identificador de transporte y el identificador
//! interno de conversación pertenece al **adaptador**, no al núcleo, y persiste en un almacén
//! **propio del adaptador**, separado del `sqlstore` de las credenciales de sesión del transporte.
//! El motivo es concreto: la rama `LoggedOut` con `device_removed` obliga a descartar el
//! `sqlstore`, y este mapa tiene que sobrevivir exactamente a ese momento para que cada contacto
//! siga cayendo en el hilo que ya tenía.
//!
//! Verificado el 2026-07-30: antes de esta tarea ese mapa era un mapa en memoria, campo de
//! `EstadoInterno` en `crates/hexcell-canal-simulado/src/adaptador.rs`, sin ningún archivo
//! detrás. Esta base lo
//! materializa como la tercera de las cuatro que el respaldo trata (`sessions.db`,
//! `knowledge_live.db`, este almacén y el `sqlstore`), abierta y migrada con el mismo mecanismo
//! que las otras dos.
//!
//! # Por qué este módulo nunca nombra un identificador de conversación
//!
//! Las dos columnas son texto opaco: `contacto` como clave primaria, `identificador_interno` como
//! valor. La API entera habla en `String`, nunca en el tipo del dominio que traduce ese valor: si
//! esta capa construyera ese tipo estaría duplicando la traducción que `adr-0010` le asigna al
//! adaptador, y dos piezas que traducen lo mismo divergen sin que nadie lo note hasta que ambas
//! ya han escrito datos. Acuñar el valor —decidir cuál es el siguiente identificador y a partir de
//! qué contador— sigue siendo responsabilidad exclusiva de
//! `hexcell_canal_simulado::AdaptadorSimulado::inyectar_desde_contacto`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{OptionalExtension, params};

use crate::error::ErrorDeAlmacen;
use crate::migraciones::{VERSION_DE_ESQUEMA_DE_IDENTIDAD, aplicar_migraciones_de_identidad};
use crate::pools::{abrir_lectura_escritura, abrir_solo_lectura};
use crate::respaldo::{self, CopiaVerificada};

/// Nombre del archivo del almacén de identidad del adaptador dentro de la ruta de datos.
pub const NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR: &str = "adapter_identity.db";

/// Almacén de identidad del adaptador, abierto como `sessions.db`: una conexión de escritura y
/// una de lectura, cada una tras su propio cerrojo, ambas con los mismos parámetros de conexión
/// (`busy_timeout`, `synchronous`, `foreign_keys`) que las demás bases de la célula.
pub struct AlmacenDeIdentidad {
    ruta: PathBuf,
    escritura: Mutex<rusqlite::Connection>,
    lectura: Mutex<rusqlite::Connection>,
}

impl AlmacenDeIdentidad {
    /// Abre y migra el almacén a partir de la ruta de datos ya validada de la célula.
    pub fn abrir(ruta_datos: &Path) -> Result<Self, ErrorDeAlmacen> {
        let ruta = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR);
        let escritura = abrir_lectura_escritura(&ruta)?;
        aplicar_migraciones_de_identidad(&escritura)?;
        let lectura = abrir_solo_lectura(&ruta)?;

        Ok(Self {
            ruta,
            escritura: Mutex::new(escritura),
            lectura: Mutex::new(lectura),
        })
    }

    /// Ruta del archivo que respalda este almacén.
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }

    /// Busca el identificador interno ya registrado para `contacto`, si lo hay.
    pub fn buscar(&self, contacto: &str) -> Result<Option<String>, ErrorDeAlmacen> {
        let conexion = bloquear(&self.lectura);
        conexion
            .query_row(
                "SELECT identificador_interno FROM identidades_de_contacto WHERE contacto = ?1",
                params![contacto],
                |fila| fila.get(0),
            )
            .optional()
            .map_err(ErrorDeAlmacen::en("buscar la identidad de un contacto"))
    }

    /// Registra `contacto` con `identificador_interno`, de forma idempotente: si el contacto ya
    /// tenía un identificador registrado, esta llamada no lo cambia.
    pub fn registrar(
        &self,
        contacto: &str,
        identificador_interno: &str,
    ) -> Result<(), ErrorDeAlmacen> {
        let conexion = bloquear(&self.escritura);
        conexion
            .execute(
                "INSERT OR IGNORE INTO identidades_de_contacto \
                 (contacto, identificador_interno) VALUES (?1, ?2)",
                params![contacto, identificador_interno],
            )
            .map_err(ErrorDeAlmacen::en("registrar la identidad de un contacto"))?;
        Ok(())
    }

    /// Cuenta de contactos registrados hasta ahora.
    ///
    /// El adaptador la usa para acuñar el siguiente identificador a partir de este contador, no
    /// del propio nombre del contacto: eso hace que el identificador dependa del **orden** en que
    /// cada contacto se vio por primera vez, y es lo que impide que un almacén vacío reproduzca
    /// por accidente el mismo identificador que uno restaurado de verdad.
    pub fn contactos_registrados(&self) -> Result<i64, ErrorDeAlmacen> {
        let conexion = bloquear(&self.lectura);
        conexion
            .query_row("SELECT count(*) FROM identidades_de_contacto", [], |fila| {
                fila.get(0)
            })
            .map_err(ErrorDeAlmacen::en("contar los contactos registrados"))
    }

    /// Respalda en caliente este almacén sobre un directorio existente, bajo su nombre canónico.
    ///
    /// Usa su propia conexión de solo lectura, nunca la de escritura: el mismo criterio que
    /// `GestorDePools::respaldar_en` aplica a `sessions.db` y a `knowledge_live.db`.
    pub fn respaldar_en(&self, directorio: &Path) -> Result<CopiaVerificada, ErrorDeAlmacen> {
        let destino = directorio.join(NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR);
        let conexion = bloquear(&self.lectura);
        respaldo::respaldar_base(
            &conexion,
            &destino,
            VERSION_DE_ESQUEMA_DE_IDENTIDAD,
            NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
        )
    }
}

/// Toma el cerrojo, recuperando el contenido si estaba envenenado por un pánico de otro hilo: la
/// conexión sigue siendo válida y SQLite deshace sola cualquier transacción a medias, igual que
/// hacen ya `PoolDeSesiones` y `PoolDeConocimiento`.
fn bloquear(
    cerrojo: &Mutex<rusqlite::Connection>,
) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
    match cerrojo.lock() {
        Ok(guardian) => guardian,
        Err(envenenado) => envenenado.into_inner(),
    }
}
