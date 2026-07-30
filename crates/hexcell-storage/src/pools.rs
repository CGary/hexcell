//! Pools duales de SQLite: `sessions.db` en lectura y escritura, `knowledge_live.db` en solo
//! lectura, con sonda de vitalidad por pool.
//!
//! Esta es la persistencia dual de FR-05 (`docs/adr/adr-0003-persistencia-dual.md`). La separación
//! no es organizativa: las dos bases tienen patrones de acceso opuestos —una se escribe en el
//! camino caliente de cada mensaje, la otra se lee y no se escribe nunca en producción— y
//! juntarlas obligaría a que el conocimiento se bloqueara detrás del escritor de sesiones.
//!
//! # Tamaño de los pools
//!
//! `sessions.db` recibe **una** conexión de escritura y **una** de lectura. La de escritura es una
//! sola porque SQLite serializa a los escritores por diseño: N conexiones de escritura no
//! escribirían en paralelo, solo se estorbarían y producirían `SQLITE_BUSY` donde antes había
//! espera ordenada dentro del proceso. La de lectura está separada de ella para que una consulta
//! de historial no tenga que esperar detrás de la escritura en curso, que es lo que WAL permite.
//!
//! `knowledge_live.db` recibe [`CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO`] conexiones de solo
//! lectura, repartidas por turno rotatorio. Dos y no más: el hardware objetivo es un i7 de diez
//! años con 8 GB de RAM compartidos entre todas las células, cada conexión paga su propia caché de
//! páginas, y una célula sirve tráfico conversacional bajo. El turno rotatorio se implementa con
//! un contador atómico y `Vec<Mutex<Connection>>` en vez de con un canal de conexiones libres
//! porque no hay nada que gestionar —el conjunto es fijo y no crece ni se recicla—, y un canal
//! añadiría un modo de fallo (quedarse sin conexiones devueltas) que este no tiene.
//!
//! # Por qué la sonda de vitalidad mira el archivo además de consultar
//!
//! Comprobado el 2026-07-30: en Linux, borrar el archivo de la base **no** perturba a una conexión
//! ya abierta —el descriptor sigue apuntando al inodo—, así que una sonda que solo lanzara una
//! consulta seguiría respondiendo que todo va bien sobre una base que ya no existe en disco. La
//! sonda comprueba las dos cosas: que la ruta sigue existiendo y que una consulta barata contra
//! una tabla real responde.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::ErrorDeAlmacen;
use crate::migraciones::{aplicar_migraciones_de_conocimiento, aplicar_migraciones_de_sesiones};
use crate::respaldo::{self, CopiaVerificada};

/// Nombre del archivo de la base de sesiones dentro de la ruta de datos de la célula.
pub const NOMBRE_DE_ARCHIVO_DE_SESIONES: &str = "sessions.db";

/// Nombre del archivo de la base de conocimiento dentro de la ruta de datos de la célula.
pub const NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO: &str = "knowledge_live.db";

/// Espera máxima de una conexión ante una base ocupada por otro escritor.
///
/// Cinco segundos: el punto medio entre devolver un fallo por una contención que se resuelve sola
/// en milisegundos —lo normal en una célula de tráfico bajo— y quedarse colgado indefinidamente
/// en un proceso que además atiende el servidor de salud. Sin este valor, SQLite devuelve
/// `SQLITE_BUSY` de inmediato y el fallo aparecería como pérdida de mensajes en producción.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Modo de sincronización con el disco de todas las conexiones de la célula.
///
/// `NORMAL` sobre WAL, no `FULL`. La contrapartida se escribe entera para que nadie la copie sin
/// entenderla: un **corte de luz o una caída del sistema operativo** pueden perder las
/// transacciones confirmadas desde el último punto de control; una caída **del proceso** no
/// pierde ninguna, porque los datos ya están en el sistema de archivos. `FULL` costaría un
/// `fsync` por transacción sobre el disco de un equipo de diez años, en el camino caliente de
/// cada mensaje, para cubrir un corte de luz que la política de respaldos de la etapa A-2 ya trata
/// como el escenario del que se restaura.
pub const SINCRONIA: &str = "NORMAL";

