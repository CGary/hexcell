//! Pruebas de integración para la promoción de épocas y conmutación atómica.
//!
//! Valida la secuencia de seis pasos, verificación de compuertas de aborto limpio,
//! consistencia atómica del enlace simbólico, medición de latencia NFR-03 y entrega viva
//! para drenaje ordenado.

mod comun;

use std::fs;
use std::path::Path;
use std::time::Instant;

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO};
use hexcell_storage::promocion::{
    DesenlaceDePromocion, MotivoDeAbortoDePromocion, numero_de_epoca_siguiente, promover_epoca,
};
use rusqlite::Connection;

/// Fabrica un archivo de staging consistente que supera todas las compuertas de integridad.
fn preparar_staging_valido(ruta_datos: &Path, dimension: usize) -> ConfiguracionDeFragmentacion {
    let ruta_staging = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir base de staging");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    // La ingesta real siempre abre staging con `abrir_lectura_escritura`, que fija el modo WAL
    // desde la primera conexión de escritura. Replicarlo aquí evita que un test que sostiene un
    // lector concurrente choque con un cambio de modo de diario (delete -> wal) que si exige
    // exclusividad, en vez de con el punto de control que es lo que ese test quiere ejercitar.
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

    let texto_contenido = "Texto de catálogo para validación semántica.";
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
fn verificar_ac1_aborto_por_integridad_rechazada_deja_archivos_intactos() {
    let temp = DirectorioTemporal::nuevo("ac1-rechazo-integridad");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir staging");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging");

    // Insertar fragmento huérfano (sin vector) y sonda para inducir rechazo en compuerta
    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref1', 'doc1', 'texto', 1000)",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (1, 1, 0, 'texto')",
            [],
        )
        .unwrap();
    let vector_bytes: Vec<u8> = vec![1.0f32; 768]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    conexion
        .execute(
            "INSERT INTO sonda_semantica (id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms) VALUES (1, 'consulta', ?1, 0.5, 1000)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();
    drop(conexion);

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 0,
    };

    let resultado =
        promover_epoca(&gestor, temp.ruta(), &config, 5000).expect("ejecutar promocion");

    match resultado {
        DesenlaceDePromocion::Abortada {
            motivo: MotivoDeAbortoDePromocion::IntegridadRechazada { motivos },
        } => {
            assert!(!motivos.is_empty());
        }
        _ => panic!("debe abortar por integridad rechazada"),
    }

    // Staging permanece en su sitio y no se crearon archivos de época
    assert!(ruta_staging.exists());
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());
}

#[test]
fn verificar_ac1_aborto_por_sonda_ausente() {
    let temp = DirectorioTemporal::nuevo("ac1-sonda-ausente");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir staging");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging");
    drop(conexion);

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 10,
        solapamiento: 0,
    };

    let resultado =
        promover_epoca(&gestor, temp.ruta(), &config, 5000).expect("ejecutar promocion");

    assert_eq!(
        resultado,
        DesenlaceDePromocion::Abortada {
            motivo: MotivoDeAbortoDePromocion::SondaAusente,
        }
    );
    assert!(ruta_staging.exists());
}

