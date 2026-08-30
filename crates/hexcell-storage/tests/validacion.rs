//! Pruebas de integración para la compuerta de integridad de conocimiento.
//!
//! Estos escenarios validan el comportamiento síncrono del componente de
//! validación a través de la simulación de anomalías estructurales
//! (vectores huérfanos, discrepancia dimensional, huecos en ordinales)
//! y evaluaciones semánticas.

mod comun;

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::validacion::{
    MotivoDeRechazo, SondaResuelta, VeredictoDeIntegridad, validar_integridad_del_indice,
};
use rusqlite::Connection;
use std::fs;
use std::path::Path;

/// Crea una base de datos vacía en el directorio temporal, aplica migraciones
/// y comprueba de forma explícita que las claves foráneas estén activas.
fn crear_base_de_prueba(temp: &DirectorioTemporal) -> Connection {
    let ruta = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(ruta).expect("Debe abrir la base de datos");

    // Forzar activación de restricciones relacionales para reproducir producción.
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    let fk: i64 = conexion
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);

    aplicar_migraciones_de_conocimiento(&conexion).expect("Debe migrar el esquema");
    conexion
}

#[test]
fn verificar_ac1_vectores_huerfanos() {
    let temp = DirectorioTemporal::nuevo("ac1-huerfanos");
    let conexion = crear_base_de_prueba(&temp);

    // Declarar dimensión 4 en metadatos para esta prueba.
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    // Crear documento
    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'textotrozo', 1000)",
        [],
    ).unwrap();

    let vector_valido = vec![0.0f32, 0.0f32, 0.0f32, 0.0f32];
    let bytes_vector = vector_valido
        .iter()
        .flat_map(|val| val.to_le_bytes())
        .collect::<Vec<u8>>();

    // Fragmento 0 (con vector)
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    // Fragmento 1 (huérfano, sin vector asociado)
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (11, 1, 1, 'trozo')",
            [],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        let huerfano = motivos
            .iter()
            .any(|m| matches!(m, MotivoDeRechazo::VectoresHuerfanos { cantidad: 1 }));
        let ordinal = motivos
            .iter()
            .any(|m| matches!(m, MotivoDeRechazo::FaltaContiguidadOrdinal { .. }));
        let cobertura = motivos
            .iter()
            .any(|m| matches!(m, MotivoDeRechazo::DiferenciaDeFragmentos { .. }));

        assert!(huerfano, "Debe identificar el vector huerfano");
        assert!(
            !ordinal,
            "No debe diagnosticar fallas de orden inexistentes"
        );
        assert!(
            !cobertura,
            "No debe diagnosticar fallas de fragmentacion inexistentes"
        );
    } else {
        panic!("El veredicto debe ser de rechazo");
    }
}

#[test]
fn verificar_ac2_hueco_ordinal() {
    let temp = DirectorioTemporal::nuevo("ac2-ordinales");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    // Crear documento
    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'textotrozounodos', 1000)",
        [],
    ).unwrap();

    let vector_valido = vec![0.0f32, 0.0f32, 0.0f32, 0.0f32];
    let bytes_vector = vector_valido
        .iter()
        .flat_map(|val| val.to_le_bytes())
        .collect::<Vec<u8>>();

    // Insertamos ordinales 0, 1 y 3 (omitiendo el 2 para generar la falla)
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (11, 1, 1, 'trozo')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (11, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (13, 1, 3, 'unodos')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (13, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        let mut reportado = false;
        for m in motivos {
            if let MotivoDeRechazo::FaltaContiguidadOrdinal { faltantes } = m {
                assert_eq!(faltantes, vec![2]);
                reportado = true;
            }
        }
        assert!(reportado, "Debe enumerar la ausencia del ordinal 2");
    } else {
        panic!("El veredicto debe ser de rechazo");
    }
}