/// Conexiones de solo lectura del pool de conocimiento.
pub const CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO: usize = 2;

/// Sufijo del archivo WAL que SQLite mantiene junto a cada base en modo `journal_mode = WAL`.
///
/// Se nombra una sola vez y aquí para que el punto de control y cualquier test que lo verifique
/// construyan la misma ruta de la misma forma.
pub const SUFIJO_DE_ARCHIVO_WAL: &str = "-wal";

/// Consulta barata de la sonda de vitalidad de `sessions.db`.
const CONSULTA_DE_VITALIDAD_DE_SESIONES: &str = "SELECT count(*) FROM estado_del_motor";

/// Consulta barata de la sonda de vitalidad de `knowledge_live.db`.
const CONSULTA_DE_VITALIDAD_DE_CONOCIMIENTO: &str =
    "SELECT count(*) FROM metadatos_de_conocimiento";

/// Resultado de la sonda de vitalidad de un pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Vitalidad {
    /// El archivo sigue en su sitio y la consulta de sonda respondió.
    Sana,
    /// El pool no está utilizable. Nombra **qué** falló, porque una respuesta de no preparado que
    /// no dice cuál de los componentes cayó obliga a diagnosticar a ciegas desde fuera.
    Caida {
        /// Componente concreto que falló, con el nombre del archivo que lo respalda.
        componente: &'static str,
        /// Motivo legible, en español.
        motivo: String,
    },
}

/// Pool de `sessions.db`: una conexión de escritura y una de lectura, cada una tras su cerrojo.
pub struct PoolDeSesiones {
    ruta: PathBuf,
    escritura: Mutex<Connection>,
    lectura: Mutex<Connection>,
}

impl PoolDeSesiones {
    /// Ejecuta una operación sobre la conexión de escritura, en exclusión mutua.
    ///
    /// El cerrojo se toma y se suelta **dentro** de esta llamada: no se devuelve ningún guardián
    /// al exterior, así que ningún consumidor puede mantenerlo vivo cruzando un `.await`.
    pub fn con_escritura<T>(
        &self,
        operacion: impl FnOnce(&Connection) -> Result<T, ErrorDeAlmacen>,
    ) -> Result<T, ErrorDeAlmacen> {
        let conexion = match self.escritura.lock() {
            Ok(guardian) => guardian,
            // Un cerrojo envenenado significa que otro hilo entró en pánico sosteniéndolo. La
            // conexión sigue siendo válida y SQLite deshace sola cualquier transacción abierta,
            // así que se recupera el contenido en vez de propagar el envenenamiento.
            Err(envenenado) => envenenado.into_inner(),
        };
        operacion(&conexion)
    }

    /// Ejecuta una operación sobre la conexión de lectura, en exclusión mutua.
    pub fn con_lectura<T>(
        &self,
        operacion: impl FnOnce(&Connection) -> Result<T, ErrorDeAlmacen>,
    ) -> Result<T, ErrorDeAlmacen> {
        let conexion = match self.lectura.lock() {
            Ok(guardian) => guardian,
            Err(envenenado) => envenenado.into_inner(),
        };
        operacion(&conexion)
    }

    /// Ruta del archivo que respalda este pool.
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }

    /// Sonda de vitalidad: archivo presente **y** consulta que responde.
    pub fn vitalidad(&self) -> Vitalidad {
        sondear(
            &self.ruta,
            NOMBRE_DE_ARCHIVO_DE_SESIONES,
            self.con_lectura(|conexion| contar(conexion, CONSULTA_DE_VITALIDAD_DE_SESIONES)),
        )
    }
}

