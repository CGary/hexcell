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

/// Sufijo canónico del archivo de marca de una época sospechosa ya archivada por el operador.
///
/// Archivar no borra la marca: la renombra a este sufijo y le anexa la certificación
/// (`adr-0041`); el número de la época sigue reservado y este sufijo jamás se confunde con una
/// base de datos de época en los escaneos.
pub const SUFIJO_DE_MARCA_ARCHIVADA: &str = ".sospechosa.archivada";

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

/// Certificación del operador al archivar una marca: no vacía tras recortar y sin caracteres de
/// control (un salto de línea forjaría líneas del archivo); la valida [`archivar_marca_de_epoca_sospechosa`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificacionDeArchivo {
    /// Nombre o credencial del operador que certifica el archivo.
    pub certifico: String,
    /// Motivo por el cual se archiva la marca.
    pub motivo: String,
}

/// Certificación ya registrada en un archivo de marca archivada, con su fecha absoluta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificacionRegistrada {
    /// Nombre o credencial del operador que certificó el archivo.
    pub certifico: String,
    /// Motivo con el que se archivó la marca.
    pub motivo: String,
    /// Fecha absoluta ISO (YYYY-MM-DD) en que se anexó la certificación.
    pub fecha_absoluta: String,
}

/// Estado de una marca de época sospechosa en el listado administrativo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EstadoDeMarca {
    /// La marca sigue activa (`.sospechosa`) y pendiente de archivo.
    Vigente,
    /// La marca fue archivada (`.sospechosa.archivada`) con su certificación.
    Archivada,
    /// La marca no se pudo interpretar o su número discrepa; `error` nombra el variante
    /// de [`ErrorDeAlmacen`] que la produjo.
    Ilegible {
        /// `MarcaDeEpocaIlegible` o `NumeroDeMarcaDiscrepante`.
        error: &'static str,
    },
}

/// Entrada del listado tolerante: una marca ilegible no aborta; aparece como `Ilegible` con el
/// número del nombre cuando es interpretable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntradaDeMarcaListada {
    /// Número de época de la marca, o `None` si no se pudo derivar del nombre de archivo.
    pub numero_de_epoca: Option<i64>,
    /// Marca interpretada, ausente solo en las entradas ilegibles.
    pub marca: Option<MarcaDeEpocaSospechosa>,
    /// Estado de la marca: vigente, archivada o ilegible.
    pub estado: EstadoDeMarca,
    /// Certificación de archivo registrada, si la marca está archivada y la lleva.
    pub certificacion: Option<CertificacionRegistrada>,
}

/// Desenlace de un intento de archivo de una marca de época sospechosa.
///
/// El rechazo de una certificación inválida es un valor de resultado (precedente
/// [`crate::reversion::DesenlaceDeReversion::Rechazada`]), no un error de almacenamiento: la
/// capa HTTP lo traduce a 400.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesenlaceDeArchivoDeMarca {
    /// La marca activa fue renombrada a `.sospechosa.archivada` y la certificación anexada.
    Archivada {
        /// Ruta del archivo de marca archivada resultante.
        ruta: PathBuf,
        /// Certificación registrada, con la fecha absoluta de hoy.
        certificacion: CertificacionRegistrada,
    },
    /// Ya archivada con certificación: no se tocó ningún archivo (idempotencia).
    SinCambios {
        /// Ruta del archivo de marca archivada ya existente.
        ruta: PathBuf,
    },
    /// No existe ninguna marca (activa o archivada) para el número pedido.
    MarcaInexistente,
    /// La certificación no superó la validación; ningún archivo fue tocado.
    Rechazada {
        /// Motivo exhaustivo del rechazo.
        motivo: MotivoDeRechazoDeArchivo,
    },
}

/// Motivo exhaustivo por el cual una certificación de archivo fue rechazada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MotivoDeRechazoDeArchivo {
    /// `certifico` quedó vacío tras recortar espacios.
    CertificoVacio,
    /// `motivo` quedó vacío tras recortar espacios.
    MotivoVacio,
    /// El campo indicado contiene un carácter de control (p. ej. un salto de línea).
    CaracterDeControl {
        /// Campo infractor: `certifico` o `motivo`.
        campo: &'static str,
    },
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

