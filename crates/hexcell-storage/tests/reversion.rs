//! Pruebas de integración para la reversión de épocas de conocimiento.
//!
//! Valida la conmutación segura a épocas selladas previas, la compuerta de integridad estructural
//! (GUARD 1), la compuerta de sonda semántica (GUARD 2), la reutilización de identidad intrínseca
//! sin colisión (AC-3), y la inercia estricta ante rechazos.

mod comun;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO};
use hexcell_storage::promocion::{numero_de_epoca_siguiente, promover_epoca};
use hexcell_storage::reversion::{
    DesenlaceDeReversion, MotivoDeRechazoDeReversion, es_motivo_semantico, revertir_a_epoca,
};
use hexcell_storage::validacion::MotivoDeRechazo;
use rusqlite::Connection;

/// Fabrica un archivo de staging consistente con dimensiones fijadas para pruebas.
fn preparar_staging_valido(ruta_datos: &Path, dimension: usize) -> ConfiguracionDeFragmentacion {
    let ruta_staging = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir base de staging");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    conexion
        .query_row("PRAGMA journal_mode = WAL", [], |fila| {
            fila.get::<_, String>(0)
        })
        .unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging");

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = ?1 WHERE id = 1",
            rusqlite::params![dimension as i64],
        )
        .unwrap();

    let texto_contenido = "Contenido de catálogo para validación de reversión.";
    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_1', 'Título 1', ?1, 1000)",
            rusqlite::params![texto_contenido],
        )
        .unwrap();

    let vector = vec![1.0f32; dimension];
    let vector_bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, ?1)",
            rusqlite::params![texto_contenido],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (1, ?1)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();

    conexion
        .execute(
            "INSERT INTO sonda_semantica (id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms) VALUES (1, 'consulta', ?1, 0.5, 1000)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();

    drop(conexion);

    ConfiguracionDeFragmentacion {
        tamano_de_fragmento: texto_contenido.chars().count(),
        solapamiento: 0,
    }
}

#[test]
fn verificar_guarda_1_reversion_rechazada_por_integridad_estructural() {
    let temp = DirectorioTemporal::nuevo("reversion-guarda-1-estructural");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    // 1. Promover a época 1
    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    // 2. Inyectar un defecto puramente estructural en knowledge_epoch_1.db:
    // Insertar un fragmento huérfano sin vector, manteniendo intactos vectores_de_fragmento
    // y sonda_semantica, de modo que el conjunto de motivos semánticos quede vacío.
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    {
        let conexion = Connection::open(&ruta_epoca_1).expect("abrir epoca 1 para mutar");
        conexion
            .execute(
                "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (99, 1, 99, 'fragmento huerfano')",
                [],
            )
            .expect("insertar fragmento huerfano");
    }

    // 3. Promover a época 2 para que producción esté sirviendo la época 2
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a 2");

    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    assert_eq!(
        fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
        "knowledge_epoch_2.db"
    );

    // 4. Solicitar reversión a época 1: debe rechazarse con IntegridadEstructuralRechazada
    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 1).expect("ejecutar reversion");

    match resultado {
        DesenlaceDeReversion::Rechazada {
            motivo: MotivoDeRechazoDeReversion::IntegridadEstructuralRechazada { motivos },
        } => {
            assert!(!motivos.is_empty(), "debe reportar motivos estructurales");
            assert!(
                motivos.iter().all(|m| !es_motivo_semantico(m)),
                "todos los motivos deben ser estructurales"
            );
        }
        otro => panic!("se esperaba IntegridadEstructuralRechazada, se obtuvo: {otro:?}"),
    }

    // 5. Invariantes: el symlink sigue apuntando a época 2 y ningún archivo fue borrado
    assert_eq!(
        fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
        "knowledge_epoch_2.db"
    );
    assert!(ruta_epoca_1.exists());
    assert!(temp.ruta().join("knowledge_epoch_2.db").exists());
}