#[test]
fn verificar_ac2_punto_de_control_incompleto_aborta_sin_renombrar() {
    let temp = DirectorioTemporal::nuevo("ac2-checkpoint-incompleto");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");

    // Mantener un lector REAL sobre staging: para que el snapshot quede efectivamente tomado hay
    // que dar un paso (`next()`) sobre las filas, no basta con preparar la sentencia. Ese paso es
    // justo lo que la versión vacía de este test omitía, y por eso su rama de aborto nunca corría.
    let conexion_bloqueante = Connection::open(&ruta_staging).expect("abrir conexion bloqueante");
    conexion_bloqueante.execute("BEGIN;", []).unwrap();
    let mut stmt = conexion_bloqueante
        .prepare("SELECT count(*) FROM metadatos_de_epoca")
        .unwrap();
    let mut filas = stmt.query([]).unwrap();
    let fila = filas
        .next()
        .unwrap()
        .expect("el lector bloqueante debe tomar al menos una fila real");
    let _conteo: i64 = fila.get(0).unwrap();

    let resultado =
        promover_epoca(&gestor, temp.ruta(), &config, 6000).expect("ejecutar promocion");

    match resultado {
        DesenlaceDePromocion::Abortada {
            motivo:
                MotivoDeAbortoDePromocion::PuntoDeControlIncompleto {
                    paginas_en_wal,
                    paginas_consolidadas,
                    ..
                },
        } => {
            // El lector bloqueante impide un TRUNCATE completo: quedan páginas sin consolidar.
            assert!(
                paginas_en_wal > 0 || paginas_consolidadas < paginas_en_wal,
                "se esperaba evidencia real de un checkpoint parcial"
            );
        }
        otro => panic!("se esperaba Abortada{{PuntoDeControlIncompleto}}, se obtuvo: {otro:?}"),
    }

    // Aserciones negativas: nada se renombró mientras el lector seguía vivo.
    assert!(
        ruta_staging.exists(),
        "knowledge_staging.db debe seguir existiendo bajo su propio nombre"
    );
    assert!(
        !ruta_epoca_1.exists(),
        "knowledge_epoch_1.db no debe haberse creado tras un checkpoint incompleto"
    );

    // Liberar el lector y reintentar: la promoción debe completarse limpiamente.
    drop(filas);
    drop(stmt);
    drop(conexion_bloqueante);

    let reintento =
        promover_epoca(&gestor, temp.ruta(), &config, 7000).expect("reintentar promocion");
    match reintento {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca, ..
        } => {
            assert_eq!(numero_de_epoca, 1);
        }
        otro => panic!("el reintento sin lector bloqueante debe promoverse, se obtuvo: {otro:?}"),
    }
    assert!(ruta_epoca_1.exists());
}

#[test]
fn verificar_ac3_sellado_atomico_y_persistencia_sin_wal_huerfano() {
    let temp = DirectorioTemporal::nuevo("ac3-sellado-atomico");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    let ahora = 1_700_000_000_000i64;
    let desenlace = promover_epoca(&gestor, temp.ruta(), &config, ahora).expect("promover");

    match desenlace {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            ruta_del_archivo,
            ..
        } => {
            assert_eq!(numero_de_epoca, 1);
            assert!(ruta_del_archivo.exists());

            // Comprobar que no quedaron páginas pendientes en el archivo WAL y que staging WAL no quedó huérfano
            let mut ruta_wal = ruta_del_archivo.as_os_str().to_owned();
            ruta_wal.push("-wal");
            let tamano_wal = std::fs::metadata(&ruta_wal).map(|m| m.len()).unwrap_or(0);
            assert_eq!(
                tamano_wal, 0,
                "el WAL no debe contener páginas pendientes tras el sellado"
            );

            let mut ruta_staging_wal = temp
                .ruta()
                .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA)
                .into_os_string();
            ruta_staging_wal.push("-wal");
            assert!(
                !Path::new(&ruta_staging_wal).exists(),
                "el WAL de staging no debe quedar huérfano"
            );

            // Validar que la fila de metadatos tiene numero_de_epoca y sellada_ms consistentes
            let conexion = Connection::open_with_flags(
                &ruta_del_archivo,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let (num, sellada): (Option<i64>, Option<i64>) = conexion
                .query_row(
                    "SELECT numero_de_epoca, sellada_ms FROM metadatos_de_epoca WHERE id = 1",
                    [],
                    |fila| Ok((fila.get(0)?, fila.get(1)?)),
                )
                .unwrap();
            assert_eq!(num, Some(1));
            assert_eq!(sellada, Some(ahora));
        }
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción válida no debió abortar: {motivo:?}");
        }
    }
}

