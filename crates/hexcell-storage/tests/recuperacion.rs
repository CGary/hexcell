//! Pruebas de integración para el motor de recuperación de contexto RAG (`recuperar_contexto`).
//!
//! Valida el comportamiento síncrono del motor de recuperación sobre la época viva:
//! resolución dinámica del pool a través de ArcSwap, escaneo atómico con un único con_lectura,
//! aborto ante vectores incomparables o dimensiones discrepantes, ordenación determinista por
//! total_cmp y desempate por id_fragmento, y retorno de contexto tipado sin ensamblado de prompt.

mod comun;

use std::sync::Arc;
use std::time::Duration;

use comun::DirectorioTemporal;
use hexcell_core::recuperacion::ConfiguracionDeRecuperacion;
use hexcell_storage::drenaje::{DesenlaceDeDrenaje, drenar_epoca_superseida};
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{GestorDePools, PoolDeConocimiento};
use hexcell_storage::recuperacion::recuperar_contexto;
use rusqlite::Connection;

/// Auxiliar privado para construir una base de conocimientos con los datos indicados.
fn crear_base_de_conocimiento(
    ruta_base: &std::path::Path,
    dimension: usize,
    fragmentos_datos: &[(i64, &str, Vec<f32>)],
) {
    let conexion = Connection::open(ruta_base).expect("abrir base de conocimiento de prueba");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    conexion
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))
        .unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).expect("aplicar migraciones de conocimiento");

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = ?1 WHERE id = 1",
            rusqlite::params![dimension as i64],
        )
        .unwrap();

    if !fragmentos_datos.is_empty() {
        conexion
            .execute(
                "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_test', 'Doc Test', 'Contenido completo de prueba', 1000)",
                [],
            )
            .unwrap();

        for &(id_frag, texto, ref vec_vals) in fragmentos_datos {
            conexion
                .execute(
                    "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (?1, 1, ?1, ?2)",
                    rusqlite::params![id_frag, texto],
                )
                .unwrap();

            let bytes_vector: Vec<u8> = vec_vals.iter().flat_map(|v| v.to_le_bytes()).collect();
            conexion
                .execute(
                    "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (?1, ?2)",
                    rusqlite::params![id_frag, bytes_vector],
                )
                .unwrap();
        }
    }
}

/// Auxiliar para instanciar un PoolDeConocimiento sobre un archivo de prueba.
fn crear_pool_de_prueba(
    dir_temp: &DirectorioTemporal,
    nombre_archivo: &str,
    dimension: usize,
    fragmentos: &[(i64, &str, Vec<f32>)],
) -> Arc<PoolDeConocimiento> {
    let ruta_db = dir_temp.ruta().join(nombre_archivo);
    crear_base_de_conocimiento(&ruta_db, dimension, fragmentos);
    Arc::new(PoolDeConocimiento::abrir_sobre(&ruta_db).unwrap())
}

#[test]
fn verificar_ac1_intercambio_de_epoca_entre_llamadas() {
    let temp = DirectorioTemporal::nuevo("ac1-intercambio-epocas");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor de pools");

    let pool_epoca1 = crear_pool_de_prueba(
        &temp,
        "knowledge_epoch_1.db",
        2,
        &[(10, "Texto Época 1", vec![1.0, 0.0])],
    );
    gestor.intercambiar_pool_de_conocimiento(pool_epoca1);

    let config = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 5,
        umbral_de_similitud: 0.1,
    };

    let query = vec![1.0, 0.0];
    let res1 = recuperar_contexto(&gestor, &query, &config).expect("recuperación 1 exitosa");
    assert_eq!(res1.fragmentos().len(), 1);
    assert_eq!(res1.fragmentos()[0].texto, "Texto Época 1");

    // Intercambiar dinámicamente la época viva a un nuevo pool con distinto contenido.
    let pool_epoca2 = crear_pool_de_prueba(
        &temp,
        "knowledge_epoch_2.db",
        2,
        &[(20, "Texto Época 2", vec![1.0, 0.0])],
    );
    gestor.intercambiar_pool_de_conocimiento(pool_epoca2);

    // La segunda llamada debe resolver la nueva época a través del ArcSwap sin reinicio.
    let res2 = recuperar_contexto(&gestor, &query, &config).expect("recuperación 2 exitosa");
    assert_eq!(res2.fragmentos().len(), 1);
    assert_eq!(res2.fragmentos()[0].texto, "Texto Época 2");
}