/// Pool de `knowledge_live.db`: varias conexiones de solo lectura repartidas por turno rotatorio.
pub struct PoolDeConocimiento {
    ruta: PathBuf,
    lecturas: Vec<Mutex<Connection>>,
    siguiente: AtomicUsize,
}

impl PoolDeConocimiento {
    /// Ejecuta una operación sobre la siguiente conexión de lectura del turno rotatorio.
    ///
    /// El reparto es por turno y no por «la primera libre» a propósito: buscar la primera libre
    /// exigiría sondear cerrojos, y con dos conexiones y tráfico conversacional bajo el turno
    /// reparte igual de bien por una fracción del código.
    pub fn con_lectura<T>(
        &self,
        operacion: impl FnOnce(&Connection) -> Result<T, ErrorDeAlmacen>,
    ) -> Result<T, ErrorDeAlmacen> {
        if self.lecturas.is_empty() {
            return Err(ErrorDeAlmacen::PoolDeConocimientoVacio);
        }
        let indice = self.siguiente.fetch_add(1, Ordering::Relaxed) % self.lecturas.len();
        let Some(celda) = self.lecturas.get(indice) else {
            return Err(ErrorDeAlmacen::PoolDeConocimientoVacio);
        };
        let conexion = match celda.lock() {
            Ok(guardian) => guardian,
            Err(envenenado) => envenenado.into_inner(),
        };
        operacion(&conexion)
    }

    /// Ruta del archivo que respalda este pool.
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }

    /// Sonda de vitalidad: archivo presente **y** consulta que responde.
    pub fn vitalidad(&self) -> Vitalidad {
        sondear(
            &self.ruta,
            NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO,
            self.con_lectura(|conexion| contar(conexion, CONSULTA_DE_VITALIDAD_DE_CONOCIMIENTO)),
        )
    }
}

/// Agrupa los dos pools de una célula y los abre a partir de su ruta de datos.
pub struct GestorDePools {
    sesiones: PoolDeSesiones,
    conocimiento: PoolDeConocimiento,
}

impl GestorDePools {
    /// Abre y migra las dos bases derivadas de la ruta de datos ya validada de la célula.
    ///
    /// Se llama **antes** de vincular el servidor de salud: si la persistencia no arranca, la
    /// célula no debe llegar a anunciarse como viva.
    pub fn abrir(ruta_datos: &Path) -> Result<Self, ErrorDeAlmacen> {
        let metadatos = std::fs::metadata(ruta_datos).map_err(|causa| {
            ErrorDeAlmacen::RutaDeDatosInaccesible {
                ruta: ruta_datos.to_path_buf(),
                causa,
            }
        })?;
        if !metadatos.is_dir() {
            return Err(ErrorDeAlmacen::RutaDeDatosInaccesible {
                ruta: ruta_datos.to_path_buf(),
                causa: std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "la ruta de datos de la célula debe ser un directorio",
                ),
            });
        }

