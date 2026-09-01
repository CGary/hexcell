//! Retención y purga ordenada de épocas selladas de conocimiento.
//!
//! Este módulo implementa la única ruta autorizada de eliminación de archivos de época en la
//! base de código (`purgar_epocas_retiradas`), sujeta a cuatro cercas estructurales y cuatro
//! invariantes de no-purga simultáneas:
//!
//! # Cuatro invariantes de no-purga
//! 1. **Época viva**: el destino resuelto de `knowledge_live.db` nunca se elimina.
//! 2. **Superseída sin drenar**: ninguna época presente en el registro `epocas_en_uso` se elimina.
//! 3. **Destino de reversión**: purga toma `gestor.iniciar_promocion()`, impidiendo concurrir
//!    con cualquier promoción o reversión activa.
//! 4. **Ventana de retención**: las N épocas sanas más recientes fuera de la viva se conservan.
//!
//! # Marcas de sospecha de defecto
//! Una época revertida porta una marca `.sospechosa` cuyo contenido lleva su número intrínseco.
//! La marca nunca se purga, reserva el número para que `numero_de_epoca_siguiente` no lo reutilice
//! y despoja a la época de protección de recencia para permitir su purga prioritaria.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::conocimiento::{NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA, SUFIJO_DE_ARCHIVO_SHM};
use crate::error::ErrorDeAlmacen;
use crate::pools::{
    GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, SUFIJO_DE_ARCHIVO_WAL, abrir_solo_lectura,
    verificar_enlace_vivo_resoluble,
};
use crate::promocion::PREFIJO_DE_ARCHIVO_DE_EPOCA;

/// Ventana de retención por omisión: época viva más dos predecesoras selladas.
pub const VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO: usize = 2;

/// Sufijo canónico del archivo de marca que identifica a una época sospechosa de defecto.
pub const SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA: &str = ".sospechosa";

/// Información y metadatos contenidos en el archivo de marca de una época sospechosa.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarcaDeEpocaSospechosa {
    /// Número ordinal intrínseco de la época marcada.
    pub numero_de_epoca: i64,
    /// Motivo documentado por el cual se marcó la época tras una reversión.
    pub motivo: String,
    /// Fecha absoluta en formato ISO (YYYY-MM-DD) de la creación de la marca.
    pub fecha_absoluta: String,
}

/// Escribe de forma síncrona el archivo de marca sospechosa para la época indicada.
///
/// El archivo se nombra `knowledge_epoch_N.sospechosa` y graba en su contenido el número intrínseco,
/// el motivo y la fecha absoluta.
pub fn escribir_marca_de_epoca_sospechosa(
    ruta_datos: &Path,
    numero_de_epoca: i64,
    motivo: &str,
    fecha_absoluta: &str,
) -> Result<PathBuf, ErrorDeAlmacen> {
    let nombre_archivo = format!(
        "{PREFIJO_DE_ARCHIVO_DE_EPOCA}{numero_de_epoca}{SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA}"
    );
    let ruta_marca = ruta_datos.join(&nombre_archivo);
    let contenido = format!(
        "numero_de_epoca: {numero_de_epoca}\nmotivo: {motivo}\nfecha_absoluta: {fecha_absoluta}\n"
    );

    std::fs::write(&ruta_marca, contenido).map_err(|causa| {
        ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta_marca.clone(),
            operacion: "escribir marca de época sospechosa",
            causa,
        }
    })?;

    Ok(ruta_marca)
}

