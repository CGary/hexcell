//! Almacén SQLite del plano de control de `hexcell-admin` (tarea 14 de A-6, HEX-083): estado de
//! control de cada célula, historial de transiciones y registro de sustituciones, con el mismo
//! patrón de migración versionada que `crates/hexcell-storage/src/migraciones.rs` —guion SQL
//! embebido con `include_str!` y `PRAGMA user_version` en la misma transacción—.
//!
//! Reglas de este módulo: no abre sockets ni lee variables de entorno (la ruta llega por
//! parámetro); no guarda ningún identificador de transporte ni número de teléfono; no decide la
//! hora (cada escritura recibe un `ahora_ms: i64` explícito, así las pruebas son deterministas y
//! la raíz de composición es la única dueña del reloj); y no valida transiciones, que es trabajo
//! de la capa de comando contra `EstadoDeCelula::transiciones_permitidas`.

use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::estado_de_celula::EstadoDeCelula;

/// Variable de entorno que fija la ruta del almacén del plano de control.
pub const VARIABLE_DE_RUTA_DEL_ALMACEN: &str = "HEXCELL_ADMIN_ALMACEN";

/// Ruta por omisión cuando la variable de entorno no está fijada.
pub const RUTA_POR_OMISION_DEL_ALMACEN: &str = "/var/lib/hexcell-admin/plano_de_control.db";

/// Versión de esquema que este binario espera encontrar en el almacén del plano de control.
pub const VERSION_DE_ESQUEMA_DEL_PLANO: i64 = 1;

/// Motivo con el que se da de alta implícitamente una célula la primera vez que se pausa o
/// reanuda sin tener fila previa en `celulas`.
pub const MOTIVO_DE_ALTA_IMPLICITA: &str = "alta_implicita";

/// Motivo con el que `cell terminate` persistirá el estado `Retirada`.
///
/// **Inerte en HEX-083:** la tarea 12 (cell terminate) no está fusionada en main a fecha de
/// este commit; la constante queda declarada aquí para que la tarea 12 la consuma cuando
/// llegue, sin que HEX-083 implemente el comportamiento contra código no fusionado.
pub const MOTIVO_DE_SESION_CERRADA: &str = "sesion_cerrada";

const MIGRACION_0001: &str = include_str!("../migraciones/0001-plano-de-control.sql");

/// Fallo tipado del almacén del plano de control.
#[derive(Debug)]
pub enum ErrorDeAlmacenDePlano {
    /// El directorio padre de la ruta configurada no existe; el almacén no lo crea.
    DirectorioInaccesible {
        /// Ruta del directorio que se esperaba encontrar.
        directorio: PathBuf,
    },
    /// El motor SQLite rechazó una operación.
    Sqlite {
        /// Descripción, en español, de la operación que fallaba.
        operacion: &'static str,
        /// Causa original devuelta por SQLite.
        causa: rusqlite::Error,
    },
    /// El archivo del almacén no existe y la operación en curso no puede crearlo. Distinto de
    /// [`Self::DirectorioInaccesible`]: ahí falta el directorio que el operador debe crear.
    ArchivoAusente {
        /// Ruta del archivo que se esperaba encontrar.
        ruta: PathBuf,
    },
    /// El almacén existe pero su versión de esquema no es la que este binario espera.
    EsquemaSinMigrar {
        /// Versión leída de `PRAGMA user_version`.
        encontrada: i64,
        /// Versión que este binario espera.
        esperada: i64,
    },
    /// La etiqueta de estado leída de la base no corresponde a ninguna variante conocida.
    EstadoDesconocido {
        /// Etiqueta tal y como salió de la base.
        etiqueta: String,
    },
}

impl ErrorDeAlmacenDePlano {
    /// Convierte un `rusqlite::Error` en un `ErrorDeAlmacenDePlano::Sqlite` con la operación
    /// ya nombrada, para usar como `.map_err(ErrorDeAlmacenDePlano::en("..."))`.
    pub fn en(operacion: &'static str) -> impl FnOnce(rusqlite::Error) -> Self {
        move |causa| Self::Sqlite { operacion, causa }
    }
}