        let ruta_sesiones = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_SESIONES);
        let escritura = abrir_lectura_escritura(&ruta_sesiones)?;
        aplicar_migraciones_de_sesiones(&escritura)?;
        let lectura = abrir_solo_lectura(&ruta_sesiones)?;

        let ruta_conocimiento = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
        // Abrir en solo lectura un archivo que no existe falla, así que la base de conocimiento se
        // crea y se migra una sola vez en lectura y escritura, y esa conexión se cierra —al salir
        // de este bloque— antes de abrir el pool de producción. Es la única escritura que la
        // célula hace sobre esta base: en producción es de solo lectura (FR-05).
        {
            let inicial = abrir_lectura_escritura(&ruta_conocimiento)?;
            aplicar_migraciones_de_conocimiento(&inicial)?;
        }

        let mut lecturas = Vec::with_capacity(CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO);
        for _ in 0..CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO {
            lecturas.push(Mutex::new(abrir_solo_lectura(&ruta_conocimiento)?));
        }

        Ok(Self {
            sesiones: PoolDeSesiones {
                ruta: ruta_sesiones,
                escritura: Mutex::new(escritura),
                lectura: Mutex::new(lectura),
            },
            conocimiento: PoolDeConocimiento {
                ruta: ruta_conocimiento,
                lecturas,
                siguiente: AtomicUsize::new(0),
            },
        })
    }

    /// Pool de `sessions.db`.
    pub fn sesiones(&self) -> &PoolDeSesiones {
        &self.sesiones
    }

    /// Pool de `knowledge_live.db`.
    pub fn conocimiento(&self) -> &PoolDeConocimiento {
        &self.conocimiento
    }

    /// Respalda en caliente `sessions.db` y `knowledge_live.db` sobre un directorio existente,
    /// bajo sus nombres canónicos, sin tocar la conexión de escritura.
    ///
    /// Las dos copias se toman **siempre** de una conexión de lectura —`con_lectura` de cada
    /// pool—, nunca de una recién abierta ni de `con_escritura`: comprobado el 2026-07-30 con
    /// `sqlite3 -readonly`, `VACUUM INTO` **sí** funciona sobre una conexión de solo lectura y
    /// produce una copia que supera `integrity_check`, justo lo contrario de
    /// `PRAGMA wal_checkpoint`, que HEX-007 ya comprobó que falla ahí. Bajo WAL una lectura nunca
    /// bloquea al escritor, y el camino caliente del motor —`procesar_deduplicacion`,
    /// `anotar_entrante` y `anotar_saliente`— pasa siempre por `con_escritura`: el respaldo no
    /// puede hacer esperar al escritor ni producir `SQLITE_BUSY` contra él. El coste aceptado, y
    /// documentado aquí porque es donde vive: una lectura de historial concurrente con este
    /// respaldo espera detrás de él en la conexión de lectura de `sessions.db`.
    ///
    /// Las dos rutas de destino se comprueban **antes** de la primera copia, para que un destino
    /// ya ocupado o inalcanzable falle sin dejar la otra copia a medias.
    pub fn respaldar_en(
        &self,
        directorio: &Path,
    ) -> Result<ResumenDeRespaldoDePools, ErrorDeAlmacen> {
        let ruta_sesiones = directorio.join(NOMBRE_DE_ARCHIVO_DE_SESIONES);
        let ruta_conocimiento = directorio.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
        respaldo::verificar_destino_disponible(&ruta_sesiones)?;
        respaldo::verificar_destino_disponible(&ruta_conocimiento)?;

        let copia_de_sesiones = self.sesiones.con_lectura(|conexion| {
            respaldo::respaldar_base(
                conexion,
                &ruta_sesiones,
                crate::migraciones::VERSION_DE_ESQUEMA_DE_SESIONES,
                NOMBRE_DE_ARCHIVO_DE_SESIONES,
            )
        })?;
        let copia_de_conocimiento = self.conocimiento.con_lectura(|conexion| {
            respaldo::respaldar_base(
                conexion,
                &ruta_conocimiento,
                crate::migraciones::VERSION_DE_ESQUEMA_DE_CONOCIMIENTO,
                NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO,
            )
        })?;

        Ok(ResumenDeRespaldoDePools {
            copias: vec![copia_de_sesiones, copia_de_conocimiento],
        })
    }

    /// Ejecuta el punto de control del WAL al apagar la célula.
    ///
    /// Visita los dos pools, pero solo `sessions.db` puede recibir de verdad un punto de control:
    /// comprobado el 2026-07-30, `PRAGMA wal_checkpoint` sobre una conexión abierta con
    /// `SQLITE_OPEN_READ_ONLY` falla con un error de E/S de disco, y **todas** las conexiones de
    /// [`PoolDeConocimiento`] son de solo lectura por construcción (FR-05,
    /// `docs/adr/adr-0003-persistencia-dual.md`). Abrir una conexión de lectura y escritura sobre
    /// `knowledge_live.db` solo para este momento del apagado violaría precisamente el invariante
    /// que FR-05 fija, así que no se hace: se informa que ese pool es de solo lectura y no tiene
    /// nada que consolidar.
    ///
    /// Sobre `sessions.db` se ejecuta `PRAGMA wal_checkpoint(TRUNCATE)` en la única conexión de
    /// escritura: tras un `TRUNCATE` con éxito, SQLite devuelve `0|0|0` en sus tres contadores —no
    /// hay ninguna cifra positiva que comprobar—, y lo observable es que el archivo `-wal` queda en
    /// cero bytes mientras la conexión sigue abierta. Un fallo del punto de control se informa,
    /// nunca se propaga como error fatal: un WAL no consolidado no es pérdida de datos, SQLite lo
    /// reproduce solo en la siguiente apertura.
    pub fn punto_de_control_de_wal(&self) -> ResumenDePuntoDeControl {
        let resultado_de_sesiones = self.sesiones.con_escritura(|conexion| {
            conexion
                .query_row(
                    "PRAGMA wal_checkpoint(TRUNCATE)",
                    [],
                    |fila| -> rusqlite::Result<(i64, i64, i64)> {
                        Ok((fila.get(0)?, fila.get(1)?, fila.get(2)?))
                    },
                )
                .map_err(ErrorDeAlmacen::en("ejecutar el punto de control del WAL"))
        });

        let ocupado = match resultado_de_sesiones {
            Ok((bloqueado, ..)) => bloqueado != 0,
            Err(_) => true,
        };

        let ruta_wal = ruta_wal_de(&self.sesiones.ruta);
        let tamano_wal_de_sesiones_bytes = std::fs::metadata(&ruta_wal)
            .map(|metadatos| metadatos.len())
            .unwrap_or(0);

        ResumenDePuntoDeControl {
            ocupado,
            tamano_wal_de_sesiones_bytes,
        }
    }
}