#[test]
fn verificar_guarda_2_reversion_rechazada_por_sonda_semantica() {
    let temp = DirectorioTemporal::nuevo("reversion-guarda-2-semantica");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    // 1. Promover a época 1
    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    // 2. Inyectar un defecto puramente semántico en knowledge_epoch_1.db:
    // Estructura 100% intacta, pero elevar el umbral_de_aceptacion por encima del coseno alcanzable (1.0).
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    {
        let conexion = Connection::open(&ruta_epoca_1).expect("abrir epoca 1 para mutar");
        conexion
            .execute(
                "UPDATE sonda_semantica SET umbral_de_aceptacion = 1.05 WHERE id = 1",
                [],
            )
            .expect("actualizar umbral");
    }

    // 3. Promover a época 2
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a 2");

    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    assert_eq!(
        fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
        "knowledge_epoch_2.db"
    );

    // 4. Solicitar reversión a época 1: debe rechazarse con SondaSemanticaRechazada
    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 1).expect("ejecutar reversion");

    match resultado {
        DesenlaceDeReversion::Rechazada {
            motivo:
                MotivoDeRechazoDeReversion::SondaSemanticaRechazada {
                    similitud_observada,
                    umbral_requerido,
                },
        } => {
            assert!(
                similitud_observada < umbral_requerido,
                "similitud {similitud_observada} debe ser menor al umbral {umbral_requerido}"
            );
            assert_eq!(umbral_requerido, 1.05);
        }
        otro => panic!("se esperaba SondaSemanticaRechazada, se obtuvo: {otro:?}"),
    }

    // 5. Invariantes: el symlink sigue apuntando a época 2 y ningún archivo fue borrado
    assert_eq!(
        fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
        "knowledge_epoch_2.db"
    );
    assert!(ruta_epoca_1.exists());
    assert!(temp.ruta().join("knowledge_epoch_2.db").exists());
}

#[test]
fn verificar_ac3_reversion_exitosa_reutiliza_numero_y_archivo_sin_colision() {
    let temp = DirectorioTemporal::nuevo("reversion-ac3-exitosa");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    // Promover a época 1 y luego a época 2
    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a 2");

    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    assert_eq!(
        fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
        "knowledge_epoch_2.db"
    );

    // Revertir a época 1
    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 1).expect("revertir a 1");

    match resultado {
        DesenlaceDeReversion::Revertida {
            numero_de_epoca,
            ruta_del_archivo,
            epoca_superseida,
            duracion_de_conmutacion_ms,
        } => {
            assert_eq!(numero_de_epoca, 1);
            assert_eq!(ruta_del_archivo, temp.ruta().join("knowledge_epoch_1.db"));
            assert_eq!(epoca_superseida.numero_de_epoca(), Some(2));
            assert_eq!(
                epoca_superseida.ruta_del_archivo(),
                fs::canonicalize(temp.ruta().join("knowledge_epoch_2.db")).unwrap()
            );
            assert!(duracion_de_conmutacion_ms.is_finite());
            assert!(duracion_de_conmutacion_ms >= 0.0);
            assert!(duracion_de_conmutacion_ms < 10.0);
        }
        otro => panic!("se esperaba Revertida, se obtuvo: {otro:?}"),
    }

    // Comprobar que el symlink ahora apunta a knowledge_epoch_1.db
    assert_eq!(
        fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
        "knowledge_epoch_1.db"
    );

    // Comprobar que el pool activo sirve lecturas sobre la época 1
    let numero_leido = gestor
        .conocimiento()
        .con_lectura(|c| {
            c.query_row(
                "SELECT numero_de_epoca FROM metadatos_de_epoca WHERE id = 1",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(ErrorDeAlmacen::en("leer numero_de_epoca tras reversion"))
        })
        .unwrap();
    assert_eq!(numero_leido, Some(1));

    // Invariante de conteo de época: con live en época 1 y época 2 aún en disco,
    // numero_de_epoca_siguiente debe retornar 3 (sin huecos ni colisiones).
    let siguiente = numero_de_epoca_siguiente(temp.ruta()).expect("calcular siguiente");
    assert_eq!(siguiente, 3);
}

#[test]
fn verificar_reversion_rechazada_es_inerte() {
    let temp = DirectorioTemporal::nuevo("reversion-inercia");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a 2");

    // Corromper época 1 para inducir rechazo
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    {
        let conexion = Connection::open(&ruta_epoca_1).unwrap();
        conexion
            .execute(
                "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (99, 1, 99, 'huerfano')",
                [],
            )
            .unwrap();
    }

    let pool_antes = gestor.conocimiento();
    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let enlace_antes = fs::read_link(&ruta_live).unwrap();

    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 1).expect("revertir");
    assert!(matches!(resultado, DesenlaceDeReversion::Rechazada { .. }));

    // El puntero de conocimiento es idéntico por referencia Arc::ptr_eq
    let pool_despues = gestor.conocimiento();
    assert!(Arc::ptr_eq(&pool_antes, &pool_despues));

    // El enlace simbólico es idéntico byte a byte
    let enlace_despues = fs::read_link(&ruta_live).unwrap();
    assert_eq!(enlace_antes, enlace_despues);

    // No sobreviven archivos temporales .knowledge_live.tmp.*
    let hay_enlace_temporal = fs::read_dir(temp.ruta())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(".knowledge_live.tmp."))
        });
    assert!(!hay_enlace_temporal);
}

