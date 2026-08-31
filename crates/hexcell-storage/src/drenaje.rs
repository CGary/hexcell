//! Drenaje ordenado de épocas superseídas de la base de conocimiento.
//!
//! Este módulo implementa el proceso síncrono que aguarda a que las conexiones de lectura
//! activas sobre una época superseída alcancen el reposo completo antes de cerrar el pool
//! y verificar la ausencia de diarios WAL con datos no consolidados.
//!
//! # Predicado de dos lados
//! El reposo no puede determinarse únicamente con `lecturas_en_reposo()`, pues esta sonda
//! solo prueba si hay consultas ejecutándose en el instante del sondeo y no quién retiene
//! referencias vivas al pool. Por ello, el predicado exige la conjunción estricta de:
//! 1. `lecturas_en_reposo()` (todos los cerrojos de lectura libres).
//! 2. `Arc::strong_count == 1` (ningún otro componente retiene un clon del pool).
//!
//! # Expiración con fallo cerrado
//! Si el límite temporal transcurre antes de que el predicado se cumpla, el drenaje
//! retorna [`DesenlaceDeDrenaje::Expirada`] devolviendo el descriptor vivo [`EpocaSuperseida`].
//! Esto mantiene el pool accesible, deja el consumo de descriptores observable y permite
//! reintentar el drenaje más adelante, sin cerrar conexiones a la fuerza ni borrar archivos.
//!
//! # Verificación y aborto de archivos asociados
//! Tras el cierre limpio mediante `Arc::into_inner`, la verificación post-cierre comprueba
//! los archivos secundarios en disco. Siguiendo la resolución del 31 de agosto de 2026 sobre
//! RISK-1, las conexiones SQLite en solo lectura generan archivos `-shm` y `-wal` de cero
//! bytes que sobreviven al cierre por falta de permisos de borrado. La verificación distingue
//! el residuo inocuo de los datos en riesgo por tamaño: un `-wal` con tamaño mayor a cero
//! produce [`ErrorDeAlmacen::CompanieroDeEpocaSobreviviente`] sin eliminarlo, mientras que un
//! `-wal` vacío y un `-shm` se toleran como residuo benigno.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::conocimiento::SUFIJO_DE_ARCHIVO_SHM;
use crate::error::ErrorDeAlmacen;
use crate::pools::SUFIJO_DE_ARCHIVO_WAL;
use crate::promocion::EpocaSuperseida;

/// Límite de tiempo por omisión para el drenaje de una época superseída (10 segundos).
///
/// Este valor supera el tiempo de espera por bloqueo (`BUSY_TIMEOUT` de 5 segundos) para no
/// señalar como bloqueada una lectura legítimamente en contención, y permanece por debajo
/// del margen de 20 segundos del apagado ordenado general.
pub const LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO: Duration = Duration::from_secs(10);

/// Intervalo de sondeo entre evaluaciones consecutivas del predicado de reposo (5 milisegundos).
pub const INTERVALO_DE_SONDEO_DE_DRENAJE: Duration = Duration::from_millis(5);

/// Resultado del proceso de drenaje ordenado de una época superseída.
#[derive(Debug, PartialEq)]
pub enum DesenlaceDeDrenaje {
    /// La época superseída alcanzó el reposo completo y su pool fue cerrado con éxito.
    Drenada {
        /// Ruta física del archivo de base de datos de la época drenada.
        ruta_del_archivo: PathBuf,
        /// Número ordinal de la época drenada, o `None` si era la base inicial.
        numero_de_epoca: Option<i64>,
        /// Tiempo transcurrido durante la espera en milisegundos.
        espera_ms: u64,
    },
    /// El límite de tiempo expiró mientras aún existían lectores activos o referencias retenidas.
    Expirada {
        /// Descriptor vivo de la época superseída devuelto intacto para permitir reintentos.
        epoca_superseida: EpocaSuperseida,
        /// Cantidad de referencias fuertes al pool observadas al momento de expirar.
        titulares: usize,
        /// Estado de reposo de los cerrojos de lectura al momento de expirar.
        lecturas_en_reposo: bool,
    },
    /// El predicado de reposo se cumplió pero otra referencia apareció antes del consumo exclusivo.
    Retenida {
        /// Ruta física del archivo de base de datos de la época.
        ruta_del_archivo: PathBuf,
        /// Número ordinal de la época, o `None` si era la base inicial.
        numero_de_epoca: Option<i64>,
        /// Cantidad de referencias observadas.
        titulares: usize,
    },
}

