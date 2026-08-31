//! Secuencia de reversión de épocas para la base de conocimiento en producción.
//!
//! Este módulo implementa la conmutación segura hacia atrás hacia una época sellada previa,
//! condicionada a que la época destino re-supere tanto las verificaciones de integridad
//! estructural como la sonda semántica persistida en su propio archivo (`leer_sonda_semantica`).
//!
//! # Principios de diseño
//! 1. **Identidad intrínseca**: la reversión reutiliza el número y archivo existentes de la época destino;
//!    nunca acuña copias ni incrementa números, preservando la trazabilidad interna del archivo.
//! 2. **Exclusión mutua compartida**: toma `gestor.iniciar_promocion()` para garantizar que solo una
//!    conmutación (promoción o reversión) opere a la vez sobre el enlace simbólico y el `ArcSwap`.
//! 3. **Partición disjunta (AC-6)**: los motivos de rechazo se dividen de forma determinista y exhaustiva
//!    entre fallos estructurales e insuficiencia semántica, garantizando mutabilidad aislada en pruebas.
//! 4. **Inercia ante rechazo**: cualquier fallo aborta antes de abrir pools nuevos o reasignar enlaces;
//!    la producción permanece sirviendo la época viva previa sin alteraciones.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;

use crate::conocimiento::leer_sonda_semantica;
use crate::error::ErrorDeAlmacen;
use crate::pools::{GestorDePools, PoolDeConocimiento, verificar_enlace_vivo_resoluble};
use crate::promocion::{
    EpocaSuperseida, PREFIJO_DE_ARCHIVO_DE_EPOCA, reasignar_enlace_simbolico_vivo,
};
use crate::validacion::{MotivoDeRechazo, VeredictoDeIntegridad, validar_integridad_del_indice};

/// Determina si un motivo de rechazo de integridad es de naturaleza semántica o estructural.
///
/// Se evalúa con coincidencia exhaustiva sin comodín `_` para forzar que cualquier variante añadida
/// a [`MotivoDeRechazo`] en el futuro deba ser clasificada explícitamente en este punto.
pub fn es_motivo_semantico(motivo: &MotivoDeRechazo) -> bool {
    match motivo {
        MotivoDeRechazo::SimilitudInsuficiente { .. }
        | MotivoDeRechazo::VectoresIncomparables { .. }
        | MotivoDeRechazo::DimensionDeLaSondaDiscrepante { .. }
        | MotivoDeRechazo::SondaSemanticaOmitidaPorMetadatosAusentes => true,

        MotivoDeRechazo::MetadatosDeEpocaAusentes
        | MotivoDeRechazo::VectoresHuerfanos { .. }
        | MotivoDeRechazo::FaltaContiguidadOrdinal { .. }
        | MotivoDeRechazo::IndiceVacio
        | MotivoDeRechazo::DiferenciaDeFragmentos { .. }
        | MotivoDeRechazo::ConfiguracionDeFragmentacionInvalida { .. }
        | MotivoDeRechazo::DimensionDeVectorNoUniforme { .. }
        | MotivoDeRechazo::CalculoDeCoberturaOmitidoPorMetadatosAusentes
        | MotivoDeRechazo::CalculoDeDimensionOmitidoPorMetadatosAusentes => false,
    }
}

/// Motivo por el cual una reversión de época fue rechazada limpiamente.
#[derive(Clone, Debug, PartialEq)]
pub enum MotivoDeRechazoDeReversion {
    /// La base de datos destino carece de la fila de sonda semántica persistida.
    SondaAusente,
    /// La auditoría de integridad estructural rechazó el índice de la época destino.
    IntegridadEstructuralRechazada {
        /// Fallos estructurales detectados durante la validación del índice.
        motivos: Vec<MotivoDeRechazo>,
    },
    /// La auditoría semántica rechazó el índice destino por similitud insuficiente o inconsistencia de sonda.
    SondaSemanticaRechazada {
        /// Mayor valor de similitud coseno observado contra los fragmentos del índice.
        similitud_observada: f32,
        /// Límite mínimo requerido para la aprobación.
        umbral_requerido: f32,
    },
    /// La época destino solicitada es la que ya se encuentra actualmente activa en producción.
    EpocaYaEsLaViva {
        /// Número ordinal de la época que ya está viva.
        numero_de_epoca: i64,
    },
}