/// Lee y valida todas las marcas de época sospechosa **activas** del directorio de datos.
///
/// Aborta ante la primera marca mala. Las archivadas las cuenta [`numeros_de_epoca_marcados`].
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

        let (marca, _certificacion) =
            interpretar_archivo_de_marca(&ruta, nombre, SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA)?;
        marcas.push(marca);
    }

    Ok(marcas)
}

/// Interpreta **un** archivo de marca dado el sufijo de su nombre (activo o archivado).
///
/// Parser compartido por el lector estricto, el listado tolerante y el contador de números
/// reservados. Lee además las líneas opcionales de certificación; una a medias es ilegible.
fn interpretar_archivo_de_marca(
    ruta: &Path,
    nombre: &str,
    sufijo: &str,
) -> Result<(MarcaDeEpocaSospechosa, Option<CertificacionRegistrada>), ErrorDeAlmacen> {
    if !nombre.starts_with(PREFIJO_DE_ARCHIVO_DE_EPOCA) {
        return Err(ErrorDeAlmacen::MarcaDeEpocaIlegible {
            ruta: ruta.to_path_buf(),
            motivo: format!(
                "el archivo {nombre} no inicia con el prefijo canónico {PREFIJO_DE_ARCHIVO_DE_EPOCA}"
            ),
        });
    }

    let parte_numero = &nombre[PREFIJO_DE_ARCHIVO_DE_EPOCA.len()..nombre.len() - sufijo.len()];
    let numero_en_nombre: i64 =
        parte_numero
            .parse()
            .map_err(|_| ErrorDeAlmacen::MarcaDeEpocaIlegible {
                ruta: ruta.to_path_buf(),
                motivo: format!("no se pudo interpretar el número en el nombre {nombre}"),
            })?;

    let contenido =
        std::fs::read_to_string(ruta).map_err(|causa| ErrorDeAlmacen::MarcaDeEpocaIlegible {
            ruta: ruta.to_path_buf(),
            motivo: format!("fallo al leer el archivo de marca: {causa}"),
        })?;

    let mut numero_en_contenido: Option<i64> = None;
    let mut motivo_opt: Option<String> = None;
    let mut fecha_opt: Option<String> = None;
    let mut certifico_opt: Option<String> = None;
    let mut motivo_de_certificacion_opt: Option<String> = None;
    let mut fecha_de_certificacion_opt: Option<String> = None;

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
        } else if let Some(resto) = linea.strip_prefix("certificacion_certifico:") {
            certifico_opt = Some(resto.trim().to_string());
        } else if let Some(resto) = linea.strip_prefix("certificacion_motivo:") {
            motivo_de_certificacion_opt = Some(resto.trim().to_string());
        } else if let Some(resto) = linea.strip_prefix("certificacion_fecha_absoluta:") {
            fecha_de_certificacion_opt = Some(resto.trim().to_string());
        }
    }

    let Some(num_contenido) = numero_en_contenido else {
        return Err(ErrorDeAlmacen::MarcaDeEpocaIlegible {
            ruta: ruta.to_path_buf(),
            motivo: "campo numero_de_epoca ausente o inválido en el contenido de la marca"
                .to_string(),
        });
    };

    if numero_en_nombre != num_contenido {
        return Err(ErrorDeAlmacen::NumeroDeMarcaDiscrepante {
            ruta: ruta.to_path_buf(),
            numero_en_nombre,
            numero_en_contenido: num_contenido,
        });
    }

    let certificacion = match (
        certifico_opt,
        motivo_de_certificacion_opt,
        fecha_de_certificacion_opt,
    ) {
        (Some(certifico), Some(motivo), Some(fecha_absoluta)) => Some(CertificacionRegistrada {
            certifico,
            motivo,
            fecha_absoluta,
        }),
        (None, None, None) => None,
        _ => {
            return Err(ErrorDeAlmacen::MarcaDeEpocaIlegible {
                ruta: ruta.to_path_buf(),
                motivo: "certificación de archivo incompleta en el contenido de la marca"
                    .to_string(),
            });
        }
    };

    Ok((
        MarcaDeEpocaSospechosa {
            numero_de_epoca: num_contenido,
            motivo: motivo_opt.unwrap_or_default(),
            fecha_absoluta: fecha_opt.unwrap_or_default(),
        },
        certificacion,
    ))
}