/// Verifica que tras el cierre del pool no permanezcan archivos secundarios con datos no consolidados.
///
/// Una conexión SQLite abierta en solo lectura genera archivos `-shm` y `-wal` de cero bytes
/// que persisten tras su cierre al no tener permisos de eliminación. Por tanto, la comprobación
/// opera evaluando el tamaño: un archivo `-wal` con tamaño mayor a cero delata transacciones
/// no consolidadas y retorna error sin borrar el archivo, mientras que un `-wal` de cero bytes
/// y un `-shm` se toleran como residuo documentado inocuo.
fn verificar_companeros_de_la_epoca(ruta_archivo: &Path) -> Result<(), ErrorDeAlmacen> {
    let mut ruta_wal = ruta_archivo.as_os_str().to_owned();
    ruta_wal.push(SUFIJO_DE_ARCHIVO_WAL);
    let ruta_wal = PathBuf::from(ruta_wal);

    if let Ok(metadatos_wal) = std::fs::metadata(&ruta_wal) {
        let bytes = metadatos_wal.len();
        if bytes > 0 {
            return Err(ErrorDeAlmacen::CompanieroDeEpocaSobreviviente {
                ruta: ruta_wal,
                bytes,
            });
        }
    }

    let mut ruta_shm = ruta_archivo.as_os_str().to_owned();
    ruta_shm.push(SUFIJO_DE_ARCHIVO_SHM);
    let ruta_shm = PathBuf::from(ruta_shm);

    // El archivo de memoria compartida carece de datos propios cuando el diario está vacío.
    if let Ok(_metadatos_shm) = std::fs::metadata(&ruta_shm) {
        // Residuo inocuo tolerado de conexiones en solo lectura.
    }

    Ok(())
}

/// Ejecuta el drenaje síncrono y acotado de una época de conocimiento superseída.
///
/// Evalúa periódicamente el predicado de dos lados: que las conexiones de lectura estén en reposo
/// (`lecturas_en_reposo()`) y que no existan otras referencias activas (`Arc::strong_count == 1`).
/// Si el plazo monótono calculado desde `instante_de_reemplazo` supera `limite`, retorna
/// [`DesenlaceDeDrenaje::Expirada`] conservando el descriptor vivo sin cerrar conexiones ni borrar
/// archivos en disco.
pub fn drenar_epoca_superseida(
    epoca: EpocaSuperseida,
    limite: Duration,
) -> Result<DesenlaceDeDrenaje, ErrorDeAlmacen> {
    let instante_inicio = std::time::Instant::now();

    loop {
        let lecturas_en_reposo = epoca.lecturas_en_reposo();
        let titulares = Arc::strong_count(epoca.pool());

        if lecturas_en_reposo && titulares == 1 {
            let ruta_del_archivo = epoca.ruta_del_archivo().to_path_buf();
            let numero_de_epoca = epoca.numero_de_epoca();
            let espera_ms =
                u64::try_from(instante_inicio.elapsed().as_millis()).unwrap_or(u64::MAX);

            let pool = epoca.tomar_pool();
            return match Arc::into_inner(pool) {
                Some(pool_cerrado) => {
                    drop(pool_cerrado);
                    verificar_companeros_de_la_epoca(&ruta_del_archivo)?;
                    Ok(DesenlaceDeDrenaje::Drenada {
                        ruta_del_archivo,
                        numero_de_epoca,
                        espera_ms,
                    })
                }
                None => Ok(DesenlaceDeDrenaje::Retenida {
                    ruta_del_archivo,
                    numero_de_epoca,
                    titulares: 2,
                }),
            };
        }

        if epoca.instante_de_reemplazo().elapsed() >= limite {
            return Ok(DesenlaceDeDrenaje::Expirada {
                epoca_superseida: epoca,
                titulares,
                lecturas_en_reposo,
            });
        }

        std::thread::sleep(INTERVALO_DE_SONDEO_DE_DRENAJE);
    }
}