/// Resultado final de la ejecución de una secuencia de reversión a una época sellada previa.
#[derive(Clone, Debug, PartialEq)]
pub enum DesenlaceDeReversion {
    /// La época destino superó todas las validaciones y fue conmutada atómicamente a producción.
    Revertida {
        /// Número ordinal de la época a la que se revirtió.
        numero_de_epoca: i64,
        /// Ruta física del archivo de la época destino.
        ruta_del_archivo: PathBuf,
        /// Descriptor de la época superseída entregado vivo para su drenaje ordenado posterior.
        epoca_superseida: EpocaSuperseida,
        /// Latencia medida en milisegundos entre el swap y la primera lectura servida (NFR-03).
        duracion_de_conmutacion_ms: f64,
    },
    /// La reversión fue rechazada limpiamente por alguna compuerta de validación o estado del sistema.
    Rechazada {
        /// Causa descriptiva del rechazo limpio.
        motivo: MotivoDeRechazoDeReversion,
    },
}

/// Ejecuta la secuencia síncrona de reversión de la base de conocimiento a una época sellada previa.
///
/// La secuencia consta de las siguientes compuertas y pasos:
/// 1. Adquisición de la exclusión mutua de promoción (`gestor.iniciar_promocion()`).
/// 2. Verificación de enlace vivo resoluble (`verificar_enlace_vivo_resoluble`).
/// 3. Resolución de la ruta física de la época destino en disco (`knowledge_epoch_N.db`).
/// 4. Detección y rechazo de self-supersede si la época destino ya es la viva activa.
/// 5. Lectura y deserialización de la sonda semántica persistida en la época destino.
/// 6. Auditoría síncrona offline de integridad estructural y semántica del índice.
/// 7. Partición de motivos y rechazo temprano si se detectan anomalías estructurales o semánticas.
/// 8. Resolución canónica de la ruta viva previa antes de modificar el sistema de archivos.
/// 9. Precalentamiento del nuevo pool de lectura sobre la ruta explícita del archivo destino.
/// 10. Reasignación atómica del enlace simbólico `knowledge_live.db`.
/// 11. Reemplazo atómico del pool en memoria (`ArcSwap`), medición NFR-03 y entrega del descriptor superseído.
pub fn revertir_a_epoca(
    gestor: &GestorDePools,
    ruta_datos: &Path,
    configuracion_de_fragmentacion: &ConfiguracionDeFragmentacion,
    numero_destino: i64,
) -> Result<DesenlaceDeReversion, ErrorDeAlmacen> {
    // 1. Exclusión mutua: garantizar que ninguna otra promoción o reversión concurra.
    let _guardian = gestor.iniciar_promocion()?;

    // 2. Guarda contra enlace vivo colgante antes de evaluar el resto de condiciones.
    verificar_enlace_vivo_resoluble(ruta_datos)?;

    // 3. Localizar el archivo físico correspondiente a la época destino solicitada.
    let nombre_archivo_destino = format!("{PREFIJO_DE_ARCHIVO_DE_EPOCA}{numero_destino}.db");
    let ruta_destino = ruta_datos.join(&nombre_archivo_destino);
    if !ruta_destino.is_file() {
        return Err(ErrorDeAlmacen::EpocaDestinoAusente {
            numero_de_epoca: numero_destino,
            ruta: ruta_destino,
        });
    }

    // 4. Rechazar si el destino ya es el archivo activo para evitar auto-superseído.
    let ruta_destino_canonica = std::fs::canonicalize(&ruta_destino).map_err(|causa| {
        ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta_destino.clone(),
            operacion: "resolver la ruta física de la época destino",
            causa,
        }
    })?;
    let ruta_live_apertura = gestor.conocimiento().ruta().to_path_buf();
    let ruta_live_canonica = std::fs::canonicalize(&ruta_live_apertura).map_err(|causa| {
        ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta: ruta_live_apertura.clone(),
            operacion: "resolver la ruta física de la época viva actual",
            causa,
        }
    })?;
    if ruta_destino_canonica == ruta_live_canonica {
        return Ok(DesenlaceDeReversion::Rechazada {
            motivo: MotivoDeRechazoDeReversion::EpocaYaEsLaViva {
                numero_de_epoca: numero_destino,
            },
        });
    }

    // 5. Leer la sonda semántica persistida en el archivo destino.
    let sonda = match leer_sonda_semantica(&ruta_destino)? {
        Some(s) => s,
        None => {
            return Ok(DesenlaceDeReversion::Rechazada {
                motivo: MotivoDeRechazoDeReversion::SondaAusente,
            });
        }
    };

    // 6. Validar integridad estructural y semántica de la época destino.
    let veredicto =
        validar_integridad_del_indice(&ruta_destino, configuracion_de_fragmentacion, &sonda)?;
    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        // 7. Partición disjunta (AC-6): clasificar motivos en ramas disjuntas.
        // Precedencia: si hay cualquier motivo estructural, el veredicto es IntegridadEstructuralRechazada.
        // De lo contrario, si solo hay motivos semánticos, el veredicto es SondaSemanticaRechazada.
        let (motivos_semanticos, motivos_estructurales): (
            Vec<MotivoDeRechazo>,
            Vec<MotivoDeRechazo>,
        ) = motivos.into_iter().partition(es_motivo_semantico);

        if !motivos_estructurales.is_empty() {
            return Ok(DesenlaceDeReversion::Rechazada {
                motivo: MotivoDeRechazoDeReversion::IntegridadEstructuralRechazada {
                    motivos: motivos_estructurales,
                },
            });
        }

        let (similitud_observada, umbral_requerido) = motivos_semanticos
            .iter()
            .find_map(|m| match m {
                MotivoDeRechazo::SimilitudInsuficiente {
                    similitud_observada,
                    umbral_requerido,
                } => Some((*similitud_observada, *umbral_requerido)),
                _ => None,
            })
            .unwrap_or((0.0, sonda.umbral_de_aceptacion));

        return Ok(DesenlaceDeReversion::Rechazada {
            motivo: MotivoDeRechazoDeReversion::SondaSemanticaRechazada {
                similitud_observada,
                umbral_requerido,
            },
        });
    }

    // 8. Resolver la ruta física previa antes de modificar el enlace simbólico.
    let ruta_anterior = ruta_live_canonica;

    let pool_anterior = gestor.conocimiento();
    let numero_anterior: Option<i64> = pool_anterior
        .con_lectura(|conexion| {
            conexion
                .query_row(
                    "SELECT numero_de_epoca FROM metadatos_de_epoca WHERE id = 1",
                    [],
                    |fila| fila.get(0),
                )
                .map_err(ErrorDeAlmacen::en("leer número de época previa"))
        })
        .ok()
        .flatten();

    // 9. Precalentar las conexiones del nuevo pool sobre la ruta explícita del archivo destino.
    let nuevo_pool = Arc::new(PoolDeConocimiento::abrir_sobre(&ruta_destino)?);

    // 10. Reasignar atómicamente el enlace simbólico en vivo usando el modismo POSIX.
    reasignar_enlace_simbolico_vivo(ruta_datos, &nombre_archivo_destino)?;

    // 11. Conmutar atómicamente el puntero en memoria y cronometrar la latencia NFR-03.
    let instante_inicio = std::time::Instant::now();
    let pool_superseido = gestor.intercambiar_pool_de_conocimiento(Arc::clone(&nuevo_pool));

    let cuenta = nuevo_pool.con_lectura(|conexion| {
        conexion
            .query_row(
                "SELECT count(*) FROM metadatos_de_conocimiento",
                [],
                |fila| fila.get::<_, i64>(0),
            )
            .map_err(ErrorDeAlmacen::en(
                "verificar lectura inicial en nuevo pool tras reversión",
            ))
    })?;
    debug_assert_eq!(
        cuenta, 0,
        "la lectura de liveness contra el nuevo pool no devolvió el conteo esperado"
    );

    let duracion = instante_inicio.elapsed();
    let duracion_ms = duracion.as_secs_f64() * 1000.0;
    let duracion_ms = if duracion_ms.is_finite() {
        duracion_ms
    } else {
        f64::INFINITY
    };

    let epoca_superseida = EpocaSuperseida::nueva(
        pool_superseido,
        ruta_anterior,
        numero_anterior,
        instante_inicio,
    );

    Ok(DesenlaceDeReversion::Revertida {
        numero_de_epoca: numero_destino,
        ruta_del_archivo: ruta_destino,
        epoca_superseida,
        duracion_de_conmutacion_ms: duracion_ms,
    })
}
