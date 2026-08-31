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

use crate::conocimiento::{inspeccionar_base_en_sombra, leer_sonda_semantica};
use crate::error::ErrorDeAlmacen;
use crate::pools::{GestorDePools, PoolDeConocimiento, verificar_enlace_vivo_resoluble};
use crate::promocion::{
    CONTEO_ESPERADO_DE_METADATOS_DE_CONOCIMIENTO, EpocaSuperseida, PREFIJO_DE_ARCHIVO_DE_EPOCA,
    reasignar_enlace_simbolico_vivo,
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
    /// El número de época persistido dentro del archivo destino no coincide con el número
    /// solicitado por nombre de archivo. La identidad de una época es intrínseca al contenido
    /// del archivo, no a su nombre: un respaldo restaurado puede renombrar `knowledge_epoch_N.db`
    /// sin tocar el número que lleva grabado adentro, y sería el defecto de HEX-054 servir esa
    /// época bajo el número equivocado en vez de detectar la discrepancia aquí.
    NumeroDeEpocaIntrinsecoDiscrepante {
        /// Número solicitado, derivado del nombre del archivo `knowledge_epoch_N.db`.
        numero_solicitado: i64,
        /// Número leído desde `metadatos_de_epoca` dentro del archivo destino, si pudo leerse.
        numero_leido: Option<i64>,
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
        /// Latencia medida en milisegundos entre la conmutación atómica del pool y la primera
        /// lectura servida (NFR-03).
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
/// 4. Verificación de que el número de época grabado dentro del archivo coincide con el
///    solicitado por nombre, porque la identidad de una época es intrínseca al contenido.
/// 5. Detección y rechazo de re-superseído propio si la época destino ya es la viva activa.
/// 6. Lectura y deserialización de la sonda semántica persistida en la época destino.
/// 7. Auditoría síncrona offline de integridad estructural y semántica del índice.
/// 8. Partición de motivos y rechazo temprano si se detectan anomalías estructurales o semánticas.
/// 9. Resolución canónica de la ruta viva previa antes de modificar el sistema de archivos.
/// 10. Precalentamiento del nuevo pool de lectura sobre la ruta explícita del archivo destino.
/// 11. Reasignación atómica del enlace simbólico `knowledge_live.db`.
/// 12. Reemplazo atómico del pool en memoria (`ArcSwap`), medición NFR-03 y entrega del descriptor superseído.
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

    // 3. El nombre de archivo es solo la clave de búsqueda: no hay un índice previo de épocas
    // selladas que consultar, así que hace falta construirlo por convención para encontrar el
    // candidato. Su número interno, la fuente de verdad real, se audita en el paso siguiente.
    let nombre_archivo_destino = format!("{PREFIJO_DE_ARCHIVO_DE_EPOCA}{numero_destino}.db");
    let ruta_destino = ruta_datos.join(&nombre_archivo_destino);
    if !ruta_destino.is_file() {
        return Err(ErrorDeAlmacen::EpocaDestinoAusente {
            numero_de_epoca: numero_destino,
            ruta: ruta_destino,
        });
    }

    // 4. La identidad de una época vive en su propio contenido, no en su nombre: un respaldo
    // restaurado puede renombrar knowledge_epoch_N.db sin tocar el número grabado adentro, y sería
    // exactamente el defecto que HEX-054 vino a prevenir servir esa época bajo el número
    // equivocado en vez de detectar aquí la discrepancia. Se reutiliza la misma inspección de solo
    // lectura que ya usa la auditoría de integridad (`inspeccionar_base_en_sombra`) en vez de abrir
    // una segunda conexión paralela solo para leer una columna.
    let resumen_destino = inspeccionar_base_en_sombra(&ruta_destino)?;
    let numero_leido = resumen_destino
        .metadatos_de_epoca
        .and_then(|metadatos| metadatos.numero_de_epoca);
    let numero_confirmado = match numero_leido {
        Some(numero) if numero == numero_destino => numero,
        _ => {
            return Ok(DesenlaceDeReversion::Rechazada {
                motivo: MotivoDeRechazoDeReversion::NumeroDeEpocaIntrinsecoDiscrepante {
                    numero_solicitado: numero_destino,
                    numero_leido,
                },
            });
        }
    };

    // 5. Rechazar si el destino ya es el archivo activo: revertir a la propia época viva no
    // conmuta nada y encubriría un no-op como si fuese una reversión real.
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

    // 6. La auditoría de integridad no puede evaluar similitud semántica sin la sonda: una época
    // sellada antes de que existiera la sonda persistida (o cuya fila se perdió) debe rechazarse
    // aquí, barato y sin abrir el índice completo, en vez de fallar más adelante a mitad de la
    // validación estructural.
    let sonda = match leer_sonda_semantica(&ruta_destino)? {
        Some(s) => s,
        None => {
            return Ok(DesenlaceDeReversion::Rechazada {
                motivo: MotivoDeRechazoDeReversion::SondaAusente,
            });
        }
    };

    // 7. La auditoría corre offline, sin pool abierto ni enlace tocado, para que un índice
    // corrupto o semánticamente insuficiente se detecte y rechace antes de comprometer producción
    // con datos potencialmente inválidos.
    let veredicto =
        validar_integridad_del_indice(&ruta_destino, configuracion_de_fragmentacion, &sonda)?;
    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        // 8. Partición disjunta (AC-6): clasificar motivos en ramas disjuntas.
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

    // 9. Se resuelve la ruta previa aquí, con la variable ya canónica de la compuerta 5, para no
    // recalcular la canonicalización una vez que el sistema de archivos está por mutarse.
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

    // 10. El pool se abre y precalienta ANTES de reasignar el enlace para que la ventana de
    // conmutación observable sea solo el rename atómico del paso siguiente; abrir conexiones
    // después dejaría el enlace apuntando momentáneamente a un archivo cuyo pool aún no responde.
    let nuevo_pool = Arc::new(PoolDeConocimiento::abrir_sobre(&ruta_destino)?);

    // 11. Se reutiliza el helper extraído de promover_epoca (D-29) en vez de duplicar el modismo
    // unlink+symlink, porque un rename atómico nunca deja una ventana en la que el enlace resuelva
    // a nada, mientras que unlink seguido de symlink sí la deja.
    reasignar_enlace_simbolico_vivo(ruta_datos, &nombre_archivo_destino)?;

    // 12. El intercambio se mide con reloj monótono inmediatamente después del ArcSwap porque
    // NFR-03 exige la latencia real percibida por el primer lector, no un estimado posterior.
    let instante_inicio = std::time::Instant::now();
    let pool_superseido = gestor.intercambiar_pool_de_conocimiento(Arc::clone(&nuevo_pool));

    // Primera lectura efectiva contra el nuevo pool para asegurar operatividad inmediata. La
    // aserción de NFR-03 debe ser de DOS lados: no basta con que la lectura no falle, tiene que
    // devolver el conteo esperado, porque una lectura que erró y una que devolvió lo esperado
    // transcurren igual de rápido y solo el valor distingue una medición real de una vacía.
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
        cuenta, CONTEO_ESPERADO_DE_METADATOS_DE_CONOCIMIENTO,
        "la lectura de liveness contra el nuevo pool no devolvió el conteo esperado"
    );

    let duracion = instante_inicio.elapsed();
    let duracion_ms = duracion.as_secs_f64() * 1000.0;
    // Un Duration nunca es NaN, así que este caso es en la práctica inalcanzable; pero si algún
    // día lo fuera, reportar un número imposible como si fuese perfecto ocultaría la anomalía en
    // vez de mostrarla. Se propaga un valor centinela que ningún presupuesto real puede cumplir.
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
        // Se reporta el número leído del propio archivo, no el solicitado por nombre: en este
        // punto ya coinciden (la compuerta 4 rechazó toda discrepancia), pero la fuente de verdad
        // que se propaga hacia afuera debe seguir siendo siempre la intrínseca, nunca la del nombre.
        numero_de_epoca: numero_confirmado,
        ruta_del_archivo: ruta_destino,
        epoca_superseida,
        duracion_de_conmutacion_ms: duracion_ms,
    })
}