/// Construye la ruta del archivo `-wal` que acompaña a la base indicada en modo WAL.
fn ruta_wal_de(ruta_de_la_base: &Path) -> PathBuf {
    let mut ruta = ruta_de_la_base.as_os_str().to_owned();
    ruta.push(SUFIJO_DE_ARCHIVO_WAL);
    PathBuf::from(ruta)
}

/// Resultado de [`GestorDePools::respaldar_en`]: las copias verificadas de `sessions.db` y de
/// `knowledge_live.db`, en ese orden fijo.
#[derive(Debug)]
pub struct ResumenDeRespaldoDePools {
    /// Copias verificadas, en el orden en que se tomaron.
    pub copias: Vec<CopiaVerificada>,
}

/// Resultado de [`GestorDePools::punto_de_control_de_wal`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResumenDePuntoDeControl {
    /// El punto de control encontró la base ocupada por otro escritor y no pudo completarse del
    /// todo. Se informa, no se escala: el motor ya se detuvo y la conexión de lectura puede
    /// sostener una marca de lectura por un instante.
    pub ocupado: bool,
    /// Tamaño en bytes del archivo `-wal` de `sessions.db` tras el intento de punto de control.
    /// Cero significa que `TRUNCATE` consolidó el WAL por completo.
    pub tamano_wal_de_sesiones_bytes: u64,
}