#[test]
fn verificar_ac1_arc_sostenido_y_lecturas_en_reposo() {
    let temp = DirectorioTemporal::nuevo("ac1-arc-sostenido");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor de pools");

    // Obtener la referencia al pool tal como hace el motor de recuperación.
    let pool = gestor.conocimiento();

    // Comprobar que el strong count del Arc es 2 (1 mantenido por el gestor, 1 por la variable local).
    assert_eq!(Arc::strong_count(&pool), 2);

    // Dentro de una invocación a con_lectura, lecturas_en_reposo() debe reportar false.
    let reposo_durante_escaneo = pool
        .con_lectura(|_| Ok(pool.lecturas_en_reposo()))
        .expect("ejecución de lectura exitosa");

    assert!(
        !reposo_durante_escaneo,
        "lecturas_en_reposo debe ser false mientras una lectura sostiene la conexión"
    );
}

#[test]
fn verificar_ac1_drenaje_espera_por_lector() {
    use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
    use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
    use hexcell_storage::promocion::{DesenlaceDePromocion, promover_epoca};

    let temp = DirectorioTemporal::nuevo("ac1-drenaje-espera");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor de pools");

    // Crear base de staging válida para ejecutar la promoción
    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).unwrap();
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 2 WHERE id = 1",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO documentos VALUES (1, 'ref', 't', 'c', 1000)",
            [],
        )
        .unwrap();
    let bytes = vec![1.0f32; 2]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    conexion
        .execute("INSERT INTO fragmentos VALUES (1, 1, 0, 't')", [])
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento VALUES (1, ?1)",
            rusqlite::params![bytes],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO sonda_semantica VALUES (1, 'c', ?1, 0.5, 1000)",
            rusqlite::params![bytes],
        )
        .unwrap();
    drop(conexion);

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 1,
        solapamiento: 0,
    };
    let desenlace_prom =
        promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promoción exitosa");
    let epoca_superseida = match desenlace_prom {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        _ => panic!("se esperaba época promovida"),
    };

    // Clonar un Arc del pool emulando una lectura en vuelo como la que retiene recuperar_contexto.
    let lector_en_vuelo = Arc::clone(epoca_superseida.pool());

    // Intentar drenar debe expirarse mientras el lector retenga la referencia Arc.
    let desenlace_drenaje = drenar_epoca_superseida(epoca_superseida, Duration::from_millis(50))
        .expect("drenaje no debe fallar con error");

    let epoca_superseida_expirada = match desenlace_drenaje {
        DesenlaceDeDrenaje::Expirada {
            epoca_superseida, ..
        } => epoca_superseida,
        _ => panic!("Se esperaba DesenlaceDeDrenaje::Expirada"),
    };

    // Liberar la referencia del lector.
    drop(lector_en_vuelo);

    // Tras soltar la referencia, el drenaje debe completarse con éxito.
    let desenlace_exitoso =
        drenar_epoca_superseida(epoca_superseida_expirada, Duration::from_secs(1))
            .expect("drenaje exitoso");
    assert!(matches!(
        desenlace_exitoso,
        DesenlaceDeDrenaje::Drenada { .. }
    ));
}