/// Lee y valida todas las marcas de época sospechosa presentes en el directorio de datos.
///
/// Si el número grabado en el contenido de la marca no coincide con el número derivado de su
/// nombre de archivo, retorna [`ErrorDeAlmacen::NumeroDeMarcaDiscrepante`] abortando la operación.
/// Si el contenido no se puede parsear, retorna [`ErrorDeAlmacen::MarcaDeEpocaIlegible`].
pub fn leer_marcas_de_epoca_sospechosa(
    ruta_datos: &Path,
) -> Result<Vec<MarcaDeEpocaSospechosa>, ErrorDeAlmacen> {
    let entradas =
        std::fs::read_dir(ruta_datos).map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_datos.to_path_buf(),
            causa,
        })?;

    let mut marcas = Vec::new();

    for entrada_res in entradas {
        let entrada = entrada_res.map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_datos.to_path_buf(),
            causa,
        })?;
        let ruta = entrada.path();
        if ruta.is_dir() {
            continue;
        }
        let Some(nombre) = ruta.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !nombre.ends_with(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA) {
            continue;
        }

        if !nombre.starts_with(PREFIJO_DE_ARCHIVO_DE_EPOCA) {
            return Err(ErrorDeAlmacen::MarcaDeEpocaIlegible {
                ruta: ruta.clone(),
                motivo: format!(
                    "el archivo {nombre} no inicia con el prefijo canónico {PREFIJO_DE_ARCHIVO_DE_EPOCA}"
                ),
            });
        }

        let parte_numero = &nombre[PREFIJO_DE_ARCHIVO_DE_EPOCA.len()
            ..nombre.len() - SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA.len()];
        let numero_en_nombre: i64 =
            parte_numero
                .parse()
                .map_err(|_| ErrorDeAlmacen::MarcaDeEpocaIlegible {
                    ruta: ruta.clone(),
                    motivo: format!("no se pudo interpretar el número en el nombre {nombre}"),
                })?;

        let contenido = std::fs::read_to_string(&ruta).map_err(|causa| {
            ErrorDeAlmacen::MarcaDeEpocaIlegible {
                ruta: ruta.clone(),
                motivo: format!("fallo al leer el archivo de marca: {causa}"),
            }
        })?;

        let mut numero_en_contenido: Option<i64> = None;
        let mut motivo_opt: Option<String> = None;
        let mut fecha_opt: Option<String> = None;

        for linea in contenido.lines() {
            let linea = linea.trim();
            if linea.is_empty() {
                continue;
            }
            if let Some(resto) = linea.strip_prefix("numero_de_epoca:") {
                numero_en_contenido = resto.trim().parse::<i64>().ok();
            } else if let Some(resto) = linea.strip_prefix("motivo:") {
                motivo_opt = Some(resto.trim().to_string());
            } else if let Some(resto) = linea.strip_prefix("fecha_absoluta:") {
                fecha_opt = Some(resto.trim().to_string());
            }
        }

        let Some(num_contenido) = numero_en_contenido else {
            return Err(ErrorDeAlmacen::MarcaDeEpocaIlegible {
                ruta: ruta.clone(),
                motivo: "campo numero_de_epoca ausente o inválido en el contenido de la marca"
                    .to_string(),
            });
        };

        if numero_en_nombre != num_contenido {
            return Err(ErrorDeAlmacen::NumeroDeMarcaDiscrepante {
                ruta: ruta.clone(),
                numero_en_nombre,
                numero_en_contenido: num_contenido,
            });
        }

        marcas.push(MarcaDeEpocaSospechosa {
            numero_de_epoca: num_contenido,
            motivo: motivo_opt.unwrap_or_default(),
            fecha_absoluta: fecha_opt.unwrap_or_default(),
        });
    }

    Ok(marcas)
}

/// Extrae el conjunto de números ordinales de todas las épocas con marca de sospecha válida.
pub fn numeros_de_epoca_marcados(ruta_datos: &Path) -> Result<BTreeSet<i64>, ErrorDeAlmacen> {
    let marcas = leer_marcas_de_epoca_sospechosa(ruta_datos)?;
    Ok(marcas.into_iter().map(|m| m.numero_de_epoca).collect())
}

/// Motivo exhaustivo por el cual una época sellada fue conservada y no purgada.
///
/// Coincidencia exhaustiva sin comodín `_`, forzando que cualquier nueva política de conservación
/// deba ser explícitamente declarada y clasificada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MotivoDeConservacion {
    /// Corresponde a la época actualmente viva apuntada por el enlace `knowledge_live.db`.
    EsLaEpocaViva,
    /// Se encuentra registrada en `epocas_en_uso` pendiente de drenaje ordenado.
    SuperseidaSinDrenar,
    /// Se encuentra dentro del margen de recencia fijado por la ventana de retención.
    DentroDeLaVentanaDeRetencion,
    /// El archivo secundario `-wal` contiene transacciones sin consolidar (tamaño > 0).
    DiarioConDatosSinConsolidar {
        /// Cantidad de bytes observados en el archivo WAL secundario.
        bytes: u64,
    },
}

/// Detalle de una época sellada que fue conservada en disco tras la purga.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpocaConservada {
    /// Número ordinal intrínseco de la época conservada.
    pub numero_de_epoca: i64,
    /// Ruta física del archivo de base de datos conservado.
    pub ruta_del_archivo: PathBuf,
    /// Justificación por la cual la época fue protegida de la purga.
    pub motivo: MotivoDeConservacion,
}

/// Detalle de una época sellada cuyo archivo principal y residuos inocuos fueron eliminados.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpocaPurgada {
    /// Número ordinal intrínseco de la época eliminada.
    pub numero_de_epoca: i64,
    /// Ruta física original del archivo de época eliminado.
    pub ruta_del_archivo: PathBuf,
}