/// Número de época derivado del nombre de un archivo de marca, si es interpretable.
fn numero_de_marca_desde_nombre(nombre: &str, sufijo: &str) -> Option<i64> {
    if !nombre.starts_with(PREFIJO_DE_ARCHIVO_DE_EPOCA) {
        return None;
    }
    let parte_numero = &nombre[PREFIJO_DE_ARCHIVO_DE_EPOCA.len()..nombre.len() - sufijo.len()];
    parte_numero.parse().ok()
}

/// Lista todas las marcas de época sospechosa —activas y archivadas— sin abortar ante una mala.
///
/// Alimenta `GET /admin/epocas/sospechosas`: una marca mala aparece como `Ilegible` y el resto
/// sigue; solo un fallo de `read_dir` es error.
pub fn listar_marcas_de_epoca_sospechosa(
    ruta_datos: &Path,
) -> Result<Vec<EntradaDeMarcaListada>, ErrorDeAlmacen> {
    let entradas =
        std::fs::read_dir(ruta_datos).map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_datos.to_path_buf(),
            causa,
        })?;

    let mut marcas: Vec<EntradaDeMarcaListada> = Vec::new();

    for entrada_res in entradas {
        let entrada = match entrada_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ruta = entrada.path();
        if ruta.is_dir() {
            continue;
        }
        let Some(nombre) = ruta.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // El sufijo archivado se comprueba primero: `.sospechosa.archivada` no termina en
        // `.sospechosa`, pero el orden protege el caso de un nombre con ambos sufijos.
        let (sufijo, archivada) = if nombre.ends_with(SUFIJO_DE_MARCA_ARCHIVADA) {
            (SUFIJO_DE_MARCA_ARCHIVADA, true)
        } else if nombre.ends_with(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA) {
            (SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA, false)
        } else {
            continue;
        };

        match interpretar_archivo_de_marca(&ruta, nombre, sufijo) {
            Ok((marca, certificacion)) => {
                marcas.push(EntradaDeMarcaListada {
                    numero_de_epoca: Some(marca.numero_de_epoca),
                    estado: if archivada {
                        EstadoDeMarca::Archivada
                    } else {
                        EstadoDeMarca::Vigente
                    },
                    certificacion,
                    marca: Some(marca),
                });
            }
            Err(ErrorDeAlmacen::MarcaDeEpocaIlegible { .. }) => {
                marcas.push(EntradaDeMarcaListada {
                    numero_de_epoca: numero_de_marca_desde_nombre(nombre, sufijo),
                    estado: EstadoDeMarca::Ilegible {
                        error: "MarcaDeEpocaIlegible",
                    },
                    certificacion: None,
                    marca: None,
                });
            }
            Err(ErrorDeAlmacen::NumeroDeMarcaDiscrepante { .. }) => {
                marcas.push(EntradaDeMarcaListada {
                    numero_de_epoca: numero_de_marca_desde_nombre(nombre, sufijo),
                    estado: EstadoDeMarca::Ilegible {
                        error: "NumeroDeMarcaDiscrepante",
                    },
                    certificacion: None,
                    marca: None,
                });
            }
            Err(otro) => return Err(otro),
        }
    }

    marcas.sort_by_key(|entrada| entrada.numero_de_epoca.unwrap_or(i64::MAX));
    Ok(marcas)
}

