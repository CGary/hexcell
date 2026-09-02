//! Servicio de aplicación para la recuperación de contexto RAG sobre la época viva.
//!
//! Este módulo implementa la función síncrona `recuperar_contexto`, encargada de escanear la base
//! de conocimiento activa por similitud coseno contra un vector de consulta proporcionado por el
//! solicitante. El crate es libre de ejecutores asíncronos por invariante de diseño (`adr-0003`),
//! dejando la planificación en hilos bloqueantes al consumidor que posea un runtime (`spawn_blocking`).
//!
//! # Secuencia ordenada de operaciones y disciplina de cerrojos (AC-1, AC-4, AC-5)
//!
//! 1. `let pool = gestor.conocimiento();` resuelve el puntero `ArcSwap` en esta llamada y retiene
//!    el `Arc` durante toda la ejecución del escaneo. No se almacena en caché un pool entre llamadas
//!    ni se resuelve ninguna época por número o por nombre de archivo (AC-1).
//! 2. Se invoca `pool.con_lectura(...)` **una única vez** para abarcar todo el barrido. De esta forma,
//!    el `Mutex` de lectura se mantiene adquirido durante el flujo streaming y `lecturas_en_reposo()`
//!    notifica `false` al drenaje de `HEX-056` (AC-1).
//! 3. **Verificación previa a la consulta**: Antes de preparar cualquier consulta sobre la tabla de
//!    fragmentos, se lee `dimension_de_embedding` de `metadatos_de_epoca`. Si la dimensión del vector
//!    de consulta discrepa, se retorna de inmediato `ErrorDeAlmacen::DimensionDeConsultaDiscrepante`
//!    sin escanear ningún fragmento (AC-5).
//! 4. Se transmiten (*stream*) las filas de `fragmentos` unidas con `vectores_de_fragmento` mediante
//!    un iterador de SQLite, manteniendo la memoria plana.
//! 5. Para cada fila, la conversión `VectorDeEmbedding::desde_bytes_le` seguida de `similitud_coseno`
//!    debe devolver `Some(f32)`. Si cualquiera de los dos pasos produce `None`, la operación aborta
//!    inmediatamente devueltos en `Err(ErrorDeAlmacen::VectorDeFragmentoIncomparable { id_fragmento })`.
//!    Un fragmento incoherente nunca se omite ni se evalúa como cero (AC-4).
//! 6. Se filtran únicamente aquellos resultados cuya similitud sea mayor o igual al `umbral_de_similitud` (AC-3).
//! 7. Se aplica `ordenar_por_relevancia` para garantizar un orden determinista con desempate por
//!    `id_fragmento` (AC-2) y se trunca la lista al `maximo_de_fragmentos` configurado.

use hexcell_core::recuperacion::{
    ConfiguracionDeRecuperacion, ContextoRecuperado, FragmentoRecuperado, ordenar_por_relevancia,
};

use crate::error::ErrorDeAlmacen;
use crate::pools::GestorDePools;

/// Recupera un contexto de fragmentos relevantes escaneando la época viva de conocimiento.
///
/// # Invariantes y garantías
/// - **Resuelve el pool dinámicamente**: `gestor.conocimiento()` lee el puntero en cada llamada.
/// - **Escaneo atómico en lectura**: Una única llamada a `pool.con_lectura` sostiene la conexión de lectura.
/// - **Rechazo previo por dimensión**: Mismatches en la dimensión del vector abortan antes de leer fragmentos.
/// - **Aborto estricto ante vectores incomparables**: Errores de decodificación o componentes NaN/norma cero abortan.
/// - **Resultados tipados**: Devuelve `ContextoRecuperado` (incluso si está vacío), nunca un error por cero coincidencias.
pub fn recuperar_contexto(
    gestor: &GestorDePools,
    vector_de_consulta: &[f32],
    configuracion: &ConfiguracionDeRecuperacion,
) -> Result<ContextoRecuperado, ErrorDeAlmacen> {
    // 1. Obtener la referencia Arc viva al pool de conocimiento actual a través del ArcSwap (AC-1).
    let pool = gestor.conocimiento();

    // 2. Realizar el escaneo completo bajo una única invocación a con_lectura (AC-1).
    pool.con_lectura(|conexion| {
        // 3. Inspeccionar la dimensión declarada por la época antes de consultar fragmentos (AC-5).
        let dimension_de_epoca: i64 = conexion
            .query_row(
                "SELECT dimension_de_embedding FROM metadatos_de_epoca WHERE id = 1",
                [],
                |fila| fila.get(0),
            )
            .map_err(ErrorDeAlmacen::en(
                "leer dimensión de embedding en metadatos_de_epoca",
            ))?;

        let dimension_de_consulta = vector_de_consulta.len() as i64;
        if dimension_de_consulta != dimension_de_epoca {
            return Err(ErrorDeAlmacen::DimensionDeConsultaDiscrepante {
                dimension_de_consulta,
                dimension_de_epoca,
            });
        }

        // 4. Preparar la lectura streaming de fragmentos y sus vectores asociados.
        let mut sentencia = conexion
            .prepare(
                "SELECT f.id, f.texto, v.vector \
                 FROM fragmentos f \
                 JOIN vectores_de_fragmento v ON v.id_fragmento = f.id",
            )
            .map_err(ErrorDeAlmacen::en(
                "preparar consulta de recuperación de fragmentos y vectores",
            ))?;

        let mut filas = sentencia.query([]).map_err(ErrorDeAlmacen::en(
            "ejecutar consulta de recuperación de fragmentos y vectores",
        ))?;

        let mut candidatos = Vec::new();

        // 5. Recorrer las filas streaming evaluando la similitud coseno.
        while let Some(fila) = filas
            .next()
            .map_err(ErrorDeAlmacen::en("leer fila de fragmento y vector"))?
        {
            let id_fragmento: i64 = fila
                .get(0)
                .map_err(ErrorDeAlmacen::en("obtener id_fragmento en recuperación"))?;
            let texto: String = fila
                .get(1)
                .map_err(ErrorDeAlmacen::en("obtener texto en recuperación"))?;
            let bytes_vector: Vec<u8> = fila.get(2).map_err(ErrorDeAlmacen::en(
                "obtener bytes de vector en recuperación",
            ))?;

            let vector_emb =
                hexcell_core::embeddings::VectorDeEmbedding::desde_bytes_le(&bytes_vector);
            let opt_similitud = vector_emb.and_then(|emb| {
                hexcell_core::similitud::similitud_coseno(emb.valores(), vector_de_consulta)
            });

            let similitud = match opt_similitud {
                Some(s) => s,
                None => {
                    // Un vector ilegible, con longitud inadecuada o con componentes que devuelven None
                    // en el coseno aborta la recuperación identificando la fila afectada (AC-4).
                    return Err(ErrorDeAlmacen::VectorDeFragmentoIncomparable { id_fragmento });
                }
            };

            // 6. Filtrar por el umbral de similitud configurado (AC-3).
            if similitud >= configuracion.umbral_de_similitud {
                candidatos.push(FragmentoRecuperado {
                    id_fragmento,
                    texto,
                    similitud,
                });
            }
        }

        // 7. Ordenar de forma determinista por similitud descendente e id ascendente (AC-2).
        ordenar_por_relevancia(&mut candidatos);

        // Truncar al máximo de fragmentos configurados.
        if candidatos.len() > configuracion.maximo_de_fragmentos {
            candidatos.truncate(configuracion.maximo_de_fragmentos);
        }

        Ok(ContextoRecuperado::nuevo(candidatos))
    })
}
