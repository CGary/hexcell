//! Módulo de validación de integridad para el índice de conocimiento.
//!
//! Este módulo contiene la lógica para verificar la estructura física y la calidad
//! semántica de una base de datos de época antes de permitir su promoción a producción.
//! Funciona de manera totalmente síncrona y fuera de línea, sin dependencias de red,
//! cumpliendo con el presupuesto de memoria de la célula (NFR-01) al transmitir datos
//! fila por fila.

use crate::conocimiento::inspeccionar_base_en_sombra;
use crate::error::ErrorDeAlmacen;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use std::path::Path;

/// Sonda semántica resuelta previamente durante el proceso de ingesta.
///
/// Contiene el vector ya generado y el límite inferior aceptable para la similitud.
/// Se pasa ya resuelta para evitar que el validador tenga que conectarse a servicios
/// externos de generación de vectores, garantizando la predictibilidad y velocidad de la compuerta.
#[derive(Clone, Debug, PartialEq)]
pub struct SondaResuelta {
    /// Vector de características pre-calculado para la consulta de prueba.
    pub vector: Vec<f32>,
    /// Valor mínimo que debe alcanzar el coseno de la similitud para aprobar el índice.
    pub umbral_de_aceptacion: f32,
}

/// Enumerado que lista los diferentes fallos estructurales o de cobertura detectados.
///
/// # Razón de diseño
/// Este tipo no implementa la característica `Eq` porque contiene campos de punto flotante
/// representados por `f32` (como la similitud y el límite), cuya comparación exacta no
/// está definida matemáticamente.
#[derive(Clone, Debug, PartialEq)]
pub enum MotivoDeRechazo {
    /// La base de datos carece de la fila única que detalla los metadatos de la época.
    MetadatosDeEpocaAusentes,
    /// Existen registros de fragmentos que no poseen su correspondiente vector.
    VectoresHuerfanos {
        /// Cantidad de fragmentos detectados en estado huérfano.
        cantidad: i64,
    },
    /// La secuencia de ordinales presenta discontinuidades o huecos.
    FaltaContiguidadOrdinal {
        /// Lista de índices ordinales que faltan en la secuencia.
        faltantes: Vec<i64>,
    },
    /// El índice auditado no contiene ningún fragmento de texto.
    IndiceVacio,
    /// La cantidad total de fragmentos difiere de la esperada al re-trocear el texto origen.
    DiferenciaDeFragmentos {
        /// Cantidad de fragmentos esperada tras volver a ejecutar la fragmentación.
        esperado: i64,
        /// Cantidad real registrada en la tabla de fragmentos.
        recibido: i64,
    },
    /// La configuración de fragmentación suministrada por el llamador es inválida en sí
    /// misma: el solapamiento no es estrictamente menor que el tamaño de fragmento.
    ///
    /// Se detecta antes de abrir el archivo, sin leer ningún documento, porque este es el
    /// único caso en que `fragmentar` falla y depende únicamente de estos dos números, no
    /// de una fila en particular. Nombrar un documento aquí culparía a un dato inocente por
    /// un defecto que pertenece exclusivamente al argumento del llamador.
    ConfiguracionDeFragmentacionInvalida {
        /// Tamaño de fragmento suministrado por el llamador.
        tamano_de_fragmento: usize,
        /// Solapamiento suministrado por el llamador.
        solapamiento: usize,
    },
    /// Algún vector almacenado posee un tamaño en bytes que no se corresponde con la dimensión declarada.
    DimensionDeVectorNoUniforme {
        /// Cantidad de vectores que presentan una dimensión incorrecta.
        cantidad_incorrectos: i64,
        /// Dimensión esperada según el registro de metadatos de la época.
        dimension_esperada: i64,
    },
    /// La dimensión del vector de prueba suministrado no coincide con la dimensión de la época.
    DimensionDeLaSondaDiscrepante {
        /// Dimensión del vector suministrado en la sonda de prueba.
        dimension_sonda: i64,
        /// Dimensión registrada en los metadatos del índice.
        dimension_epoca: i64,
    },
    /// La similitud coseno del fragmento más afín queda por debajo del límite mínimo.
    SimilitudInsuficiente {
        /// Mayor valor de similitud coseno observado contra los fragmentos del índice.
        similitud_observada: f32,
        /// Límite mínimo requerido para la aprobación.
        umbral_requerido: f32,
    },
    /// Ningún vector del índice pudo decodificarse o compararse contra la sonda semántica.
    ///
    /// Un `BLOB` corrupto, o un vector con componentes no finitos, hace que
    /// `similitud_coseno` devuelva `None` para esa fila: la similitud queda indefinida,
    /// no baja. Reportar aquí un número de similitud inventado (como -1.0) sería mentir
    /// sobre una observación que jamás ocurrió, así que esta compuerta rechaza en su lugar
    /// con la cuenta exacta de filas que no pudieron compararse.
    VectoresIncomparables {
        /// Cantidad de vectores de fragmento que no pudieron decodificarse o compararse.
        cantidad: i64,
    },
    /// No se pudo validar la cobertura del troceado porque no se dispone de metadatos de época.
    CalculoDeCoberturaOmitidoPorMetadatosAusentes,
    /// No se pudo comprobar la dimensión de los vectores porque no se dispone de metadatos de época.
    CalculoDeDimensionOmitidoPorMetadatosAusentes,
    /// Se omitió la comprobación semántica porque los metadatos de época no están disponibles.
    SondaSemanticaOmitidaPorMetadatosAusentes,
}