/// Archiva una marca de época sospechosa con la certificación del operador.
///
/// Valida primero la certificación; si la activa existe y la archivada no, renombra a
/// `.sospechosa.archivada` y anexa las tres líneas de certificación con `OpenOptions::append`
/// (nunca trunca, sin forzar la sincronía con el disco). La marca **nunca se borra**; el número
/// sigue reservado. Ya archivada con certificación → `SinCambios`; sin certificación → solo anexa
/// → `Archivada`; ambos presentes → error sin renombrar; ninguno → `MarcaInexistente`.
pub fn archivar_marca_de_epoca_sospechosa(
    ruta_datos: &Path,
    numero_de_epoca: i64,
    certificacion: &CertificacionDeArchivo,
) -> Result<DesenlaceDeArchivoDeMarca, ErrorDeAlmacen> {
    // 1. Validación previa: ningún acceso al sistema de archivos antes de este punto.
    if certificacion.certifico.trim().is_empty() {
        return Ok(DesenlaceDeArchivoDeMarca::Rechazada {
            motivo: MotivoDeRechazoDeArchivo::CertificoVacio,
        });
    }
    if certificacion.motivo.trim().is_empty() {
        return Ok(DesenlaceDeArchivoDeMarca::Rechazada {
            motivo: MotivoDeRechazoDeArchivo::MotivoVacio,
        });
    }
    // Un carácter de control (p. ej. un salto de línea) forjaría líneas del archivo de marca.
    if certificacion.certifico.chars().any(char::is_control) {
        return Ok(DesenlaceDeArchivoDeMarca::Rechazada {
            motivo: MotivoDeRechazoDeArchivo::CaracterDeControl { campo: "certifico" },
        });
    }
    if certificacion.motivo.chars().any(char::is_control) {
        return Ok(DesenlaceDeArchivoDeMarca::Rechazada {
            motivo: MotivoDeRechazoDeArchivo::CaracterDeControl { campo: "motivo" },
        });
    }

    let nombre_activa = format!(
        "{PREFIJO_DE_ARCHIVO_DE_EPOCA}{numero_de_epoca}{SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA}"
    );
    let nombre_archivada =
        format!("{PREFIJO_DE_ARCHIVO_DE_EPOCA}{numero_de_epoca}{SUFIJO_DE_MARCA_ARCHIVADA}");
    let ruta_activa = ruta_datos.join(&nombre_activa);
    let ruta_archivada = ruta_datos.join(&nombre_archivada);

    match (ruta_activa.exists(), ruta_archivada.exists()) {
        // Ambos presentes: abortar sin renombrar — rename() de POSIX sobrescribiría la marca
        // archivada ya certificada.
        (true, true) => Err(ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta_archivada,
            operacion: "archivar marca de época sospechosa con ambos archivos presentes",
            causa: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "la marca archivada ya existe y la activa sigue presente; el renombrado sobrescribiría",
            ),
        }),
        (true, false) => {
            std::fs::rename(&ruta_activa, &ruta_archivada).map_err(|causa| {
                ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
                    ruta: ruta_archivada.clone(),
                    operacion: "renombrar marca de época sospechosa a archivada",
                    causa,
                }
            })?;
            let registrada = CertificacionRegistrada {
                certifico: certificacion.certifico.clone(),
                motivo: certificacion.motivo.clone(),
                fecha_absoluta: crate::reversion::fecha_absoluta_de_hoy(),
            };
            anexar_certificacion_de_archivo(&ruta_archivada, &registrada)?;
            Ok(DesenlaceDeArchivoDeMarca::Archivada {
                ruta: ruta_archivada,
                certificacion: registrada,
            })
        }
        (false, true) => {
            let (_, certificacion_existente) = interpretar_archivo_de_marca(
                &ruta_archivada,
                &nombre_archivada,
                SUFIJO_DE_MARCA_ARCHIVADA,
            )?;
            if certificacion_existente.is_some() {
                // Idempotencia: ya archivada y certificada, no se toca ningún archivo.
                Ok(DesenlaceDeArchivoDeMarca::SinCambios {
                    ruta: ruta_archivada,
                })
            } else {
                // Ventana de caída: el renombrado ocurrió pero el anexo no; completar el anexo.
                let registrada = CertificacionRegistrada {
                    certifico: certificacion.certifico.clone(),
                    motivo: certificacion.motivo.clone(),
                    fecha_absoluta: crate::reversion::fecha_absoluta_de_hoy(),
                };
                anexar_certificacion_de_archivo(&ruta_archivada, &registrada)?;
                Ok(DesenlaceDeArchivoDeMarca::Archivada {
                    ruta: ruta_archivada,
                    certificacion: registrada,
                })
            }
        }
        (false, false) => Ok(DesenlaceDeArchivoDeMarca::MarcaInexistente),
    }
}