#[test]
fn verificar_ac2_limite_y_orden_sobre_epoca_real() {
    let temp = DirectorioTemporal::nuevo("ac2-limite-y-orden");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // 5 fragmentos con similitudes claramente distintas con respecto al vector de consulta [1.0, 0.0]
    let fragmentos = vec![
        (1, "Frag 1 (sim 0.2)", vec![0.2, 0.9797959]),
        (2, "Frag 2 (sim 0.9)", vec![0.9, 0.4358899]),
        (3, "Frag 3 (sim 0.5)", vec![0.5, 0.8660254]),
        (4, "Frag 4 (sim 0.98)", vec![0.98, 0.1989975]),
        (5, "Frag 5 (sim 0.4)", vec![0.4, 0.9165151]),
    ];

    let pool = crear_pool_de_prueba(&temp, "knowledge_epoch_orden.db", 2, &fragmentos);
    gestor.intercambiar_pool_de_conocimiento(pool);

    let config = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 3,
        umbral_de_similitud: 0.1,
    };

    let query = vec![1.0, 0.0];
    let res1 = recuperar_contexto(&gestor, &query, &config).expect("recuperación 1");
    assert_eq!(res1.fragmentos().len(), 3);

    // Deben estar ordenados estrictamente por similitud descendente: Frag 4 (0.98), Frag 2 (0.9), Frag 3 (0.5)
    assert_eq!(res1.fragmentos()[0].id_fragmento, 4);
    assert_eq!(res1.fragmentos()[1].id_fragmento, 2);
    assert_eq!(res1.fragmentos()[2].id_fragmento, 3);

    // Segunda llamada produce secuencia idéntica.
    let res2 = recuperar_contexto(&gestor, &query, &config).expect("recuperación 2");
    assert_eq!(res1, res2);
}

#[test]
fn verificar_ac3_resultado_vacio_es_contexto_no_error() {
    let temp = DirectorioTemporal::nuevo("ac3-contexto-vacio");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Caso (a): Época con fragmentos cuya similitud queda por debajo del umbral.
    let fragmentos = vec![(1, "Texto irrelevante", vec![0.0, 1.0])];
    let pool_a = crear_pool_de_prueba(&temp, "knowledge_epoch_baja_sim.db", 2, &fragmentos);
    gestor.intercambiar_pool_de_conocimiento(pool_a);

    let config_alta = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 5,
        umbral_de_similitud: 0.8,
    };
    let query = vec![1.0, 0.0];
    let res_a = recuperar_contexto(&gestor, &query, &config_alta).expect("debe ser Ok");
    assert!(res_a.esta_vacio());

    // Caso (b): Época con metadatos de época presentes pero cero filas en la tabla fragmentos.
    let pool_b = crear_pool_de_prueba(&temp, "knowledge_epoch_sin_filas.db", 2, &[]);
    gestor.intercambiar_pool_de_conocimiento(pool_b);

    let res_b = recuperar_contexto(&gestor, &query, &config_alta).expect("debe ser Ok");
    assert!(res_b.esta_vacio());
}

