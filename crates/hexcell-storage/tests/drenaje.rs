//! Pruebas de integración para el drenaje ordenado y acotado de épocas superseídas.
//!
//! Valida el predicado de dos lados, expiración con fallo cerrado conservando el pool vivo,
//! reintentabilidad tras expiración, verificación de archivos secundarios por tamaño (RISK-1)
//! y ausencia de eliminación de archivos.

mod comun;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::{
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA, SUFIJO_DE_ARCHIVO_SHM,
};
use hexcell_storage::drenaje::{
    DesenlaceDeDrenaje, INTERVALO_DE_SONDEO_DE_DRENAJE, LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO,
    drenar_epoca_superseida,
};
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{GestorDePools, SUFIJO_DE_ARCHIVO_WAL};
use hexcell_storage::promocion::{DesenlaceDePromocion, EpocaSuperseida, promover_epoca};
use rusqlite::Connection;

/// Prepara un archivo de base de datos en staging válido para ejecutar conmutaciones.
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

    let texto = "Contenido de prueba para drenaje de epocas.";
    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_1', 'Titulo 1', ?1, 1000)",
            rusqlite::params![texto],
        )
        .unwrap();

    let vector = vec![1.0f32; dimension];
    let vector_bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();

    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, ?1)",
            rusqlite::params![texto],
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
        tamano_de_fragmento: texto.chars().count(),
        solapamiento: 0,
    }
}

/// Ejecuta una promoción válida y extrae la época superseída viva resultante.
fn obtener_epoca_superseida_promovida(
    ruta_datos: &Path,
    gestor: &GestorDePools,
) -> EpocaSuperseida {
    let config = preparar_staging_valido(ruta_datos, 768);
    let desenlace = promover_epoca(gestor, ruta_datos, &config, 10_000).expect("promocion exitosa");
    match desenlace {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promocion previa no debio abortar: {motivo:?}");
        }
    }
}