/// Resultado final de la ejecución de una ronda de purga sobre el directorio de datos.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesenlaceDePurga {
    /// Listado de épocas cuyos archivos fueron eliminados de disco.
    pub epocas_purgadas: Vec<EpocaPurgada>,
    /// Listado de épocas que sobrevivieron a la purga con sus motivos exhaustivos.
    pub epocas_conservadas: Vec<EpocaConservada>,
}

/// Estructura interna para clasificar candidatos durante el escaneo de purga.
struct CandidatoDeEpoca {
    numero_de_epoca: i64,
    ruta_archivo: PathBuf,
    es_viva: bool,
}

/// Ejecuta la purga síncrona de épocas selladas retiradas que quedan fuera de la ventana de retención.
///
/// La secuencia aplica las siguientes compuertas en estricto orden:
/// 1. Adquiere exclusión mutua de promoción (`gestor.iniciar_promocion()`).
/// 2. Valida la resolución del enlace vivo (`verificar_enlace_vivo_resoluble`).
/// 3. Resuelve la ruta física y el número intrínseco de la época viva actual.
/// 4. Carga el registro en memoria `epocas_en_uso` y las marcas de época sospechosa.
/// 5. Escanea los archivos de época sellados en disco leyendo `metadatos_de_epoca`.
/// 6. Clasifica candidatos respetando todas las invariantes de conservación.
/// 7. Elimina únicamente el archivo `.db`, su `-wal` de cero bytes y su `-shm`, conservando
///    cualquier candidato con `-wal` de tamaño mayor a cero y preservando siempre las marcas.
pub fn purgar_epocas_retiradas(
    gestor: &GestorDePools,
    ruta_datos: &Path,
    ventana_de_retencion: usize,
) -> Result<DesenlaceDePurga, ErrorDeAlmacen> {
    // 1. Exclusión mutua: purga no puede correr concurrentemente con promoción ni reversión.
    let _guardian = gestor.iniciar_promocion()?;

    // 2. Verificar enlace vivo resoluble antes de cualquier inspección.
    verificar_enlace_vivo_resoluble(ruta_datos)?;

    // 3. Resolver canónicamente la época viva e inspeccionar su número intrínseco.
    let ruta_live = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    if !ruta_live.exists() && std::fs::symlink_metadata(&ruta_live).is_err() {
        return Err(ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_live,
            causa: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "knowledge_live.db no existe en la ruta de datos",
            ),
        });
    }

    let ruta_live_canonica = std::fs::canonicalize(&ruta_live).map_err(|causa| {
        ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta_live.clone(),
            operacion: "resolver ruta física de la época viva para purga",
            causa,
        }
    })?;

    let conexion_live = abrir_solo_lectura(&ruta_live_canonica)?;
    let consulta_live: Result<(Option<i64>, Option<i64>), rusqlite::Error> = conexion_live
        .query_row(
            "SELECT numero_de_epoca, sellada_ms FROM metadatos_de_epoca WHERE id = 1",
            [],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        );
    drop(conexion_live);

    let numero_vivo_intrinseco: Option<i64> = match consulta_live {
        Ok((num, _)) => num,
        Err(causa) => {
            return Err(ErrorDeAlmacen::EpocaVivaNoIdentificable {
                ruta: ruta_live_canonica,
                motivo: format!("fallo al leer metadatos_de_epoca: {causa}"),
            });
        }
    };

    // 4. Cargar snapshot del registro de épocas en uso y marcas sospechosas.
    let en_uso = gestor.epocas_en_uso();
    let marcas = leer_marcas_de_epoca_sospechosa(ruta_datos)?;
    let numeros_marcados: BTreeSet<i64> = marcas.into_iter().map(|m| m.numero_de_epoca).collect();

    // 5. Escanear archivos de época sellados en disco.
    let entradas =
        std::fs::read_dir(ruta_datos).map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_datos.to_path_buf(),
            causa,
        })?;

    let mut candidatos: Vec<CandidatoDeEpoca> = Vec::new();

    for entrada_res in entradas {
        let entrada = match entrada_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ruta = entrada.path();
        if std::fs::metadata(&ruta).is_ok_and(|m| m.is_dir()) {
            continue;
        }

        if ruta
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|nombre| {
                nombre == NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA
                    || nombre.starts_with('.')
                    || nombre.ends_with("-wal")
                    || nombre.ends_with("-shm")
                    || nombre.ends_with(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA)
            })
        {
            continue;
        }

        // Si es el symlink knowledge_live.db, se evalúa a través de su destino canónico
        if let Ok(meta_sym) = std::fs::symlink_metadata(&ruta)
            && meta_sym.file_type().is_symlink()
        {
            continue;
        }

        let conexion = match abrir_solo_lectura(&ruta) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let consulta: Result<(Option<i64>, Option<i64>), rusqlite::Error> = conexion.query_row(
            "SELECT numero_de_epoca, sellada_ms FROM metadatos_de_epoca WHERE id = 1",
            [],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        );
        drop(conexion);

        if let Ok((Some(num_epoca), Some(_sellada))) = consulta {
            let ruta_canonica = match std::fs::canonicalize(&ruta) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let es_viva =
                ruta_canonica == ruta_live_canonica || Some(num_epoca) == numero_vivo_intrinseco;

            candidatos.push(CandidatoDeEpoca {
                numero_de_epoca: num_epoca,
                ruta_archivo: ruta,
                es_viva,
            });
        }
    }

    // 6. Clasificación y cálculo de retención.
    // Épocas no vivas y no marcadas como sospechosas ordenadas descendentemente por número.
    let mut candidatos_sanos_no_vivos: Vec<i64> = candidatos
        .iter()
        .filter(|c| !c.es_viva && !numeros_marcados.contains(&c.numero_de_epoca))
        .map(|c| c.numero_de_epoca)
        .collect();
    candidatos_sanos_no_vivos.sort_unstable_by(|a, b| b.cmp(a));
    candidatos_sanos_no_vivos.dedup();

    let numeros_en_ventana: BTreeSet<i64> = candidatos_sanos_no_vivos
        .into_iter()
        .take(ventana_de_retencion)
        .collect();

    let mut epocas_conservadas = Vec::new();
    let mut epocas_purgadas = Vec::new();

    for candidato in candidatos {
        if candidato.es_viva {
            epocas_conservadas.push(EpocaConservada {
                numero_de_epoca: candidato.numero_de_epoca,
                ruta_del_archivo: candidato.ruta_archivo,
                motivo: MotivoDeConservacion::EsLaEpocaViva,
            });
        } else if en_uso.contains_key(&candidato.numero_de_epoca) {
            epocas_conservadas.push(EpocaConservada {
                numero_de_epoca: candidato.numero_de_epoca,
                ruta_del_archivo: candidato.ruta_archivo,
                motivo: MotivoDeConservacion::SuperseidaSinDrenar,
            });
        } else if numeros_en_ventana.contains(&candidato.numero_de_epoca) {
            epocas_conservadas.push(EpocaConservada {
                numero_de_epoca: candidato.numero_de_epoca,
                ruta_del_archivo: candidato.ruta_archivo,
                motivo: MotivoDeConservacion::DentroDeLaVentanaDeRetencion,
            });
        } else {
            // Candidata a purga: verificar si el diario WAL contiene datos no consolidados.
            let mut ruta_wal = candidato.ruta_archivo.as_os_str().to_owned();
            ruta_wal.push(SUFIJO_DE_ARCHIVO_WAL);
            let ruta_wal = PathBuf::from(ruta_wal);

            if let Ok(meta_wal) = std::fs::metadata(&ruta_wal) {
                let bytes = meta_wal.len();
                if bytes > 0 {
                    epocas_conservadas.push(EpocaConservada {
                        numero_de_epoca: candidato.numero_de_epoca,
                        ruta_del_archivo: candidato.ruta_archivo,
                        motivo: MotivoDeConservacion::DiarioConDatosSinConsolidar { bytes },
                    });
                    continue;
                }
            }

            // 7. Eliminación física acotada únicamente a la base, su -wal de 0 bytes y su -shm.
            std::fs::remove_file(&candidato.ruta_archivo).map_err(|causa| {
                ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
                    ruta: candidato.ruta_archivo.clone(),
                    operacion: "eliminar archivo de época sellada purgada",
                    causa,
                }
            })?;

            if ruta_wal.exists() {
                let _ = std::fs::remove_file(&ruta_wal);
            }

            let mut ruta_shm = candidato.ruta_archivo.as_os_str().to_owned();
            ruta_shm.push(SUFIJO_DE_ARCHIVO_SHM);
            let ruta_shm = PathBuf::from(ruta_shm);
            if ruta_shm.exists() {
                let _ = std::fs::remove_file(&ruta_shm);
            }

            epocas_purgadas.push(EpocaPurgada {
                numero_de_epoca: candidato.numero_de_epoca,
                ruta_del_archivo: candidato.ruta_archivo,
            });
        }
    }

    Ok(DesenlaceDePurga {
        epocas_purgadas,
        epocas_conservadas,
    })
}