#[test]
fn verificar_ac4_y_ac5_conmutacion_de_archivo_regular_y_segunda_epoca() {
    let temp = DirectorioTemporal::nuevo("ac4-ac5-conmutaciones");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Primera promoción: sobre el archivo regular inicial de knowledge_live.db
    let config1 = preparar_staging_valido(temp.ruta(), 768);
    let resultado1 =
        promover_epoca(&gestor, temp.ruta(), &config1, 10_000).expect("primera promocion");

    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    assert!(ruta_live.exists());

    // Comprobar que knowledge_live.db ahora es un enlace simbólico que apunta a knowledge_epoch_1.db
    let destino_enlace = fs::read_link(&ruta_live).expect("leer enlace simbolico");
    assert_eq!(destino_enlace.to_str().unwrap(), "knowledge_epoch_1.db");

    match resultado1 {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            epoca_superseida,
            ..
        } => {
            assert_eq!(numero_de_epoca, 1);
            assert_eq!(epoca_superseida.numero_de_epoca(), None);
        }
        _ => panic!("debe promoverse a epoca 1"),
    }

    // Segunda promoción: steady-state de época 1 a época 2
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    let resultado2 =
        promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("segunda promocion");

    let destino_enlace2 = fs::read_link(&ruta_live).expect("leer enlace simbolico tras epoca 2");
    assert_eq!(destino_enlace2.to_str().unwrap(), "knowledge_epoch_2.db");

    match resultado2 {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            epoca_superseida,
            ..
        } => {
            assert_eq!(numero_de_epoca, 2);
            assert_eq!(epoca_superseida.numero_de_epoca(), Some(1));
        }
        _ => panic!("debe promoverse a epoca 2"),
    }
}

#[test]
fn verificar_ac6_calculo_de_n_desde_contenido_y_no_por_nombre() {
    let temp = DirectorioTemporal::nuevo("ac6-calculo-n");

    // Crear una base con metadatos de época sellada numero = 2 guardada con nombre discrepante
    let ruta_discrepante = temp.ruta().join("knowledge_epoch_99.db");
    let conexion = Connection::open(&ruta_discrepante).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET numero_de_epoca = 2, sellada_ms = 1000 WHERE id = 1",
            [],
        )
        .unwrap();
    drop(conexion);

    // Crear un archivo falso no válido o no sellado para verificar que se omite limpiamente
    let ruta_invalida = temp.ruta().join("archivo_ignorado.db");
    fs::write(&ruta_invalida, b"contenido no sqlite").unwrap();

    let siguiente = numero_de_epoca_siguiente(temp.ruta()).expect("calcular siguiente");
    assert_eq!(siguiente, 3);
}

#[test]
fn verificar_ac7_ac8_ac9_apertura_explicita_drenaje_vivo_y_latencia_nfr03() {
    let temp = DirectorioTemporal::nuevo("ac7-ac8-ac9-latencia");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    let inicio_test = Instant::now();
    let resultado = promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover");

    match resultado {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            ruta_del_archivo,
            epoca_superseida,
            duracion_de_conmutacion_ms,
        } => {
            assert_eq!(numero_de_epoca, 1);

            // AC-7: el nuevo pool apunta a la ruta explícita del archivo de época
            let pool_actual = gestor.conocimiento();
            assert_eq!(pool_actual.ruta(), ruta_del_archivo);

            // AC-8: la época superseída sigue viva y responde a lecturas
            assert!(epoca_superseida.lecturas_en_reposo());
            let cuenta_anterior = epoca_superseida.pool().con_lectura(|c| {
                c.query_row("SELECT count(*) FROM metadatos_de_conocimiento", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(ErrorDeAlmacen::en("lectura de epoca superseida"))
            });
            assert!(cuenta_anterior.is_ok());

            // AC-9: aserción de DOS lados. No basta con que la ventana medida sea corta: una
            // lectura que erró o que nunca llegó a ejecutarse también transcurre rápido, así que
            // se exige además que la lectura de liveness contra el NUEVO pool devuelva el conteo
            // esperado (la tabla metadatos_de_conocimiento siempre está vacía tras una migración).
            let conteo_del_nuevo_pool: i64 = pool_actual
                .con_lectura(|c| {
                    c.query_row("SELECT count(*) FROM metadatos_de_conocimiento", [], |r| {
                        r.get(0)
                    })
                    .map_err(ErrorDeAlmacen::en("lectura de liveness del nuevo pool"))
                })
                .expect("la lectura de liveness del nuevo pool no debe fallar");
            assert_eq!(
                conteo_del_nuevo_pool, 0,
                "metadatos_de_conocimiento debe seguir vacía en una época recién promovida"
            );
            assert!(duracion_de_conmutacion_ms.is_finite());
            assert!(duracion_de_conmutacion_ms >= 0.0);
            assert!(
                duracion_de_conmutacion_ms < 10.0,
                "latencia de conmutacion {duracion_de_conmutacion_ms} ms excede presupuesto de 10 ms"
            );
        }
        _ => panic!("promocion esperada"),
    }

    assert!(inicio_test.elapsed().as_secs() < 5);
}