#[test]
fn verificar_reversion_sobre_enlace_colgante_falla_con_error_tipado() {
    let temp = DirectorioTemporal::nuevo("reversion-enlace-colgante");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    // Desplazar el archivo vivo para dejar knowledge_live.db como symlink colgante
    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    fs::rename(&ruta_epoca_1, temp.ruta().join("knowledge_epoch_1.db.bak")).unwrap();

    // Crear un archivo de época 2 para que el destino exista
    let ruta_epoca_2 = temp.ruta().join("knowledge_epoch_2.db");
    fs::copy(temp.ruta().join("knowledge_epoch_1.db.bak"), &ruta_epoca_2).unwrap();

    // Revertir a época 2 con knowledge_live.db colgante debe fallar con EnlaceVivoColgante antes de tocar nada
    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 2);
    match resultado {
        Err(ErrorDeAlmacen::EnlaceVivoColgante { ruta, destino }) => {
            assert_eq!(ruta, ruta_live);
            assert_eq!(destino, ruta_epoca_1);
        }
        otro => panic!("se esperaba Err(EnlaceVivoColgante), se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_reversion_a_epoca_ya_activa_rechaza_con_epoca_ya_es_la_viva() {
    let temp = DirectorioTemporal::nuevo("reversion-ya-activa");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 1).expect("revertir a 1");

    assert_eq!(
        resultado,
        DesenlaceDeReversion::Rechazada {
            motivo: MotivoDeRechazoDeReversion::EpocaYaEsLaViva { numero_de_epoca: 1 },
        }
    );
}

#[test]
fn verificar_reversion_a_epoca_ausente_falla_con_error_tipado() {
    let temp = DirectorioTemporal::nuevo("reversion-epoca-ausente");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 99);
    match resultado {
        Err(ErrorDeAlmacen::EpocaDestinoAusente {
            numero_de_epoca,
            ruta,
        }) => {
            assert_eq!(numero_de_epoca, 99);
            assert_eq!(ruta, temp.ruta().join("knowledge_epoch_99.db"));
        }
        otro => panic!("se esperaba Err(EpocaDestinoAusente), se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_reversion_sin_sonda_rechaza_con_sonda_ausente() {
    let temp = DirectorioTemporal::nuevo("reversion-sonda-ausente");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    // Borrar la fila de sonda semántica en época 1
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    {
        let conexion = Connection::open(&ruta_epoca_1).unwrap();
        conexion
            .execute("DELETE FROM sonda_semantica WHERE id = 1", [])
            .unwrap();
    }

    // Promover a época 2
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a 2");

    let resultado = revertir_a_epoca(&gestor, temp.ruta(), &config, 1).expect("revertir");
    assert_eq!(
        resultado,
        DesenlaceDeReversion::Rechazada {
            motivo: MotivoDeRechazoDeReversion::SondaAusente,
        }
    );
}

#[test]
fn verificar_particion_de_motivos_semanticos_y_estructurales_es_exhaustiva() {
    let motivos_semanticos = [
        MotivoDeRechazo::SimilitudInsuficiente {
            similitud_observada: 0.2,
            umbral_requerido: 0.5,
        },
        MotivoDeRechazo::VectoresIncomparables { cantidad: 1 },
        MotivoDeRechazo::DimensionDeLaSondaDiscrepante {
            dimension_sonda: 768,
            dimension_epoca: 1536,
        },
        MotivoDeRechazo::SondaSemanticaOmitidaPorMetadatosAusentes,
    ];

    for motivo in &motivos_semanticos {
        assert!(
            es_motivo_semantico(motivo),
            "el motivo {motivo:?} debe ser clasificado como semántico"
        );
    }

    let motivos_estructurales = [
        MotivoDeRechazo::MetadatosDeEpocaAusentes,
        MotivoDeRechazo::VectoresHuerfanos { cantidad: 2 },
        MotivoDeRechazo::FaltaContiguidadOrdinal { faltantes: vec![1] },
        MotivoDeRechazo::IndiceVacio,
        MotivoDeRechazo::DiferenciaDeFragmentos {
            esperado: 5,
            recibido: 3,
        },
        MotivoDeRechazo::ConfiguracionDeFragmentacionInvalida {
            tamano_de_fragmento: 10,
            solapamiento: 10,
        },
        MotivoDeRechazo::DimensionDeVectorNoUniforme {
            cantidad_incorrectos: 1,
            dimension_esperada: 768,
        },
        MotivoDeRechazo::CalculoDeCoberturaOmitidoPorMetadatosAusentes,
        MotivoDeRechazo::CalculoDeDimensionOmitidoPorMetadatosAusentes,
    ];

    for motivo in &motivos_estructurales {
        assert!(
            !es_motivo_semantico(motivo),
            "el motivo {motivo:?} debe ser clasificado como estructural"
        );
    }
}