/// Abre una conexión de lectura y escritura, creando el archivo si no existía, y le aplica los
/// parámetros de SQLite de la célula.
///
/// `pub(crate)` porque [`crate::almacen_de_identidad`] la reutiliza para abrir su propia base
/// exactamente con el mismo criterio: WAL fijado desde la conexión de escritura, y los mismos
/// parámetros de conexión que `sessions.db` y `knowledge_live.db`.
pub(crate) fn abrir_lectura_escritura(ruta: &Path) -> Result<Connection, ErrorDeAlmacen> {
    let conexion = Connection::open(ruta)
        .map_err(ErrorDeAlmacen::en("abrir la base en lectura y escritura"))?;

    // WAL solo se puede fijar desde una conexión con permiso de escritura, porque el modo de
    // diario vive en la cabecera del archivo. Se activa aquí, en la conexión que además migra, y
    // las conexiones de solo lectura heredan el modo ya escrito en el archivo.
    //
    // WAL frente al diario de reversión clásico: permite que las lecturas de historial y la
    // escritura del mensaje en curso avancen a la vez en vez de excluirse, que es exactamente el
    // patrón de una célula (escrituras cortas y frecuentes concurrentes con lecturas). La
    // contrapartida es un segundo archivo (`-wal`) y la necesidad de puntos de control, que
    // SQLite hace solo por tamaño.
    conexion
        .query_row("PRAGMA journal_mode = WAL", [], |fila| {
            fila.get::<_, String>(0)
        })
        .map_err(ErrorDeAlmacen::en("activar el modo WAL"))?;

    aplicar_parametros_de_conexion(&conexion)?;
    Ok(conexion)
}

/// Abre una conexión de **solo lectura** y le aplica los parámetros de SQLite de la célula.
///
/// `pub(crate)`: ver la nota de [`abrir_lectura_escritura`].
pub(crate) fn abrir_solo_lectura(ruta: &Path) -> Result<Connection, ErrorDeAlmacen> {
    let conexion = Connection::open_with_flags(
        ruta,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(ErrorDeAlmacen::en("abrir la base en solo lectura"))?;

    aplicar_parametros_de_conexion(&conexion)?;
    Ok(conexion)
}

/// Fija los parámetros que son propiedad de **la conexión** y no del archivo.
///
/// Se aplican explícitamente en cada conexión y no se dan por supuestos: los valores por defecto
/// de SQLite (`busy_timeout` a cero, `foreign_keys` desactivadas) son precisamente los que
/// producirían pérdida silenciosa de datos en el camino caliente de una célula.
fn aplicar_parametros_de_conexion(conexion: &Connection) -> Result<(), ErrorDeAlmacen> {
    // Sin `busy_timeout`, SQLite devuelve `SQLITE_BUSY` en el primer choque en vez de esperar; en
    // el hardware objetivo (i7 de diez años, 8 GB de RAM, disco compartido entre células) los
    // choques breves son normales y esperar unos milisegundos es el comportamiento correcto.
    conexion
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(ErrorDeAlmacen::en("fijar busy_timeout"))?;

    // `synchronous = NORMAL`: ver [`SINCRONIA`] para la contrapartida completa frente a `FULL`.
    conexion
        .execute_batch(&format!("PRAGMA synchronous = {SINCRONIA};"))
        .map_err(ErrorDeAlmacen::en("fijar synchronous"))?;

    // SQLite trae las claves foráneas **desactivadas** por compatibilidad histórica. Sin esto,
    // las referencias declaradas en la migración serían documentación y no restricción, y un
    // parámetro de plantilla podría quedar apuntando a un mensaje que ya no existe.
    conexion
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(ErrorDeAlmacen::en("activar foreign_keys"))?;

    Ok(())
}

/// Ejecuta la consulta de sonda y devuelve su cuenta.
fn contar(conexion: &Connection, consulta: &str) -> Result<i64, ErrorDeAlmacen> {
    conexion
        .query_row(consulta, [], |fila| fila.get(0))
        .map_err(ErrorDeAlmacen::en("sondear la vitalidad de la base"))
}

/// Combina la comprobación de existencia del archivo con el resultado de la consulta de sonda.
fn sondear(
    ruta: &Path,
    componente: &'static str,
    resultado_de_la_consulta: Result<i64, ErrorDeAlmacen>,
) -> Vitalidad {
    if !ruta.exists() {
        return Vitalidad::Caida {
            componente,
            motivo: format!("el archivo {} ya no existe en disco", ruta.display()),
        };
    }

    match resultado_de_la_consulta {
        Ok(_) => Vitalidad::Sana,
        Err(error) => Vitalidad::Caida {
            componente,
            motivo: error.to_string(),
        },
    }
}