impl fmt::Display for ErrorDeAlmacenDePlano {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectorioInaccesible { directorio } => write!(
                f,
                "el directorio «{}» del almacén del plano de control no existe; hexcell-admin no lo crea",
                directorio.display()
            ),
            Self::ArchivoAusente { ruta } => write!(
                f,
                "el almacén del plano de control «{}» no existe; «cell status» y «cell list» sólo leen y no lo crean",
                ruta.display()
            ),
            Self::EsquemaSinMigrar {
                encontrada,
                esperada,
            } => write!(
                f,
                "el almacén del plano de control declara la versión de esquema {encontrada} y este binario espera la {esperada}"
            ),
            Self::Sqlite { operacion, causa } => {
                write!(f, "fallo de SQLite al {operacion}: {causa}")
            }
            Self::EstadoDesconocido { etiqueta } => write!(
                f,
                "la etiqueta de estado «{etiqueta}» no corresponde a ninguna variante conocida"
            ),
        }
    }
}

impl std::error::Error for ErrorDeAlmacenDePlano {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite { causa, .. } => Some(causa),
            _ => None,
        }
    }
}

/// Fila de la tabla `celulas` ya leída como valor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FilaDeCelula {
    /// Identificador de la célula.
    pub id: String,
    /// Estado actual persistido.
    pub estado: EstadoDeCelula,
    /// Motivo de la última transición.
    pub motivo: String,
    /// Marca de tiempo (ms desde epoch) de la última actualización.
    pub actualizado_ms: i64,
}

/// Fila de la tabla `sustituciones` ya leída como valor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sustitucion {
    /// Identificador de la célula.
    pub id_celula: String,
    /// Motivo de la sustitución.
    pub motivo: String,
    /// Marca de tiempo (ms desde epoch) del registro.
    pub registrado_ms: i64,
}

/// Traduce un `EstadoDeCelula` a la etiqueta ASCII snake_case que se persiste.
///
/// No usa el `Display` de `EstadoDeCelula` porque éste produce etiquetas acentuadas con
/// espacios ("en ejecución") inadecuadas para una columna de estado.
pub fn etiqueta_persistida(estado: EstadoDeCelula) -> &'static str {
    match estado {
        EstadoDeCelula::Aprovisionada => "aprovisionada",
        EstadoDeCelula::EnEjecucion => "en_ejecucion",
        EstadoDeCelula::Suspendida => "suspendida",
        EstadoDeCelula::Reemparejando => "reemparejando",
        EstadoDeCelula::Retirada => "retirada",
    }
}

/// Traduce una etiqueta persistida al `EstadoDeCelula` correspondiente, o devuelve un error
/// tipado si la etiqueta no corresponde a ninguna variante conocida.
pub fn estado_desde_etiqueta(etiqueta: &str) -> Result<EstadoDeCelula, ErrorDeAlmacenDePlano> {
    match etiqueta {
        "aprovisionada" => Ok(EstadoDeCelula::Aprovisionada),
        "en_ejecucion" => Ok(EstadoDeCelula::EnEjecucion),
        "suspendida" => Ok(EstadoDeCelula::Suspendida),
        "reemparejando" => Ok(EstadoDeCelula::Reemparejando),
        "retirada" => Ok(EstadoDeCelula::Retirada),
        otra => Err(ErrorDeAlmacenDePlano::EstadoDesconocido {
            etiqueta: otra.to_string(),
        }),
    }
}

/// Almacén del plano de control: una conexión SQLite ya migrada sobre la que se leen y
/// escriben las tres tablas del esquema.
pub struct AlmacenDelPlanoDeControl {
    conexion: Connection,
}

impl AlmacenDelPlanoDeControl {
    /// Abre (o crea) el almacén en `ruta`, aplicando las migraciones pendientes.
    ///
    /// Rechaza con [`ErrorDeAlmacenDePlano::DirectorioInaccesible`] si el directorio padre
    /// no existe: el almacén nunca crea ese directorio.
    pub fn abrir(ruta: &Path) -> Result<Self, ErrorDeAlmacenDePlano> {
        Self::exigir_directorio(ruta)?;
        let conexion = Connection::open(ruta).map_err(Self::en("abrir el almacén"))?;
        aplicar_migracion(&conexion)?;
        Ok(Self { conexion })
    }