#[test]
fn verificar_ac3_mismatch_de_fragmentacion() {
    let temp = DirectorioTemporal::nuevo("ac3-fragmentacion");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    // Documento de 10 caracteres: 'textotrozo'
    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'textotrozo', 1000)",
        [],
    ).unwrap();

    let vector_valido = vec![0.0f32, 0.0f32, 0.0f32, 0.0f32];
    let bytes_vector = vector_valido
        .iter()
        .flat_map(|val| val.to_le_bytes())
        .collect::<Vec<u8>>();

    // Escribimos 1 fragmento únicamente (deberían ser 2 para la longitud de 10)
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        let mut reportado = false;
        for m in motivos {
            if let MotivoDeRechazo::DiferenciaDeFragmentos { esperado, recibido } = m {
                assert_eq!(esperado, 2);
                assert_eq!(recibido, 1);
                reportado = true;
            }
        }
        assert!(reportado, "Debe identificar la discrepancia cuantitativa");
    } else {
        panic!("El veredicto debe ser de rechazo");
    }
}

#[test]
fn verificar_ac4_dimension_incorrecta() {
    let temp = DirectorioTemporal::nuevo("ac4-dimension");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'texto', 1000)",
        [],
    ).unwrap();

    // Escribimos un vector de dimensión 8 (32 bytes) -> pasa el CHECK SQL (múltiplo de 4) pero incumple la época.
    let vector_incorrecto = vec![0.0f32; 8];
    let bytes_incorrectos = vector_incorrecto
        .iter()
        .flat_map(|val| val.to_le_bytes())
        .collect::<Vec<u8>>();

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_incorrectos],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        let mut reportado = false;
        for m in motivos {
            if let MotivoDeRechazo::DimensionDeVectorNoUniforme {
                cantidad_incorrectos,
                dimension_esperada,
            } = m
            {
                assert_eq!(cantidad_incorrectos, 1);
                assert_eq!(dimension_esperada, 4);
                reportado = true;
            }
        }
        assert!(
            reportado,
            "Debe capturar la desviacion dimensional del vector"
        );
    } else {
        panic!("El veredicto debe ser de rechazo");
    }
}

#[test]
fn verificar_ac5_metadatos_ausentes() {
    let temp = DirectorioTemporal::nuevo("ac5-metadata");
    let conexion = crear_base_de_prueba(&temp);

    // Provocar ausencia de metadatos eliminando la fila semilla.
    conexion
        .execute("DELETE FROM metadatos_de_epoca WHERE id = 1", [])
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        assert!(motivos.contains(&MotivoDeRechazo::MetadatosDeEpocaAusentes));
        assert!(motivos.contains(&MotivoDeRechazo::CalculoDeCoberturaOmitidoPorMetadatosAusentes));
        assert!(motivos.contains(&MotivoDeRechazo::CalculoDeDimensionOmitidoPorMetadatosAusentes));
        assert!(motivos.contains(&MotivoDeRechazo::SondaSemanticaOmitidaPorMetadatosAusentes));
    } else {
        panic!("El veredicto debe ser de rechazo");
    }
}