#[test]
fn verificar_ac10_recuperabilidad_tras_interrupcion_y_reinicio() {
    let temp = DirectorioTemporal::nuevo("ac10-recuperabilidad");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    // Promover normalmente a época 1
    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a 1");

    // Simular un archivo de época huérfano renombrado pero cuyo enlace no conmutó
    let ruta_huerfana = temp.ruta().join("knowledge_epoch_2.db");
    let conexion = Connection::open(&ruta_huerfana).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET numero_de_epoca = 2, sellada_ms = 20_000 WHERE id = 1",
            [],
        )
        .unwrap();
    drop(conexion);

    // Reiniciar gestor abriendo de nuevo la ruta: debe seguir enlazando a época 1 sin corromperse
    let gestor_reiniciado = GestorDePools::abrir(temp.ruta()).expect("reabrir gestor");
    let cuenta = gestor_reiniciado.conocimiento().con_lectura(|c| {
        c.query_row("SELECT count(*) FROM metadatos_de_conocimiento", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(ErrorDeAlmacen::en("consulta liveness"))
    });
    assert!(cuenta.is_ok());
}

#[test]
fn verificar_concurrencia_exclusiva_rechaza_segunda_promocion() {
    let temp = DirectorioTemporal::nuevo("concurrencia-promocion");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Adquirir guardián de promoción manual
    let guardian = gestor.iniciar_promocion().expect("adquirir compuerta");

    // Una segunda llamada concurrente debe fallar con PromocionEnCurso
    let intento = gestor.iniciar_promocion();
    match intento {
        Err(ErrorDeAlmacen::PromocionEnCurso) => {}
        _ => panic!("debe rechazar con PromocionEnCurso"),
    }

    // Liberar la compuerta
    drop(guardian);

    // Debe admitir de nuevo una adquisición posterior
    let segundo_intento = gestor.iniciar_promocion();
    assert!(segundo_intento.is_ok());
}