    /// Abre el almacén en `ruta` en modo **sólo lectura**: no crea el archivo, no crea el
    /// directorio y no aplica ninguna migración.
    ///
    /// Es la única puerta de `cell status` y `cell list`, y hace que «no escriben nunca» sea una
    /// propiedad del descriptor y no de una convención: con `SQLITE_OPEN_READ_ONLY` y sin
    /// `SQLITE_OPEN_CREATE`, cualquier escritura que se colara en esas rutas fallaría con
    /// `SQLITE_READONLY`, y un almacén inexistente es un error tipado en vez de un archivo recién
    /// creado con su esquema aplicado.
    pub fn abrir_solo_lectura(ruta: &Path) -> Result<Self, ErrorDeAlmacenDePlano> {
        Self::exigir_directorio(ruta)?;
        if !ruta.is_file() {
            return Err(ErrorDeAlmacenDePlano::ArchivoAusente {
                ruta: ruta.to_path_buf(),
            });
        }
        let conexion = Connection::open_with_flags(ruta, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(Self::en("abrir el almacén en sólo lectura"))?;
        let encontrada: i64 = conexion
            .query_row("PRAGMA user_version", [], |fila| fila.get(0))
            .map_err(Self::en("leer la versión de esquema"))?;
        if encontrada != VERSION_DE_ESQUEMA_DEL_PLANO {
            return Err(ErrorDeAlmacenDePlano::EsquemaSinMigrar {
                encontrada,
                esperada: VERSION_DE_ESQUEMA_DEL_PLANO,
            });
        }
        Ok(Self { conexion })
    }

    /// Exige que el directorio padre de `ruta` exista. `hexcell-admin` nunca lo crea.
    fn exigir_directorio(ruta: &Path) -> Result<(), ErrorDeAlmacenDePlano> {
        if let Some(directorio) = ruta.parent()
            && !directorio.as_os_str().is_empty()
            && !directorio.is_dir()
        {
            return Err(ErrorDeAlmacenDePlano::DirectorioInaccesible {
                directorio: directorio.to_path_buf(),
            });
        }
        Ok(())
    }

    /// Lee el estado actual de una célula, o `None` si no existe fila.
    pub fn leer_estado(&self, id: &str) -> Result<Option<FilaDeCelula>, ErrorDeAlmacenDePlano> {
        let mut sentencia = self
            .conexion
            .prepare("SELECT id, estado, motivo, actualizado_ms FROM celulas WHERE id = ?1")
            .map_err(Self::en("preparar la lectura de estado"))?;
        let mut filas = sentencia
            .query_map([id], |fila| {
                Ok((
                    fila.get::<_, String>(0)?,
                    fila.get::<_, String>(1)?,
                    fila.get::<_, String>(2)?,
                    fila.get::<_, i64>(3)?,
                ))
            })
            .map_err(Self::en("mapear la lectura de estado"))?;
        match filas.next() {
            Some(Ok((id, etiqueta, motivo, actualizado_ms))) => {
                let estado = estado_desde_etiqueta(&etiqueta)?;
                Ok(Some(FilaDeCelula {
                    id,
                    estado,
                    motivo,
                    actualizado_ms,
                }))
            }
            Some(Err(error)) => Err(ErrorDeAlmacenDePlano::Sqlite {
                operacion: "leer la fila de celulas",
                causa: error,
            }),
            None => Ok(None),
        }
    }

    /// Actualiza la fila de `celulas` (UPSERT) e inserta una fila en `transiciones`, todo en
    /// una misma transacción.
    ///
    /// El parámetro `de` es el estado anterior de la célula, o `None` si la célula no tenía
    /// fila previa (alta implícita). El parámetro `a` es el estado objetivo.
    pub fn registrar_transicion(
        &self,
        id: &str,
        de: Option<EstadoDeCelula>,
        a: EstadoDeCelula,
        motivo: &str,
        ahora_ms: i64,
    ) -> Result<(), ErrorDeAlmacenDePlano> {
        let transaccion = self
            .conexion
            .unchecked_transaction()
            .map_err(Self::en("iniciar la transición"))?;
        transaccion
            .execute(
                "INSERT INTO celulas (id, estado, motivo, actualizado_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                    estado = ?2,
                    motivo = ?3,
                    actualizado_ms = ?4",
                rusqlite::params![id, etiqueta_persistida(a), motivo, ahora_ms,],
            )
            .map_err(Self::en("actualizar la fila de celulas"))?;
        let etiqueta_de = de.map(etiqueta_persistida).unwrap_or("");
        transaccion
            .execute(
                "INSERT INTO transiciones (id_celula, de, a, motivo, registrado_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, etiqueta_de, etiqueta_persistida(a), motivo, ahora_ms],
            )
            .map_err(Self::en("insertar la transición"))?;
        transaccion
            .commit()
            .map_err(Self::en("confirmar la transición"))?;
        Ok(())
    }

    /// Lee el historial de sustituciones de una célula, ordenado por `registrado_ms` ascendente.
    pub fn leer_sustituciones(&self, id: &str) -> Result<Vec<Sustitucion>, ErrorDeAlmacenDePlano> {
        let mut sentencia = self
            .conexion
            .prepare(
                "SELECT id_celula, motivo, registrado_ms FROM sustituciones
                 WHERE id_celula = ?1 ORDER BY registrado_ms ASC",
            )
            .map_err(Self::en("preparar la lectura de sustituciones"))?;
        let filas = sentencia
            .query_map([id], |fila| {
                Ok(Sustitucion {
                    id_celula: fila.get::<_, String>(0)?,
                    motivo: fila.get::<_, String>(1)?,
                    registrado_ms: fila.get::<_, i64>(2)?,
                })
            })
            .map_err(Self::en("mapear la lectura de sustituciones"))?;
        let mut resultado = Vec::new();
        for fila in filas {
            resultado.push(fila.map_err(|e| ErrorDeAlmacenDePlano::Sqlite {
                operacion: "leer la fila de sustituciones",
                causa: e,
            })?);
        }
        Ok(resultado)
    }

    /// Lista todas las filas de `celulas`, ordenadas por `id` ascendente.
    pub fn listar_celulas(&self) -> Result<Vec<FilaDeCelula>, ErrorDeAlmacenDePlano> {
        let mut sentencia = self
            .conexion
            .prepare("SELECT id, estado, motivo, actualizado_ms FROM celulas ORDER BY id ASC")
            .map_err(Self::en("preparar el listado de células"))?;
        let filas = sentencia
            .query_map([], |fila| {
                Ok((
                    fila.get::<_, String>(0)?,
                    fila.get::<_, String>(1)?,
                    fila.get::<_, String>(2)?,
                    fila.get::<_, i64>(3)?,
                ))
            })
            .map_err(Self::en("mapear el listado de células"))?;
        let mut resultado = Vec::new();
        for fila in filas {
            let (id, etiqueta, motivo, actualizado_ms) =
                fila.map_err(|e| ErrorDeAlmacenDePlano::Sqlite {
                    operacion: "leer la fila de celulas",
                    causa: e,
                })?;
            let estado = estado_desde_etiqueta(&etiqueta)?;
            resultado.push(FilaDeCelula {
                id,
                estado,
                motivo,
                actualizado_ms,
            });
        }
        Ok(resultado)
    }

    fn en(operacion: &'static str) -> impl FnOnce(rusqlite::Error) -> ErrorDeAlmacenDePlano {
        ErrorDeAlmacenDePlano::en(operacion)
    }
}

/// Aplica la migración 0001 si la versión actual es menor que 1.
fn aplicar_migracion(conexion: &Connection) -> Result<(), ErrorDeAlmacenDePlano> {
    let version_actual: i64 = conexion
        .query_row("PRAGMA user_version", [], |fila| fila.get(0))
        .map_err(ErrorDeAlmacenDePlano::en("leer la versión de esquema"))?;
    if version_actual >= VERSION_DE_ESQUEMA_DEL_PLANO {
        return Ok(());
    }
    let transaccion = conexion
        .unchecked_transaction()
        .map_err(ErrorDeAlmacenDePlano::en("iniciar la migración"))?;
    transaccion
        .execute_batch(MIGRACION_0001)
        .map_err(ErrorDeAlmacenDePlano::en("aplicar el esquema inicial"))?;
    transaccion
        .execute_batch(&format!(
            "PRAGMA user_version = {};",
            VERSION_DE_ESQUEMA_DEL_PLANO
        ))
        .map_err(ErrorDeAlmacenDePlano::en("fijar la versión de esquema"))?;
    transaccion
        .commit()
        .map_err(ErrorDeAlmacenDePlano::en("confirmar la migración"))?;
    Ok(())
}