#[test]
fn verificar_ac6_verificacion_semantica() {
    let temp = DirectorioTemporal::nuevo("ac6-semantica");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'texto', 1000)",
        [],
    ).unwrap();

    let vector = vec![1.0f32, 0.0f32, 0.0f32, 0.0f32];
    let bytes_vector = vector
        .iter()
        .flat_map(|val| val.to_le_bytes())
        .collect::<Vec<u8>>();
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };

    // Escenario A: Similitud aprobada (coseno = 1.0, umbral = 0.8)
    let sonda_aprobada = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.8,
    };
    let veredicto_aprueba = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda_aprobada,
    )
    .unwrap();

    match veredicto_aprueba {
        VeredictoDeIntegridad::Aprobado {
            cantidad_de_fragmentos,
            dimension_de_embedding,
            similitud_observada,
            umbral_aplicado,
        } => {
            assert_eq!(cantidad_de_fragmentos, 1);
            assert_eq!(dimension_de_embedding, 4);
            assert!((similitud_observada - 1.0).abs() < 1e-5);
            assert_eq!(umbral_aplicado, 0.8);
        }
        _ => panic!("Debe aprobar la base de conocimiento"),
    }

    // Escenario B: Similitud reprobada (coseno = 0.0, umbral = 0.8)
    let sonda_reprobada = SondaResuelta {
        vector: vec![0.0, 1.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.8,
    };
    let veredicto_reprueba = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda_reprobada,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto_reprueba {
        let mut reportado = false;
        for m in motivos {
            if let MotivoDeRechazo::SimilitudInsuficiente {
                similitud_observada,
                umbral_requerido,
            } = m
            {
                assert!(similitud_observada.abs() < 1e-5);
                assert_eq!(umbral_requerido, 0.8);
                reportado = true;
            }
        }
        assert!(
            reportado,
            "Debe enumerar la deficiencia semantica del indice"
        );
    } else {
        panic!("Debe desaprobar la base de conocimiento");
    }
}

#[test]
fn verificar_ac7_ruta_de_archivo_personalizada() {
    let temp = DirectorioTemporal::nuevo("ac7-ruta");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();
    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'texto', 1000)",
        [],
    ).unwrap();

    let vector = vec![1.0f32, 0.0f32, 0.0f32, 0.0f32];
    let bytes_vector = vector
        .iter()
        .flat_map(|val| val.to_le_bytes())
        .collect::<Vec<u8>>();
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_vector],
        )
        .unwrap();

    // Cerrar explicitamente para evitar bloqueos durante la copia física de archivos.
    drop(conexion);

    let ruta_original = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let ruta_epoch = temp.ruta().join("knowledge_epoch_3.db");

    fs::copy(&ruta_original, &ruta_epoch).expect("Debe copiar el archivo");

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.8,
    };

    let veredicto = validar_integridad_del_indice(&ruta_epoch, &config, &sonda).unwrap();

    match veredicto {
        VeredictoDeIntegridad::Aprobado {
            cantidad_de_fragmentos,
            dimension_de_embedding,
            ..
        } => {
            assert_eq!(cantidad_de_fragmentos, 1);
            assert_eq!(dimension_de_embedding, 4);
        }
        _ => panic!("Debe aprobarse correctamente usando una ruta arbitraria de archivo"),
    }
}

#[test]
fn verificar_comportamiento_indice_vacio() {
    let temp = DirectorioTemporal::nuevo("indice-vacio");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        assert!(
            motivos.contains(&MotivoDeRechazo::IndiceVacio),
            "Debe marcar el indice vacio"
        );
    } else {
        panic!("Debe ser rechazado");
    }
}

#[test]
fn verificar_multiples_fallos_simultaneos() {
    let temp = DirectorioTemporal::nuevo("multiples-fallos");
    let conexion = crear_base_de_prueba(&temp);

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();

    // Documento de 10 caracteres: 'textotrozo'
    conexion.execute(
        "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'textotrozo', 1000)",
        [],
    ).unwrap();

    // Insertamos solo el ordinal 1 (falta ordinal 0 y falta un vector, ademas de mismatch cuantitativo)
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (10, 1, 1, 'texto')",
            [],
        )
        .unwrap();

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };

    let veredicto = validar_integridad_del_indice(
        &temp
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA),
        &config,
        &sonda,
    )
    .unwrap();

    if let VeredictoDeIntegridad::Rechazado { motivos } = veredicto {
        let huerfano = motivos
            .iter()
            .any(|m| matches!(m, MotivoDeRechazo::VectoresHuerfanos { .. }));
        let ordinal = motivos
            .iter()
            .any(|m| matches!(m, MotivoDeRechazo::FaltaContiguidadOrdinal { .. }));
        let cobertura = motivos
            .iter()
            .any(|m| matches!(m, MotivoDeRechazo::DiferenciaDeFragmentos { .. }));

        assert!(huerfano, "Debe reportar vectores huerfanos");
        assert!(ordinal, "Debe reportar falta de contiguidad ordinal");
        assert!(
            cobertura,
            "Debe reportar diferencia en la cuenta de fragmentos"
        );
    } else {
        panic!("Debe acumular los fallos simultaneos");
    }
}