#[test]
fn verificar_fix_wal_shm_sobreviviente_aborta_sin_borrar_ni_renombrar() {
    let temp = DirectorioTemporal::nuevo("fix-companero-sobreviviente");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");

    // Lector externo que la capa de promoción no conoce: abre una conexión y lee en modo
    // autocommit, sin cerrarla. SQLite mantiene el mapeo de memoria compartida (-shm) vivo
    // mientras esta conexión siga abierta, incluso después de que el propio checkpoint TRUNCATE
    // de la promoción reporte éxito total (0,0,0). Es exactamente el escenario que el contrato
    // nombra como "un lector que esta capa no conocía": el gate debe abortar, nunca borrar.
    let lector_externo = Connection::open(&ruta_staging).expect("abrir lector externo");
    let _conteo: i64 = lector_externo
        .query_row("SELECT count(*) FROM metadatos_de_epoca", [], |fila| {
            fila.get(0)
        })
        .unwrap();

    let resultado = promover_epoca(&gestor, temp.ruta(), &config, 8000);

    match resultado {
        Err(ErrorDeAlmacen::CompanieroDeStagingSobreviviente { ruta }) => {
            let nombre = ruta.to_string_lossy();
            assert!(
                nombre.ends_with("-wal") || nombre.ends_with("-shm"),
                "el error debe nombrar el archivo -wal o -shm sobreviviente: {nombre}"
            );
        }
        otro => panic!("se esperaba Err(CompanieroDeStagingSobreviviente), se obtuvo: {otro:?}"),
    }

    // Verificar-y-abortar, nunca borrar: staging sigue existiendo bajo su propio nombre y no se
    // creó ningún archivo de época a partir de él.
    assert!(
        ruta_staging.exists(),
        "staging debe seguir existiendo intacto tras el aborto"
    );
    assert!(
        !ruta_epoca_1.exists(),
        "no debe haberse creado knowledge_epoch_1.db"
    );

    drop(lector_externo);
}

#[test]
fn verificar_fix_colision_de_epoca_destino_aborta_sin_sobrescribir() {
    let temp = DirectorioTemporal::nuevo("fix-colision-epoca-destino");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    // Simular una época 1 ya sellada ocupando el destino que le tocaría a esta promoción, como
    // si el escaneo de numero_de_epoca_siguiente hubiese omitido detectarla (fallo transitorio
    // de E/S, permisos) y el cálculo hubiera regresado a N=1.
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    let contenido_centinela = b"epoca sellada existente que jamas debe sobrescribirse ni perderse";
    fs::write(&ruta_epoca_1, contenido_centinela).unwrap();

    let resultado = promover_epoca(&gestor, temp.ruta(), &config, 9000);

    match resultado {
        Err(ErrorDeAlmacen::EpocaDestinoYaExiste {
            numero_de_epoca,
            ruta,
        }) => {
            assert_eq!(numero_de_epoca, 1);
            assert_eq!(ruta, ruta_epoca_1);
        }
        otro => panic!("se esperaba Err(EpocaDestinoYaExiste), se obtuvo: {otro:?}"),
    }

    // rename() de POSIX jamás debió invocarse sobre el destino: el archivo existente sobrevive
    // byte a byte y staging sigue existiendo bajo su propio nombre, listo para diagnóstico.
    assert_eq!(
        fs::read(&ruta_epoca_1).unwrap(),
        contenido_centinela,
        "la época existente no debe sobrescribirse jamás"
    );
    assert!(
        temp.ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA)
            .exists(),
        "staging debe seguir existiendo tras el aborto por colisión"
    );
}

#[test]
fn verificar_lector_previo_a_la_conmutacion_sigue_sirviendo_el_inodo_viejo() {
    let temp = DirectorioTemporal::nuevo("lector-inodo-viejo");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Primera promoción: dimensión 768 marca la época 1.
    let config1 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config1, 10_000).expect("promover a epoca 1");

    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);

    // Lector abierto ANTES de la segunda conmutación, a través del enlace simbólico vigente.
    // Un descriptor de archivo abierto en Linux queda atado al inodo, no al nombre: aunque el
    // enlace se reasigne después, esta conexión sigue leyendo el archivo de la época 1.
    let lector_previo = Connection::open(&ruta_live).expect("abrir lector previo");
    let dimension_vista_antes: i64 = lector_previo
        .query_row(
            "SELECT dimension_de_embedding FROM metadatos_de_epoca WHERE id = 1",
            [],
            |fila| fila.get(0),
        )
        .expect("leer dimension antes de la conmutacion");
    assert_eq!(dimension_vista_antes, 768);

    // Segunda promoción: dimensión 1536 marca la época 2 y reasigna el enlace.
    let config2 = preparar_staging_valido(temp.ruta(), 1536);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a epoca 2");

    // El enlace ahora resuelve a la época 2, pero el lector previo sigue viendo la 1.
    let destino_actual = fs::read_link(&ruta_live).expect("leer enlace tras segunda conmutacion");
    assert_eq!(destino_actual.to_str().unwrap(), "knowledge_epoch_2.db");

    let dimension_vista_despues: i64 = lector_previo
        .query_row(
            "SELECT dimension_de_embedding FROM metadatos_de_epoca WHERE id = 1",
            [],
            |fila| fila.get(0),
        )
        .expect("el lector previo debe seguir respondiendo sobre el inodo viejo");
    assert_eq!(
        dimension_vista_despues, 768,
        "el lector abierto antes de la conmutacion debe seguir sirviendo la epoca 1"
    );
}