/// Representa el resultado definitivo de la auditoría de integridad.
///
/// # Razón de diseño
/// Este tipo no implementa la característica `Eq` porque contiene campos de punto flotante
/// en su variante de aprobación, por los mismos motivos numéricos expuestos en el motivo de rechazo.
#[derive(Clone, Debug, PartialEq)]
pub enum VeredictoDeIntegridad {
    /// El índice cumple con todos los requisitos estructurales y semánticos.
    Aprobado {
        /// Cantidad total de fragmentos validados de forma contigua.
        cantidad_de_fragmentos: i64,
        /// Dimensión de los vectores de características confirmada de forma uniforme.
        dimension_de_embedding: i64,
        /// Puntuación del coseno más alta alcanzada por la sonda semántica.
        similitud_observada: f32,
        /// Límite inferior de aceptación aplicado.
        umbral_aplicado: f32,
    },
    /// El índice presenta anomalías estructurales o una afinidad semántica insuficiente.
    Rechazado {
        /// Colección exhaustiva de todos los fallos identificados durante la ejecución.
        motivos: Vec<MotivoDeRechazo>,
    },
}

/// Ejecuta una serie de validaciones estructurales y una verificación semántica en el índice.
///
/// Reúne todos los errores detectados en un veredicto estructurado para permitir un diagnóstico
/// completo al operador en un único paso, evitando ciclos repetitivos de corrección de errores.
pub fn validar_integridad_del_indice(
    ruta_archivo: &Path,
    configuracion_de_fragmentacion: &ConfiguracionDeFragmentacion,
    sonda: &SondaResuelta,
) -> Result<VeredictoDeIntegridad, ErrorDeAlmacen> {
    // 0. Validar la configuración de fragmentación antes de abrir el archivo o leer una
    // sola fila: es el único argumento del que depende un posible fallo de `fragmentar`,
    // así que el defecto (si existe) ya se conoce sin haber tocado la base de datos.
    if configuracion_de_fragmentacion.solapamiento
        >= configuracion_de_fragmentacion.tamano_de_fragmento
    {
        return Ok(VeredictoDeIntegridad::Rechazado {
            motivos: vec![MotivoDeRechazo::ConfiguracionDeFragmentacionInvalida {
                tamano_de_fragmento: configuracion_de_fragmentacion.tamano_de_fragmento,
                solapamiento: configuracion_de_fragmentacion.solapamiento,
            }],
        });
    }

    let mut motivos = Vec::new();

    // 1. Obtener la inspección factual básica del archivo utilizando la función existente.
    let resumen = inspeccionar_base_en_sombra(ruta_archivo)?;

    // Abrir una conexión de lectura para realizar las comprobaciones que requieren flujo de filas.
    let conexion = crate::pools::abrir_solo_lectura(ruta_archivo)?;

    // 2. Comprobar la existencia de los metadatos indispensables de la época.
    let metadatos = &resumen.metadatos_de_epoca;
    if metadatos.is_none() {
        motivos.push(MotivoDeRechazo::MetadatosDeEpocaAusentes);
    }

    // 3. Evaluar la existencia de vectores huérfanos.
    if resumen.fragmentos_sin_vector > 0 {
        motivos.push(MotivoDeRechazo::VectoresHuerfanos {
            cantidad: resumen.fragmentos_sin_vector,
        });
    }

    // 4. Comprobar la secuencia continua de ordinales y el caso especial de índice vacío.
    if resumen.cantidad_de_fragmentos == 0 {
        motivos.push(MotivoDeRechazo::IndiceVacio);
    } else {
        let mut faltantes = Vec::new();
        for i in 0..resumen.cantidad_de_fragmentos {
            if !resumen.ordinales.contains(&i) {
                faltantes.push(i);
            }
        }
        if !faltantes.is_empty() {
            motivos.push(MotivoDeRechazo::FaltaContiguidadOrdinal { faltantes });
        }
    }

    // 5. Validar la cobertura de troceado y la uniformidad dimensional si los metadatos están presentes.
    if let Some(meta) = metadatos {
        // Comprobación de cobertura: re-fragmentar el contenido de los documentos almacenados.
        // Se realiza transmitiendo filas secuencialmente para respetar los límites de memoria.
        let mut stmt_docs = conexion
            .prepare("SELECT id, contenido FROM documentos")
            .map_err(ErrorDeAlmacen::en(
                "preparar consulta de contenidos de documentos",
            ))?;
        let mut filas_docs = stmt_docs.query([]).map_err(ErrorDeAlmacen::en(
            "ejecutar consulta de contenidos de documentos",
        ))?;

        let mut total_esperado = 0i64;

        while let Some(fila) = filas_docs
            .next()
            .map_err(ErrorDeAlmacen::en("leer fila de documento"))?
        {
            let contenido: String = fila
                .get(1)
                .map_err(ErrorDeAlmacen::en("obtener contenido de documento"))?;
            // La comprobación 0 ya garantizó que esta configuración es válida para
            // `fragmentar`, así que el brazo `Err` es inalcanzable en este punto: la única
            // causa de ese error es la propia configuración, no el contenido de una fila.
            if let Ok(fragmentos) =
                hexcell_core::fragmentacion::fragmentar(&contenido, configuracion_de_fragmentacion)
            {
                total_esperado += fragmentos.len() as i64;
            }
        }

        if total_esperado != resumen.cantidad_de_fragmentos {
            motivos.push(MotivoDeRechazo::DiferenciaDeFragmentos {
                esperado: total_esperado,
                recibido: resumen.cantidad_de_fragmentos,
            });
        }

        // Comprobación de la dimensión uniforme de los vectores en bytes.
        let cantidad_incorrectos: i64 = conexion
            .query_row(
                "SELECT COUNT(*) FROM vectores_de_fragmento v JOIN metadatos_de_epoca m ON m.id = 1 WHERE length(v.vector) != 4 * m.dimension_de_embedding",
                [],
                |row| row.get(0),
            )
            .map_err(ErrorDeAlmacen::en("consultar uniformidad dimensional de vectores"))?;

        if cantidad_incorrectos > 0 {
            motivos.push(MotivoDeRechazo::DimensionDeVectorNoUniforme {
                cantidad_incorrectos,
                dimension_esperada: meta.dimension_de_embedding,
            });
        }
    } else {
        // Si no existen metadatos, estas comprobaciones estructurales avanzadas no son factibles.
        motivos.push(MotivoDeRechazo::CalculoDeCoberturaOmitidoPorMetadatosAusentes);
        motivos.push(MotivoDeRechazo::CalculoDeDimensionOmitidoPorMetadatosAusentes);
    }

    // 6. Realizar la prueba semántica local con los fragmentos cargados en flujo.
    let mut mejor_similitud: Option<f32> = None;

    if let Some(meta) = metadatos {
        let dim_sonda = sonda.vector.len() as i64;
        let dim_epoca = meta.dimension_de_embedding;

        if dim_sonda != dim_epoca {
            motivos.push(MotivoDeRechazo::DimensionDeLaSondaDiscrepante {
                dimension_sonda: dim_sonda,
                dimension_epoca: dim_epoca,
            });
        } else if resumen.cantidad_de_fragmentos > 0 {
            // Evaluamos la similitud únicamente si hay fragmentos cargados y las dimensiones coinciden.
            let mut stmt_vectores = conexion
                .prepare("SELECT vector FROM vectores_de_fragmento")
                .map_err(ErrorDeAlmacen::en(
                    "preparar consulta de vectores de fragmentos",
                ))?;
            let mut filas_vectores = stmt_vectores.query([]).map_err(ErrorDeAlmacen::en(
                "ejecutar consulta de vectores de fragmentos",
            ))?;

            // Cuenta las filas cuyo vector no pudo decodificarse o compararse: un BLOB
            // corrupto o un componente no finito nunca debe desaparecer en silencio, porque
            // esa fila es exactamente la que un índice degradado necesita esconder.
            let mut incomparables = 0i64;

            while let Some(fila) = filas_vectores
                .next()
                .map_err(ErrorDeAlmacen::en("leer fila de vector"))?
            {
                let bytes_vector: Vec<u8> = fila
                    .get(0)
                    .map_err(ErrorDeAlmacen::en("obtener bytes de vector"))?;
                let similitud_de_esta_fila =
                    hexcell_core::embeddings::VectorDeEmbedding::desde_bytes_le(&bytes_vector)
                        .and_then(|vector_emb| {
                            hexcell_core::similitud::similitud_coseno(
                                vector_emb.valores(),
                                &sonda.vector,
                            )
                        });

                match similitud_de_esta_fila {
                    Some(similitud) => match mejor_similitud {
                        None => mejor_similitud = Some(similitud),
                        Some(actual_mejor) => {
                            if similitud > actual_mejor {
                                mejor_similitud = Some(similitud);
                            }
                        }
                    },
                    None => incomparables += 1,
                }
            }

            // `similitud_coseno` ya descarta todo componente no finito (NaN o infinito), así
            // que un `Some` aquí es siempre un número comparable: la aprobación nunca puede
            // apoyarse en una similitud indefinida.
            match mejor_similitud {
                Some(sim) if sim < sonda.umbral_de_aceptacion => {
                    motivos.push(MotivoDeRechazo::SimilitudInsuficiente {
                        similitud_observada: sim,
                        umbral_requerido: sonda.umbral_de_aceptacion,
                    });
                }
                Some(_) => {}
                None => {
                    // Ninguna fila produjo una similitud válida: no hay un "mejor" candidato
                    // sobre el cual comparar el umbral, así que el rechazo nombra la causa
                    // real (filas incomparables) en vez de inventar una similitud de -1.0.
                    motivos.push(MotivoDeRechazo::VectoresIncomparables {
                        cantidad: incomparables,
                    });
                }
            }
        }
    } else {
        motivos.push(MotivoDeRechazo::SondaSemanticaOmitidaPorMetadatosAusentes);
    }

    // 7. Retornar el veredicto consolidado de la compuerta.
    if motivos.is_empty()
        && let Some(sim) = mejor_similitud
        && let Some(meta) = metadatos
    {
        return Ok(VeredictoDeIntegridad::Aprobado {
            cantidad_de_fragmentos: resumen.cantidad_de_fragmentos,
            dimension_de_embedding: meta.dimension_de_embedding,
            similitud_observada: sim,
            umbral_aplicado: sonda.umbral_de_aceptacion,
        });
    }
    // Inalcanzable en la práctica: si `motivos` quedó vacío, la comprobación 6 ya
    // garantizó metadatos y una similitud finita, así que la rama de aprobación de
    // arriba ya habría retornado. Este es el cierre honesto para el resto de los casos:
    // un rechazo con la colección completa de motivos acumulados.
    Ok(VeredictoDeIntegridad::Rechazado { motivos })
}