/// Anexa las tres líneas de certificación al final de un archivo de marca archivada.
///
/// Solo anexa, nunca trunca ni reescribe el contenido original, sin forzar la sincronía con el
/// disco: el criterio del escritor existente.
fn anexar_certificacion_de_archivo(
    ruta: &Path,
    certificacion: &CertificacionRegistrada,
) -> Result<(), ErrorDeAlmacen> {
    use std::io::Write;

    let lineas = format!(
        "certificacion_certifico: {}\ncertificacion_motivo: {}\ncertificacion_fecha_absoluta: {}\n",
        certificacion.certifico, certificacion.motivo, certificacion.fecha_absoluta
    );
    let mut archivo = std::fs::OpenOptions::new()
        .append(true)
        .open(ruta)
        .map_err(|causa| ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta.to_path_buf(),
            operacion: "abrir marca archivada para anexar la certificación",
            causa,
        })?;
    archivo.write_all(lineas.as_bytes()).map_err(|causa| {
        ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta.to_path_buf(),
            operacion: "anexar la certificación de archivo a la marca",
            causa,
        }
    })?;
    Ok(())
}

/// Extrae el conjunto de números ordinales de todas las épocas con marca de sospecha válida,
/// **activas y archivadas**.
///
/// Una marca archivada nunca libera el número de su época: ni la purga ni
/// `numero_de_epoca_siguiente` lo reutilizan. Fuente única de los números reservados.
pub fn numeros_de_epoca_marcados(ruta_datos: &Path) -> Result<BTreeSet<i64>, ErrorDeAlmacen> {
    let mut numeros: BTreeSet<i64> = leer_marcas_de_epoca_sospechosa(ruta_datos)?
        .into_iter()
        .map(|marca| marca.numero_de_epoca)
        .collect();
    // Las archivadas se leen estrictamente: una corrupta aborta igual que una activa.
    let entradas =
        std::fs::read_dir(ruta_datos).map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_datos.to_path_buf(),
            causa,
        })?;
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
        if !nombre.ends_with(SUFIJO_DE_MARCA_ARCHIVADA) {
            continue;
        }
        let (marca, _certificacion) =
            interpretar_archivo_de_marca(&ruta, nombre, SUFIJO_DE_MARCA_ARCHIVADA)?;
        numeros.insert(marca.numero_de_epoca);
    }
    Ok(numeros)
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

/// Decide si un nombre de archivo es ajeno al escaneo de épocas selladas.
///
/// Vocabulario compartido de los dos escaneos: staging, ocultos, diarios `-wal`/`-shm` y marcas
/// —activas y **archivadas**— quedan fuera; una archivada jamás se confunde con una base.
pub(crate) fn es_nombre_ajeno_al_escaneo_de_epocas(nombre: &str) -> bool {
    nombre == NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA
        || nombre.starts_with('.')
        || nombre.ends_with("-wal")
        || nombre.ends_with("-shm")
        || nombre.ends_with(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA)
        || nombre.ends_with(SUFIJO_DE_MARCA_ARCHIVADA)
}