#[test]
fn verificar_ac1_drenaje_espera_a_lector_activo_y_completa_en_reposo() {
    let temp = DirectorioTemporal::nuevo("ac1-espera-lector");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let pool_clon = Arc::clone(epoca.pool());
    let (tx_iniciado, rx_iniciado) = mpsc::channel();
    let (tx_liberar, rx_liberar) = mpsc::channel();

    let hilo_lector = thread::spawn(move || {
        pool_clon
            .con_lectura(|conexion| {
                tx_iniciado.send(()).unwrap();
                rx_liberar.recv().unwrap();
                let cuenta: i64 = conexion
                    .query_row(
                        "SELECT count(*) FROM metadatos_de_conocimiento",
                        [],
                        |fila| fila.get(0),
                    )
                    .unwrap();
                assert_eq!(cuenta, 0);
                Ok(())
            })
            .unwrap();
        drop(pool_clon);
    });

    rx_iniciado.recv().unwrap();

    let (tx_resultado, rx_resultado) = mpsc::channel();
    let hilo_drenaje = thread::spawn(move || {
        let res = drenar_epoca_superseida(epoca, Duration::from_secs(5));
        tx_resultado.send(res).unwrap();
    });

    // Mantener la retención por un lapso medible antes de liberar
    thread::sleep(Duration::from_millis(60));
    tx_liberar.send(()).unwrap();
    hilo_lector.join().unwrap();

    let desenlace = rx_resultado.recv().unwrap().expect("drenaje exitoso");
    hilo_drenaje.join().unwrap();

    match desenlace {
        DesenlaceDeDrenaje::Drenada {
            numero_de_epoca,
            ruta_del_archivo,
            espera_ms,
            ref constancia,
        } => {
            assert_eq!(numero_de_epoca, None);
            assert_eq!(constancia.numero_de_epoca(), None);
            assert_eq!(constancia.ruta_del_archivo(), ruta_del_archivo.as_path());
            assert!(ruta_del_archivo.exists());
            assert!(
                espera_ms >= 50,
                "espera_ms ({espera_ms}) debio ser mayor al tiempo retenido"
            );
        }
        otro => panic!("se esperaba Drenada, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac1_predicado_de_dos_lados_lector_liberado_pero_arc_retenido() {
    let temp = DirectorioTemporal::nuevo("ac1-dos-lados-arc-retenido");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    // Retener un clon del Arc sin ninguna consulta activa
    let clon_retenido = Arc::clone(epoca.pool());

    // Las lecturas estan en reposo, pero strong_count es 2
    assert!(epoca.lecturas_en_reposo());
    assert_eq!(Arc::strong_count(epoca.pool()), 2);

    let resultado = drenar_epoca_superseida(epoca, Duration::from_millis(50)).expect("drenar");

    match resultado {
        DesenlaceDeDrenaje::Expirada {
            epoca_superseida,
            titulares,
            lecturas_en_reposo,
        } => {
            assert!(lecturas_en_reposo);
            assert_eq!(titulares, 2);
            assert!(epoca_superseida.ruta_del_archivo().exists());
        }
        otro => panic!("se esperaba Expirada por fuerte retencion de Arc, se obtuvo: {otro:?}"),
    }

    drop(clon_retenido);
}

#[test]
fn verificar_ac2_lector_bloqueante_expira_mantiene_pool_vivo_y_sin_borrar_archivos() {
    let temp = DirectorioTemporal::nuevo("ac2-lector-bloqueante-expira");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let pool_clon = Arc::clone(epoca.pool());
    let (tx_iniciado, rx_iniciado) = mpsc::channel();
    let (tx_terminar, rx_terminar) = mpsc::channel();

    let hilo_lector = thread::spawn(move || {
        pool_clon
            .con_lectura(|_conexion| {
                tx_iniciado.send(()).unwrap();
                rx_terminar.recv().unwrap();
                Ok(())
            })
            .unwrap();
        drop(pool_clon);
    });

    rx_iniciado.recv().unwrap();

    let listado_previo: Vec<_> = fs::read_dir(temp.ruta())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let resultado = drenar_epoca_superseida(epoca, Duration::from_millis(50)).expect("drenar");

    match resultado {
        DesenlaceDeDrenaje::Expirada {
            epoca_superseida,
            titulares,
            lecturas_en_reposo,
        } => {
            assert!(!lecturas_en_reposo);
            assert!(titulares >= 2);
            assert!(epoca_superseida.ruta_del_archivo().exists());

            // Liberar el lector
            tx_terminar.send(()).unwrap();
            hilo_lector.join().unwrap();

            // Validar que el pool retornado sigue vivo y responde a lecturas
            let consulta = epoca_superseida.pool().con_lectura(|c| {
                c.query_row(
                    "SELECT count(*) FROM metadatos_de_conocimiento",
                    [],
                    |fila| fila.get::<_, i64>(0),
                )
                .map_err(ErrorDeAlmacen::en("lectura posterior"))
            });
            assert_eq!(consulta.unwrap(), 0);

            // Comprobar que ningun archivo fue eliminado
            let listado_posterior: Vec<_> = fs::read_dir(temp.ruta())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(listado_previo, listado_posterior);
        }
        otro => panic!("se esperaba Expirada, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac2_expirada_es_reintentable_tras_liberar_lector() {
    let temp = DirectorioTemporal::nuevo("ac2-reintentable");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let pool_clon = Arc::clone(epoca.pool());
    let (tx_iniciado, rx_iniciado) = mpsc::channel();
    let (tx_terminar, rx_terminar) = mpsc::channel();

    let hilo_lector = thread::spawn(move || {
        pool_clon
            .con_lectura(|_c| {
                tx_iniciado.send(()).unwrap();
                rx_terminar.recv().unwrap();
                Ok(())
            })
            .unwrap();
        drop(pool_clon);
    });

    rx_iniciado.recv().unwrap();

    let primer_intento =
        drenar_epoca_superseida(epoca, Duration::from_millis(40)).expect("primer drenaje");

    let epoca_viva = match primer_intento {
        DesenlaceDeDrenaje::Expirada {
            epoca_superseida, ..
        } => epoca_superseida,
        otro => panic!("se esperaba Expirada en primer intento, se obtuvo: {otro:?}"),
    };

    tx_terminar.send(()).unwrap();
    hilo_lector.join().unwrap();

    // Reintento tras la liberacion del lector debe concluir en Drenada
    let segundo_intento =
        drenar_epoca_superseida(epoca_viva, Duration::from_secs(5)).expect("segundo drenaje");

    match segundo_intento {
        DesenlaceDeDrenaje::Drenada {
            numero_de_epoca, ..
        } => {
            assert_eq!(numero_de_epoca, None);
        }
        otro => panic!("se esperaba Drenada en reintento, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac3_drenaje_limpio_sin_companeros_reporta_exito() {
    let temp = DirectorioTemporal::nuevo("ac3-drenaje-limpio");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let resultado = drenar_epoca_superseida(epoca, Duration::from_secs(5)).expect("drenaje");

    match resultado {
        DesenlaceDeDrenaje::Drenada {
            numero_de_epoca,
            ruta_del_archivo,
            ..
        } => {
            assert_eq!(numero_de_epoca, None);
            assert!(ruta_del_archivo.exists());
        }
        otro => panic!("se esperaba Drenada, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac3_regresion_residuo_wal_cero_bytes_y_shm_es_tolerado() {
    let temp = DirectorioTemporal::nuevo("ac3-residuo-wal-cero-bytes");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let mut ruta_wal = epoca.ruta_del_archivo().as_os_str().to_owned();
    ruta_wal.push(SUFIJO_DE_ARCHIVO_WAL);
    let ruta_wal = std::path::PathBuf::from(ruta_wal);
    fs::write(&ruta_wal, b"").unwrap();

    let mut ruta_shm = epoca.ruta_del_archivo().as_os_str().to_owned();
    ruta_shm.push(SUFIJO_DE_ARCHIVO_SHM);
    let ruta_shm = std::path::PathBuf::from(ruta_shm);
    fs::write(&ruta_shm, vec![0u8; 32768]).unwrap();

    let resultado = drenar_epoca_superseida(epoca, Duration::from_secs(5)).expect("drenaje");

    match resultado {
        DesenlaceDeDrenaje::Drenada {
            numero_de_epoca,
            ruta_del_archivo,
            ..
        } => {
            assert_eq!(numero_de_epoca, None);
            assert!(ruta_del_archivo.exists());
            assert!(ruta_wal.exists());
            assert!(ruta_shm.exists());
        }
        otro => panic!("se esperaba Drenada tolerando residuo, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac4_wal_no_vacio_sobreviviente_aborta_con_error_y_no_lo_borra() {
    let temp = DirectorioTemporal::nuevo("ac4-wal-no-vacio");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let mut ruta_wal = epoca.ruta_del_archivo().as_os_str().to_owned();
    ruta_wal.push(SUFIJO_DE_ARCHIVO_WAL);
    let ruta_wal = std::path::PathBuf::from(ruta_wal);
    fs::write(&ruta_wal, vec![0xABu8; 4096]).unwrap();

    let resultado = drenar_epoca_superseida(epoca, Duration::from_secs(5));

    match resultado {
        Err(ErrorDeAlmacen::CompanieroDeEpocaSobreviviente { ruta, bytes }) => {
            assert_eq!(ruta, ruta_wal);
            assert_eq!(bytes, 4096);
            assert!(ruta_wal.exists(), "el archivo no debio eliminarse");
        }
        otro => panic!("se esperaba Err(CompanieroDeEpocaSobreviviente), se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_ac3_shm_solitario_sin_wal_es_tolerado_y_no_se_borra() {
    let temp = DirectorioTemporal::nuevo("ac3-shm-solitario");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);

    let mut ruta_shm = epoca.ruta_del_archivo().as_os_str().to_owned();
    ruta_shm.push(SUFIJO_DE_ARCHIVO_SHM);
    let ruta_shm = std::path::PathBuf::from(ruta_shm);
    fs::write(&ruta_shm, vec![0u8; 32768]).unwrap();

    let resultado = drenar_epoca_superseida(epoca, Duration::from_secs(5)).expect("drenaje");

    match resultado {
        DesenlaceDeDrenaje::Drenada { .. } => {
            assert!(ruta_shm.exists(), "el archivo shm no debio eliminarse");
        }
        otro => panic!("se esperaba Drenada tolerando shm solitario, se obtuvo: {otro:?}"),
    }
}

/// Regresión: tras un reinicio del proceso, el pool se abre por el ENLACE `knowledge_live.db`, pero
/// SQLite nombra su diario según el destino resuelto. Si la época superseída guardara la ruta de
/// apertura en vez de la física, la compuerta de AC-4 inspeccionaría el diario equivocado y
/// declararía limpia una época que conserva datos sin consolidar. Las demás pruebas no lo detectan
/// porque siempre drenan la PRIMERA época de un directorio nuevo, el único caso en que ambas rutas
/// coinciden por accidente.
#[test]
fn verificar_ac4_epoca_abierta_por_enlace_verifica_el_diario_fisico_y_no_el_del_enlace() {
    let temp = DirectorioTemporal::nuevo("ac4-epoca-tras-reinicio");

    // Primera promoción: nace `knowledge_epoch_1.db` y `knowledge_live.db` pasa a ser un enlace.
    let gestor_inicial = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let epoca_inicial = obtener_epoca_superseida_promovida(temp.ruta(), &gestor_inicial);
    drop(epoca_inicial);
    drop(gestor_inicial);

    // Reinicio del proceso: el gestor nuevo abre el pool a través del enlace.
    let gestor = GestorDePools::abrir(temp.ruta()).expect("reabrir gestor tras reinicio");
    let ruta_epoca_fisica = temp.ruta().join("knowledge_epoch_1.db");
    assert!(
        ruta_epoca_fisica.exists(),
        "la primera promocion debio dejar la epoca fisica en disco"
    );
    // El directorio temporal puede colgar de una ruta con enlaces (TMPDIR hacia /private/tmp, por
    // ejemplo). Se compara canonica contra canonica para que la prueba falle por el defecto que
    // vigila y nunca por la topologia del sistema de archivos que la hospeda.
    let ruta_epoca_fisica =
        std::fs::canonicalize(&ruta_epoca_fisica).expect("canonicalizar la epoca fisica");

    // Datos sin consolidar en el diario de la época FÍSICA: justo lo que AC-4 debe detener.
    let mut ruta_wal_fisico = ruta_epoca_fisica.as_os_str().to_owned();
    ruta_wal_fisico.push(SUFIJO_DE_ARCHIVO_WAL);
    let ruta_wal_fisico = std::path::PathBuf::from(ruta_wal_fisico);
    fs::write(&ruta_wal_fisico, vec![0xCDu8; 4096]).unwrap();

    let epoca = obtener_epoca_superseida_promovida(temp.ruta(), &gestor);
    assert_eq!(
        epoca.ruta_del_archivo(),
        ruta_epoca_fisica.as_path(),
        "la epoca superseida debe apuntar al archivo fisico, no al enlace"
    );

    let resultado = drenar_epoca_superseida(epoca, Duration::from_secs(5));

    match resultado {
        Err(ErrorDeAlmacen::CompanieroDeEpocaSobreviviente { ruta, bytes }) => {
            assert_eq!(ruta, ruta_wal_fisico);
            assert_eq!(bytes, 4096);
            assert!(ruta_wal_fisico.exists(), "el archivo no debio eliminarse");
        }
        otro => panic!("se esperaba Err por el diario fisico no consolidado, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_constantes_nombradas_de_drenaje() {
    assert_eq!(
        LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO,
        Duration::from_secs(10)
    );
    assert_eq!(INTERVALO_DE_SONDEO_DE_DRENAJE, Duration::from_millis(5));
}