#[test]
fn verificar_ac4_vector_incomparable_aborta_y_nombra_fragmento() {
    let temp = DirectorioTemporal::nuevo("ac4-vector-incomparable");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let ruta_db = temp.ruta().join("knowledge_epoch_incomparable.db");

    // Crear base con metadatos de dimensión 2
    let conexion = Connection::open(&ruta_db).expect("abrir base de conocimientos");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 2 WHERE id = 1",
            [],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_inc', 'Doc Inc', 'Contenido', 1000)",
            [],
        )
        .unwrap();

    // Subcaso (a): Vector cuya longitud en bytes es múltiplo de 4 pero discrepa de 4 * dimension_de_embedding (p.ej. 16 bytes = 4 floats en vez de 2)
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (42, 1, 0, 'Incomparable por longitud')",
            [],
        )
        .unwrap();

    let bytes_largos: Vec<u8> = vec![1.0f32, 2.0, 3.0, 4.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (42, ?1)",
            rusqlite::params![bytes_largos],
        )
        .unwrap();

    drop(conexion);

    let pool_a = Arc::new(PoolDeConocimiento::abrir_sobre(&ruta_db).unwrap());
    gestor.intercambiar_pool_de_conocimiento(pool_a);

    let config = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 5,
        umbral_de_similitud: 0.1,
    };
    let query_valida = vec![1.0, 0.0];

    let res_a = recuperar_contexto(&gestor, &query_valida, &config);
    match res_a {
        Err(ErrorDeAlmacen::VectorDeFragmentoIncomparable { id_fragmento }) => {
            assert_eq!(id_fragmento, 42);
        }
        otro => panic!("Se esperaba VectorDeFragmentoIncomparable(42), se obtuvo: {otro:?}"),
    }

    // Subcaso (b): Vector de la longitud correcta pero cuya norma es cero (componentes cero)
    let pool_b = crear_pool_de_prueba(
        &temp,
        "knowledge_epoch_norma_cero.db",
        2,
        &[(43, "Norma Cero", vec![0.0, 0.0])],
    );
    gestor.intercambiar_pool_de_conocimiento(pool_b);

    let res_b = recuperar_contexto(&gestor, &query_valida, &config);
    match res_b {
        Err(ErrorDeAlmacen::VectorDeFragmentoIncomparable { id_fragmento }) => {
            assert_eq!(id_fragmento, 43);
        }
        otro => panic!("Se esperaba VectorDeFragmentoIncomparable(43), se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac5_dimension_de_consulta_discrepante_antes_de_escaneo() {
    let temp = DirectorioTemporal::nuevo("ac5-dimension-discrepante");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Época declarando dimensión 4 en metadatos
    let pool = crear_pool_de_prueba(
        &temp,
        "knowledge_epoch_dim4.db",
        4,
        &[(1, "Fragmento Valido", vec![1.0, 0.0, 0.0, 0.0])],
    );
    gestor.intercambiar_pool_de_conocimiento(pool);

    let config = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 5,
        umbral_de_similitud: 0.1,
    };

    // Vector de consulta con dimensión 3 (mismatch contra dimensión 4 de la época)
    let query_incorrecta = vec![1.0, 0.0, 0.0];

    let res = recuperar_contexto(&gestor, &query_incorrecta, &config);
    match res {
        Err(ErrorDeAlmacen::DimensionDeConsultaDiscrepante {
            dimension_de_consulta,
            dimension_de_epoca,
        }) => {
            assert_eq!(dimension_de_consulta, 3);
            assert_eq!(dimension_de_epoca, 4);
        }
        otro => panic!("Se esperaba DimensionDeConsultaDiscrepante, se obtuvo: {otro:?}"),
    }

    // Aserción de orden de verificación pre-escaneo:
    // Construir una época que contenga UN fragmento incomparable (subcaso AC-4) Y lanzar una consulta con dimensión errónea.
    let ruta_mixta = temp.ruta().join("knowledge_epoch_mixta.db");
    let conexion = Connection::open(&ruta_mixta).unwrap();
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_m', 'Doc', 'Cont', 1000)",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (99, 1, 0, 'Corrupto')",
            [],
        )
        .unwrap();
    // 8 bytes (2 floats) en lugar de 16 bytes (4 floats)
    let bytes_corruptos: Vec<u8> = vec![1.0f32, 2.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (99, ?1)",
            rusqlite::params![bytes_corruptos],
        )
        .unwrap();
    drop(conexion);

    let pool_mixto = Arc::new(PoolDeConocimiento::abrir_sobre(&ruta_mixta).unwrap());
    gestor.intercambiar_pool_de_conocimiento(pool_mixto);

    // Consulta con dimensión 3 debe fallar por DimensionDeConsultaDiscrepante, NUNCA por VectorDeFragmentoIncomparable
    let res_mixto = recuperar_contexto(&gestor, &query_incorrecta, &config);
    assert!(
        matches!(
            res_mixto,
            Err(ErrorDeAlmacen::DimensionDeConsultaDiscrepante { .. })
        ),
        "La validación de dimensión debe ser previa al escaneo de filas"
    );
}

#[test]
fn verificar_ac6_resultado_tipado_sin_ensamblado_de_prompt() {
    let temp = DirectorioTemporal::nuevo("ac6-resultado-tipado");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    let pool = crear_pool_de_prueba(
        &temp,
        "knowledge_epoch_tipado.db",
        2,
        &[(7, "Texto Fragmento Tipado", vec![1.0, 0.0])],
    );
    gestor.intercambiar_pool_de_conocimiento(pool);

    let config = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 5,
        umbral_de_similitud: 0.1,
    };
    let query = vec![1.0, 0.0];

    let contexto = recuperar_contexto(&gestor, &query, &config).expect("recuperación exitosa");
    assert_eq!(contexto.fragmentos().len(), 1);

    let frag = &contexto.fragmentos()[0];
    assert_eq!(frag.id_fragmento, 7);
    assert_eq!(frag.texto, "Texto Fragmento Tipado");
    assert!((frag.similitud - 1.0).abs() < f32::EPSILON);
}