#[test]
fn verificar_configuracion_de_fragmentacion_invalida_se_rechaza_en_la_entrada() {
    // La comprobacion 0 corre antes de abrir el archivo: una ruta inexistente basta.
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 5,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };
    let ruta = Path::new("/ruta/inexistente/no-se-abre.db");

    assert_eq!(
        validar_integridad_del_indice(ruta, &config, &sonda).unwrap(),
        VeredictoDeIntegridad::Rechazado {
            motivos: vec![MotivoDeRechazo::ConfiguracionDeFragmentacionInvalida {
                tamano_de_fragmento: 5,
                solapamiento: 5,
            }],
        },
        "debe reportarse sola, sin abrir el archivo ni nombrar un documento"
    );
}

fn bytes_le(valores: &[f32]) -> Vec<u8> {
    valores.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn preparar_base_con_fragmentos_sin_vector(conexion: &Connection, cantidad: i64) {
    conexion
        .execute_batch(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1;
             INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms)
                 VALUES (1, 'ref1', 'doc1', 'texto', 1000);",
        )
        .unwrap();
    for i in 0..cantidad {
        conexion
            .execute(
                "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (?1, 1, ?1, 'texto')",
                rusqlite::params![10 + i],
            )
            .unwrap();
    }
}

fn afirmar_rechazo_por_incomparables(temp: &DirectorioTemporal, cantidad: i64, motivo: &str) {
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };
    let sonda = SondaResuelta {
        vector: vec![1.0, 0.0, 0.0, 0.0],
        umbral_de_aceptacion: 0.5,
    };
    let ruta = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    match validar_integridad_del_indice(&ruta, &config, &sonda).unwrap() {
        VeredictoDeIntegridad::Rechazado { motivos } => assert!(
            motivos.iter().any(
                |m| matches!(m, MotivoDeRechazo::VectoresIncomparables { cantidad: c } if *c == cantidad)
            ),
            "{motivo}"
        ),
        VeredictoDeIntegridad::Aprobado { .. } => panic!("{motivo}"),
    }
}

#[test]
fn verificar_vector_degradado_a_nan_se_rechaza_en_vez_de_aprobarse() {
    let temp = DirectorioTemporal::nuevo("vector-nan-degradado");
    let conexion = crear_base_de_prueba(&temp);
    preparar_base_con_fragmentos_sin_vector(&conexion, 1);

    // BLOB de tamano correcto pero cada componente es NaN: sin esta correccion,
    // el maximo en curso queda envenenado para siempre y se aprueba un indice corrupto.
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_le(&[f32::NAN; 4])],
        )
        .unwrap();

    afirmar_rechazo_por_incomparables(&temp, 1, "un vector NaN nunca debe aprobarse");
}

#[test]
fn verificar_todos_los_vectores_de_dimension_distinta_se_rechazan_como_incomparables() {
    let temp = DirectorioTemporal::nuevo("vectores-indecodificables");
    let conexion = crear_base_de_prueba(&temp);
    preparar_base_con_fragmentos_sin_vector(&conexion, 2);

    // Dos BLOBs de dimension distinta a la sonda: se decodifican, pero
    // `similitud_coseno` no compara longitudes distintas y da None para ambos.
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (10, ?1)",
            rusqlite::params![bytes_le(&[1.0, 2.0])],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (11, ?1)",
            rusqlite::params![bytes_le(&[1.0, 2.0, 3.0])],
        )
        .unwrap();

    afirmar_rechazo_por_incomparables(&temp, 2, "sin vectores decodificables debe rechazarse");
}