#[test]
fn verificar_ningun_enlace_temporal_sobrevive_en_el_directorio_de_datos() {
    let temp = DirectorioTemporal::nuevo("sin-enlace-temporal-residual");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    let config1 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config1, 10_000).expect("promover a epoca 1");

    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover a epoca 2");

    // El nombre temporal usado por el modismo de enlace-mas-rename nunca debe quedar residual:
    // rename() lo consume atómicamente sobre knowledge_live.db en cada promoción.
    let hay_enlace_temporal = std::fs::read_dir(temp.ruta())
        .expect("leer directorio de datos")
        .filter_map(|entrada| entrada.ok())
        .any(|entrada| {
            entrada
                .file_name()
                .to_str()
                .is_some_and(|nombre| nombre.starts_with(".knowledge_live.tmp."))
        });
    assert!(
        !hay_enlace_temporal,
        "no debe sobrevivir ningún enlace simbólico temporal en el directorio de datos"
    );
}

#[test]
fn verificar_medio_sellado_es_rechazado_por_el_check_de_metadatos_de_epoca() {
    let temp = DirectorioTemporal::nuevo("medio-sellado-rechazado");
    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir staging");
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging");

    // Fijar solo numero_de_epoca, dejando sellada_ms en NULL: viola
    // CHECK ((numero_de_epoca IS NULL) = (sellada_ms IS NULL)).
    let resultado_solo_numero = conexion.execute(
        "UPDATE metadatos_de_epoca SET numero_de_epoca = 1 WHERE id = 1",
        [],
    );
    assert!(
        resultado_solo_numero.is_err(),
        "fijar numero_de_epoca sin sellada_ms debe violar el CHECK"
    );

    // Fijar solo sellada_ms, dejando numero_de_epoca en NULL: viola el mismo CHECK en sentido
    // contrario, probando que ninguna de las dos mitades del sellado a medias es alcanzable.
    let resultado_solo_sellada = conexion.execute(
        "UPDATE metadatos_de_epoca SET sellada_ms = 5000 WHERE id = 1",
        [],
    );
    assert!(
        resultado_solo_sellada.is_err(),
        "fijar sellada_ms sin numero_de_epoca debe violar el CHECK"
    );

    // La fila permanece intacta: ambos campos siguen NULL, el único estado válido para staging.
    let (num, sellada): (Option<i64>, Option<i64>) = conexion
        .query_row(
            "SELECT numero_de_epoca, sellada_ms FROM metadatos_de_epoca WHERE id = 1",
            [],
            |fila| Ok((fila.get(0)?, fila.get(1)?)),
        )
        .unwrap();
    assert_eq!(num, None);
    assert_eq!(sellada, None);
}