/// Lista las rutas candidatas a escanear en la purga de épocas retiradas.
///
/// Único punto del sitio de purga que lista y filtra nombres: el escaneo posterior itera esta
/// lista y deja en el bucle el salto de enlaces simbólicos, la apertura SQLite y la consulta.
/// Un fallo de `read_dir` mapea a [`ErrorDeAlmacen::RutaDeDatosInaccesible`].
pub(crate) fn rutas_de_epoca_a_escanear_en_purga(
    ruta_datos: &Path,
) -> Result<Vec<PathBuf>, ErrorDeAlmacen> {
    let entradas =
        std::fs::read_dir(ruta_datos).map_err(|causa| ErrorDeAlmacen::RutaDeDatosInaccesible {
            ruta: ruta_datos.to_path_buf(),
            causa,
        })?;

    let mut rutas = Vec::new();
    for entrada_res in entradas {
        let entrada = match entrada_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ruta = entrada.path();
        if std::fs::metadata(&ruta).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let Some(nombre) = ruta.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if es_nombre_ajeno_al_escaneo_de_epocas(nombre) {
            continue;
        }
        rutas.push(ruta);
    }
    Ok(rutas)
}

/// Ejecuta la purga síncrona de épocas selladas retiradas fuera de la ventana de retención.
///
/// Secuencia: 1) exclusión mutua; 2) enlace vivo resoluble; 3) número intrínseco de la viva;
/// 4) `epocas_en_uso` y números reservados por [`numeros_de_epoca_marcados`] (fuente única,
/// activas y archivadas); 5) escaneo de selladas; 6) clasificación respetando las invariantes;
/// 7) borrado solo del `.db`, `-wal` de cero bytes y `-shm`, conservando `-wal` con datos.
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

    // 4. Cargar el registro de épocas en uso y los números reservados (fuente única, activas y
    //    archivadas: la época archivada sigue sin protección de recencia y su número reservado).
    let en_uso = gestor.epocas_en_uso();
    let numeros_marcados = numeros_de_epoca_marcados(ruta_datos)?;

    // 5. Escanear archivos de época sellados en disco.
    let mut candidatos: Vec<CandidatoDeEpoca> = Vec::new();

    for ruta in rutas_de_epoca_a_escanear_en_purga(ruta_datos)? {
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

            // El brazo de ruta canónica es hoy inatacable por mutación en aislamiento: la restricción
            // CHECK ((numero_de_epoca IS NULL) = (sellada_ms IS NULL)) de
            // migraciones/conocimiento/0002-esquema-de-conocimiento.sql:103 liga ambas columnas, así
            // que el único archivo cuya ruta canónica puede igualar a ruta_live_canonica es
            // precisamente aquel del que numero_vivo_intrinseco ya se leyó como Some desde esa misma
            // fila; el brazo numérico queda entonces necesariamente verdadero también, y mutar M4
            // (borrar este brazo) da cero pruebas fallidas POR CONSTRUCCIÓN, no por falta de cobertura.
            // Se conserva deliberadamente como defensa en profundidad para el día en que una
            // migración futura desacople esa identidad intrínseca de la ruta física: borrar una
            // guarda por ser hoy inalcanzable es exactamente lo que muerde después de ese cambio.
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
#[cfg(test)]
mod pruebas {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SECUENCIA: AtomicUsize = AtomicUsize::new(0);

    /// Directorio temporal único por (etiqueta, pid, secuencia). No se borra al salir de
    /// alcance: la guarda estática de HEX-093 cuenta el verbo de eliminación en este archivo.
    fn dir(etiqueta: &str) -> PathBuf {
        let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
        let ruta = std::env::temp_dir().join(format!(
            "hexcell-storage-retencion-{etiqueta}-{}-{secuencia}",
            std::process::id()
        ));
        std::fs::create_dir_all(&ruta).expect("crear el directorio temporal de la prueba");
        ruta
    }

    fn marca(d: &Path, n: i64, motivo: &str) {
        escribir_marca_de_epoca_sospechosa(d, n, motivo, "2026-09-30").expect("marca");
    }

    fn marca_corrupta(d: &Path, nombre: &str, numero: i64) {
        std::fs::write(d.join(nombre), format!("numero_de_epoca: {numero}\n")).expect("corrupta");
    }

    fn certificacion() -> CertificacionDeArchivo {
        CertificacionDeArchivo {
            certifico: "operador de turno".into(),
            motivo: "época defectuosa archivada".into(),
        }
    }

    fn archivar(d: &Path, n: i64) {
        archivar_marca_de_epoca_sospechosa(d, n, &certificacion()).expect("archivar");
    }

    fn nombres(rutas: &[PathBuf]) -> Vec<String> {
        rutas
            .iter()
            .filter_map(|r| r.file_name().and_then(|n| n.to_str()).map(String::from))
            .collect()
    }

    #[test]
    fn listar_marcas_vacio_y_vigente() {
        let d = dir("listar-vacio");
        assert!(
            listar_marcas_de_epoca_sospechosa(&d)
                .expect("listar")
                .is_empty()
        );

        marca(&d, 7, "defecto de prueba");
        let e = &listar_marcas_de_epoca_sospechosa(&d).expect("listar")[0];
        assert_eq!(e.numero_de_epoca, Some(7));
        assert!(matches!(e.estado, EstadoDeMarca::Vigente));
        assert!(e.certificacion.is_none());
        assert_eq!(e.marca.as_ref().unwrap().motivo, "defecto de prueba");
    }

    #[test]
    fn listar_marcas_reporta_ilegible_sin_abortar() {
        let d = dir("listar-ilegible");
        marca(&d, 2, "sana");
        marca_corrupta(&d, "knowledge_epoch_1.sospechosa", 99);
        marca_corrupta(&d, "knowledge_epoch_x.sospechosa", 1);

        let es = listar_marcas_de_epoca_sospechosa(&d).expect("listado tolerante");
        assert_eq!(es.len(), 3);
        assert!(es.iter().any(|e| e.numero_de_epoca == Some(2)));
        let ilegibles: Vec<_> = es
            .iter()
            .filter(|e| matches!(e.estado, EstadoDeMarca::Ilegible { .. }))
            .collect();
        assert_eq!(ilegibles.len(), 2);
        let pares: Vec<_> = ilegibles
            .iter()
            .map(|e| match e.estado {
                EstadoDeMarca::Ilegible { error } => (error, e.numero_de_epoca),
                _ => ("", None),
            })
            .collect();
        assert_eq!(pares[0], ("NumeroDeMarcaDiscrepante", Some(1)));
        assert_eq!(pares[1], ("MarcaDeEpocaIlegible", None));
    }

    #[test]
    fn archivar_renombra_y_conserva_contenido_original_con_certificacion() {
        let d = dir("archivar-renombra");
        marca(&d, 3, "defecto de prueba");

        let activa = d.join("knowledge_epoch_3.sospechosa");
        let archivada = d.join("knowledge_epoch_3.sospechosa.archivada");
        let original = std::fs::read(&activa).expect("leer marca activa");

        let DesenlaceDeArchivoDeMarca::Archivada {
            ruta,
            certificacion,
        } = archivar_marca_de_epoca_sospechosa(&d, 3, &certificacion()).expect("archivar")
        else {
            panic!("se esperaba Archivada")
        };
        assert_eq!(ruta, archivada);
        assert_eq!(certificacion.certifico, "operador de turno");
        assert_eq!(certificacion.motivo, "época defectuosa archivada");
        assert!(!certificacion.fecha_absoluta.is_empty());

        assert!(!activa.exists());
        let contenido = std::fs::read_to_string(&archivada).expect("leer archivada");
        let esperado = std::str::from_utf8(&original).expect("el original es UTF-8");
        assert!(contenido.starts_with(esperado));
        assert!(contenido.contains("certificacion_certifico: operador de turno"));
        assert!(contenido.contains("certificacion_motivo: época defectuosa archivada"));
        assert!(contenido.contains("certificacion_fecha_absoluta: 20"));
    }

    #[test]
    fn archivar_reejecutado_devuelve_sin_cambios_y_no_toca_el_archivo() {
        let d = dir("archivar-sin-cambios");
        marca(&d, 3, "defecto de prueba");
        archivar(&d, 3);
        let archivada = d.join("knowledge_epoch_3.sospechosa.archivada");
        let bytes_1 = std::fs::read(&archivada).expect("leer archivada");

        assert!(matches!(
            archivar_marca_de_epoca_sospechosa(&d, 3, &certificacion()).expect("segundo archivo"),
            DesenlaceDeArchivoDeMarca::SinCambios { .. }
        ));
        assert_eq!(
            std::fs::read(&archivada).expect("releer archivada"),
            bytes_1,
            "el archivo archivado queda byte a byte idéntico"
        );
    }

    #[test]
    fn archivar_marca_inexistente_devuelve_marca_inexistente() {
        let d = dir("archivar-inexistente");
        assert!(matches!(
            archivar_marca_de_epoca_sospechosa(&d, 9, &certificacion()).expect("archivar"),
            DesenlaceDeArchivoDeMarca::MarcaInexistente
        ));
        assert!(!d.join("knowledge_epoch_9.sospechosa.archivada").exists());
    }

    #[test]
    fn archivar_con_certificacion_vacia_se_rechaza_sin_tocar_archivos() {
        let d = dir("archivar-rechazo");
        marca(&d, 3, "defecto de prueba");

        let activa = d.join("knowledge_epoch_3.sospechosa");
        let original = std::fs::read(&activa).expect("leer marca activa");
        let archivada = d.join("knowledge_epoch_3.sospechosa.archivada");

        for (certifico, motivo) in [
            ("", "motivo"),
            ("   ", "motivo"),
            ("operador", ""),
            ("ope\nrador", "motivo"),
        ] {
            let caso = CertificacionDeArchivo {
                certifico: certifico.into(),
                motivo: motivo.into(),
            };
            let desenlace = archivar_marca_de_epoca_sospechosa(&d, 3, &caso)
                .expect("la certificación inválida se rechaza como valor");
            assert!(matches!(
                desenlace,
                DesenlaceDeArchivoDeMarca::Rechazada { .. }
            ));
            assert_eq!(
                std::fs::read(&activa).expect("marca activa intacta"),
                original
            );
            assert!(!archivada.exists());
        }
    }

    #[test]
    fn numeros_marcados_cuenta_vigentes_y_archivadas() {
        let d = dir("numeros-marcados");
        marca(&d, 3, "defecto de prueba");
        marca(&d, 5, "otro defecto");
        archivar(&d, 5);

        let numeros = numeros_de_epoca_marcados(&d).expect("números marcados");
        assert_eq!(numeros, BTreeSet::from([3, 5]));
    }

    #[test]
    fn escaneo_de_epocas_excluye_marcas_archivadas() {
        let ajeno = es_nombre_ajeno_al_escaneo_de_epocas;
        assert!(ajeno("knowledge_epoch_3.sospechosa.archivada"));
        assert!(ajeno("knowledge_epoch_3.sospechosa"));
        assert!(!ajeno("knowledge_epoch_3.db"));
    }

    fn con_marca_archivada(etiqueta: &str) -> PathBuf {
        let d = dir(etiqueta);
        std::fs::write(d.join("knowledge_epoch_1.db"), "no es sqlite").expect("db");
        marca(&d, 3, "defecto de prueba");
        archivar(&d, 3);
        d
    }

    #[test]
    fn escaneo_de_purga_no_considera_marcas_archivadas() {
        let d = con_marca_archivada("escaneo-purga");
        let rutas = rutas_de_epoca_a_escanear_en_purga(&d).expect("listado de purga");
        assert_eq!(
            nombres(&rutas),
            vec!["knowledge_epoch_1.db".to_string()],
            "ni la marca activa ni la archivada pueden colarse como épocas en la purga"
        );
    }

    #[test]
    fn escaneo_de_numeracion_no_considera_marcas_archivadas() {
        let d = con_marca_archivada("escaneo-numeracion");
        let rutas = crate::promocion::rutas_de_epoca_a_escanear_para_numerar(&d)
            .expect("listado de numeración");
        assert_eq!(
            nombres(&rutas),
            vec!["knowledge_epoch_1.db".to_string()],
            "ni la marca activa ni la archivada pueden colarse como épocas en la numeración"
        );
    }
}