#[test]
fn verificar_abrir_gestor_sobre_enlace_ya_existente_reporta_ambas_sondas_sanas() {
    let temp = DirectorioTemporal::nuevo("abrir-sobre-enlace-existente");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover a epoca 1");

    // knowledge_live.db ya es un enlace simbólico a una época sellada. Reabrir el gestor desde
    // cero (migraciones son no-op sobre un esquema ya en su versión final) no debe escribir en
    // el archivo de época ni impedir que ambas sondas de vitalidad respondan Sana.
    let gestor_reabierto = GestorDePools::abrir(temp.ruta()).expect("reabrir gestor");

    assert_eq!(
        gestor_reabierto.sesiones().vitalidad(),
        hexcell_storage::pools::Vitalidad::Sana
    );
    assert_eq!(
        gestor_reabierto.conocimiento().vitalidad(),
        hexcell_storage::pools::Vitalidad::Sana
    );
}

#[test]
fn verificar_ac10_interrupcion_tras_enlace_simbolico_y_antes_del_swap_de_pool() {
    let temp = DirectorioTemporal::nuevo("ac10-interrupcion-tras-enlace");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let _config = preparar_staging_valido(temp.ruta(), 768);

    // Reproducir a mano los pasos 1 a 4 de promover_epoca sin llegar al swap de ArcSwap,
    // simulando una interrupción justo después de reasignar el enlace simbólico. El gestor
    // original nunca ve la nueva época: su pool sigue siendo el de antes de la promoción.
    let numero_siguiente = numero_de_epoca_siguiente(temp.ruta()).expect("calcular N");
    assert_eq!(numero_siguiente, 1);

    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    if let Some(motivo) = hexcell_storage::promocion::sellar_y_consolidar_staging(
        &ruta_staging,
        numero_siguiente,
        10_000,
    )
    .expect("sellar staging")
    {
        panic!("no se esperaba aborto: {motivo:?}");
    }
    hexcell_storage::promocion::reasignar_enlace_de_la_epoca_viva(
        temp.ruta(),
        &ruta_staging,
        numero_siguiente,
    )
    .expect("reasignar enlace");

    // El puntero en memoria del gestor original nunca se tocó: sigue sirviendo el estado previo
    // a la promoción (la base de conocimiento inicial, sin sellar).
    let cuenta_gestor_original = gestor.conocimiento().con_lectura(|c| {
        c.query_row("SELECT count(*) FROM metadatos_de_conocimiento", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(ErrorDeAlmacen::en("lectura sobre el gestor original"))
    });
    assert!(
        cuenta_gestor_original.is_ok(),
        "el gestor original interrumpido antes del swap debe seguir sirviendo lecturas válidas"
    );

    // Un gestor reabierto desde cero, en cambio, sana solo: sigue el enlace ya reasignado y
    // llega directo a la época nueva, sin código de reparación dedicado.
    let gestor_reiniciado = GestorDePools::abrir(temp.ruta()).expect("reabrir gestor tras crash");
    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let destino = fs::read_link(&ruta_live).expect("leer enlace tras interrupcion simulada");
    assert_eq!(destino.to_str().unwrap(), "knowledge_epoch_1.db");
    let cuenta_gestor_reiniciado = gestor_reiniciado.conocimiento().con_lectura(|c| {
        c.query_row("SELECT count(*) FROM metadatos_de_conocimiento", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(ErrorDeAlmacen::en("lectura sobre el gestor reiniciado"))
    });
    assert!(cuenta_gestor_reiniciado.is_ok());
}

#[test]
fn verificar_guarda_4_canonicalize_ruidoso_en_promocion_aborta_y_es_reintentable() {
    let temp = DirectorioTemporal::nuevo("promocion-guarda-4-canonicalize");
    // Abrir el gestor sobre el directorio limpio (crea knowledge_live.db como archivo regular)
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    let ruta_live = temp.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let ruta_live_bak = temp.ruta().join("knowledge_live.db.bak");
    let ruta_staging = temp
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");

    // Mover knowledge_live.db a un lado para que canonicalize no pueda resolver la ruta física del pool
    fs::rename(&ruta_live, &ruta_live_bak).expect("mover knowledge_live.db");

    // 1. promover_epoca debe abortar ruidosamente con Err(ArchivoDeEpocaInaccesible)
    let resultado = promover_epoca(&gestor, temp.ruta(), &config, 10_000);
    match resultado {
        Err(ErrorDeAlmacen::ArchivoDeEpocaInaccesible {
            ruta, operacion, ..
        }) => {
            assert_eq!(ruta, ruta_live);
            assert_eq!(
                operacion,
                "resolver la ruta fisica de la epoca viva antes de reasignar el enlace"
            );
        }
        otro => panic!("se esperaba Err(ArchivoDeEpocaInaccesible), se obtuvo: {otro:?}"),
    }

    // 2. Aborto LIMPIO: knowledge_epoch_1.db no existe, el symlink no fue reasignado,
    // y knowledge_staging.db sigue existiendo sellado con numero_de_epoca = 1.
    assert!(
        !ruta_epoca_1.exists(),
        "knowledge_epoch_1.db no debe existir tras el aborto"
    );
    assert!(
        !ruta_live.exists(),
        "knowledge_live.db no debió ser reasignado"
    );
    assert!(
        ruta_staging.exists(),
        "knowledge_staging.db debe seguir existiendo intacto"
    );

    // 3. REINTENTABLE: restaurar el archivo live y llamar a promover_epoca de nuevo sobre el MISMO gestor
    fs::rename(&ruta_live_bak, &ruta_live).expect("restaurar knowledge_live.db");

    let resultado_reintento =
        promover_epoca(&gestor, temp.ruta(), &config, 20_000).expect("reintentar promocion");

    match resultado_reintento {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            ruta_del_archivo,
            ..
        } => {
            // numero_de_epoca_siguiente omite knowledge_staging.db por nombre, por lo que recomputa N = 1
            assert_eq!(numero_de_epoca, 1);
            assert_eq!(ruta_del_archivo, ruta_epoca_1);
            assert!(ruta_epoca_1.exists());
            assert_eq!(
                fs::read_link(&ruta_live).unwrap().to_str().unwrap(),
                "knowledge_epoch_1.db"
            );
        }
        otro => panic!("se esperaba Promovida tras reintento, se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_guarda_8_numero_de_epoca_siguiente_reserva_numero_de_epoca_marcada_sospechosa() {
    let temp = DirectorioTemporal::nuevo("guarda-8-reserva-numero");

    // Crear época sellada 1
    let ruta_epoca_1 = temp.ruta().join("knowledge_epoch_1.db");
    let conexion = Connection::open(&ruta_epoca_1).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET numero_de_epoca = 1, sellada_ms = 1000 WHERE id = 1",
            [],
        )
        .unwrap();
    drop(conexion);

    // Escribir una marca de época sospechosa para la época 2 (cuyo .db ya fue purgado)
    hexcell_storage::retencion::escribir_marca_de_epoca_sospechosa(
        temp.ruta(),
        2,
        "epoca purgada por defecto",
        "2026-08-31",
    )
    .expect("escribir marca");

    // numero_de_epoca_siguiente debe calcular max(1, 2) + 1 = 3
    let siguiente = numero_de_epoca_siguiente(temp.ruta()).expect("calcular siguiente");
    assert_eq!(siguiente, 3);
}

#[test]
fn verificar_promocion_registra_epoca_superseida_en_epocas_en_uso() {
    let temp = DirectorioTemporal::nuevo("promocion-registra-en-uso");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Primera promoción: base inicial (None) no se registra en epocas_en_uso
    let config1 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config1, 10_000).expect("promover epoca 1");
    assert!(gestor.epocas_en_uso().is_empty());

    // Segunda promoción: época 1 es superseída y debe registrarse en epocas_en_uso
    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover epoca 2");

    let en_uso = gestor.epocas_en_uso();
    assert_eq!(en_uso.len(), 1);
    assert!(en_uso.contains_key(&1));
    let ruta_canon_1 = std::fs::canonicalize(temp.ruta().join("knowledge_epoch_1.db")).unwrap();
    assert_eq!(en_uso.get(&1).unwrap(), &ruta_canon_1);
}
