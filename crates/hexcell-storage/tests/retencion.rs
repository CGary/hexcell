//! Pruebas de integración para la retención y purga ordenada de épocas selladas.
//!
//! Valida las cuatro cercas estructurales, las cuatro invariantes de no-purga simultáneas
//! (GUARDA-1 a GUARDA-12), la preservación de diarios con datos sin consolidar, la inmunidad
//! de las marcas sospechosas y la resolución estricta por identidad intrínseca.

mod comun;

use std::fs;
use std::path::Path;
use std::time::Duration;

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
use hexcell_storage::drenaje::drenar_epoca_superseida;
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO};
use hexcell_storage::promocion::promover_epoca;
use hexcell_storage::retencion::{
    EpocaConservada, EpocaPurgada, MotivoDeConservacion, SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA,
    VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO, escribir_marca_de_epoca_sospechosa,
    purgar_epocas_retiradas,
};
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

    let texto_contenido = "Texto de contenido de prueba para retención.";
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

/// Helper para crear una base de época sellada con número intrínseco explícito.
fn crear_epoca_sellada(ruta_datos: &Path, numero: i64, nombre_archivo: &str) {
    let ruta = ruta_datos.join(nombre_archivo);
    let conexion = Connection::open(&ruta).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET numero_de_epoca = ?1, sellada_ms = 1000 WHERE id = 1",
            rusqlite::params![numero],
        )
        .unwrap();
}

#[test]
fn verificar_constantes_nombradas_de_retencion() {
    assert_eq!(VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO, 2);
    assert_eq!(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA, ".sospechosa");
}

#[test]
fn verificar_guarda_1_exclusion_mutua_rechaza_purga_con_promocion_en_curso() {
    let temp = DirectorioTemporal::nuevo("guarda-1-exclusion-mutua");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Adquirir el guardián de promoción
    let _guardian = gestor.iniciar_promocion().expect("iniciar promocion");

    // Invocación concurrente a purgar_epocas_retiradas debe fallar con PromocionEnCurso
    let resultado = purgar_epocas_retiradas(&gestor, temp.ruta(), 2);
    match resultado {
        Err(ErrorDeAlmacen::PromocionEnCurso) => {}
        otro => panic!("se esperaba Err(PromocionEnCurso), se obtuvo: {otro:?}"),
    }
}

#[test]
fn verificar_guarda_2_epoca_viva_sobrevive_intacta_con_ventana_cero() {
    let temp = DirectorioTemporal::nuevo("guarda-2-epoca-viva-ventana-cero");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");
    let config = preparar_staging_valido(temp.ruta(), 768);

    // Promover a época 1
    promover_epoca(&gestor, temp.ruta(), &config, 10_000).expect("promover 1");

    // Con ventana 0, la época viva (1) debe sobrevivir por EsLaEpocaViva
    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar con ventana 0");

    assert!(desenlace.epocas_purgadas.is_empty());
    assert_eq!(desenlace.epocas_conservadas.len(), 1);
    assert_eq!(desenlace.epocas_conservadas[0].numero_de_epoca, 1);
    assert_eq!(
        desenlace.epocas_conservadas[0].motivo,
        MotivoDeConservacion::EsLaEpocaViva
    );
    assert!(temp.ruta().join("knowledge_epoch_1.db").exists());
}

/// GUARD-3 en aislamiento: una época superseída SIN drenar sobrevive a la purga porque
/// `epocas_en_uso` la protege. Se detiene ahí, sin drenar, para que neutralizar
/// `en_uso.contains_key(...)` falle exactamente esta prueba y no una que también drene.
#[test]
fn verificar_guarda_3_superseida_sin_drenar_sobrevive_a_la_purga() {
    let temp = DirectorioTemporal::nuevo("guarda-3-sin-drenar");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Promover época 1 y luego época 2: la 1 queda superseída y registrada sin drenar.
    let config1 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config1, 10_000).expect("promover 1");

    let config2 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover 2");

    // Con ventana 0, época 1 NO se purga porque está en epocas_en_uso, no por recencia.
    let desenlace_purga = purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar");
    assert!(desenlace_purga.epocas_purgadas.is_empty());

    let conservada_1 = desenlace_purga
        .epocas_conservadas
        .iter()
        .find(|c| c.numero_de_epoca == 1)
        .expect("epoca 1 debe estar conservada");
    assert_eq!(
        conservada_1.motivo,
        MotivoDeConservacion::SuperseidaSinDrenar
    );
    assert!(temp.ruta().join("knowledge_epoch_1.db").exists());
}

/// GUARD-4 en aislamiento: presentar la `ConstanciaDeDrenaje` legítima retira la entrada de
/// `epocas_en_uso` y libera esa época a una purga posterior.
///
/// Nota honesta: la NO FALSIFICABILIDAD (campos privados, constructor `pub(crate)` solo en el
/// camino `Drenada`) es una garantía de COMPILACIÓN, ya cubierta por los `verify.commands` del
/// contrato (grep sin `derive(Clone)`/constructor público); no hay rama en EJECUCIÓN que mutar
/// para falsificarla. Esta prueba aísla el comportamiento POSITIVO en su propio nombre, separado
/// de GUARD-3.
#[test]
fn verificar_guarda_4_constancia_legitima_retira_registro_y_libera_purga() {
    let temp = DirectorioTemporal::nuevo("guarda-4-constancia-libera");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    let config1 = preparar_staging_valido(temp.ruta(), 768);
    promover_epoca(&gestor, temp.ruta(), &config1, 10_000).expect("promover 1");

    let config2 = preparar_staging_valido(temp.ruta(), 768);
    let desenlace_promocion =
        promover_epoca(&gestor, temp.ruta(), &config2, 20_000).expect("promover 2");

    let epoca_superseida = match desenlace_promocion {
        hexcell_storage::promocion::DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        _ => panic!("se esperaba Promovida"),
    };

    // Precondición: sin drenar, la época 1 sigue protegida (GUARD-3, probado por separado).
    assert!(gestor.epocas_en_uso().contains_key(&1));

    // Drenar la época superseída para obtener la constancia no falsificable.
    let desenlace_drenaje = drenar_epoca_superseida(epoca_superseida, Duration::from_secs(5))
        .expect("drenar exitosamente");

    let constancia = match desenlace_drenaje {
        hexcell_storage::drenaje::DesenlaceDeDrenaje::Drenada { constancia, .. } => constancia,
        otro => panic!("se esperaba Drenada, se obtuvo: {otro:?}"),
    };

    // Retirar del registro presentando la constancia legítima.
    let retirada = gestor.retirar_epoca_en_uso(&constancia);
    assert!(retirada.is_some());
    assert!(gestor.epocas_en_uso().is_empty());

    // Ahora sí se purga: la constancia legítima fue lo único que liberó la época 1.
    let desenlace_purga_2 =
        purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("segunda purga");

    assert_eq!(desenlace_purga_2.epocas_purgadas.len(), 1);
    assert_eq!(desenlace_purga_2.epocas_purgadas[0].numero_de_epoca, 1);
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());

    assert_eq!(desenlace_purga_2.epocas_conservadas.len(), 1);
    assert_eq!(desenlace_purga_2.epocas_conservadas[0].numero_de_epoca, 2);
    assert_eq!(
        desenlace_purga_2.epocas_conservadas[0].motivo,
        MotivoDeConservacion::EsLaEpocaViva
    );
}

#[test]
fn verificar_guarda_5_ventana_de_retencion_conserva_mas_recientes_y_purga_antiguas() {
    let temp = DirectorioTemporal::nuevo("guarda-5-ventana-retencion");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Crear 4 épocas selladas sanas: 1, 2, 3, 4
    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");
    crear_epoca_sellada(temp.ruta(), 3, "knowledge_epoch_3.db");
    crear_epoca_sellada(temp.ruta(), 4, "knowledge_epoch_4.db");

    // Asignar knowledge_live.db a época 4
    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_4.db",
    )
    .expect("reasignar symlink");

    // Con ventana 2:
    // - Época viva (4): EsLaEpocaViva
    // - Épocas 3 y 2: DentroDeLaVentanaDeRetencion
    // - Época 1: Purgada
    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 2).expect("purgar con ventana 2");

    assert_eq!(desenlace.epocas_purgadas.len(), 1);
    assert_eq!(desenlace.epocas_purgadas[0].numero_de_epoca, 1);
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());

    assert_eq!(desenlace.epocas_conservadas.len(), 3);
    let mapa_conservadas: std::collections::BTreeMap<i64, MotivoDeConservacion> = desenlace
        .epocas_conservadas
        .into_iter()
        .map(|c| (c.numero_de_epoca, c.motivo))
        .collect();

    assert_eq!(
        mapa_conservadas.get(&4),
        Some(&MotivoDeConservacion::EsLaEpocaViva)
    );
    assert_eq!(
        mapa_conservadas.get(&3),
        Some(&MotivoDeConservacion::DentroDeLaVentanaDeRetencion)
    );
    assert_eq!(
        mapa_conservadas.get(&2),
        Some(&MotivoDeConservacion::DentroDeLaVentanaDeRetencion)
    );

    assert!(temp.ruta().join("knowledge_epoch_2.db").exists());
    assert!(temp.ruta().join("knowledge_epoch_3.db").exists());
    assert!(temp.ruta().join("knowledge_epoch_4.db").exists());
}

/// AC-4 COMPUESTA: un único directorio ejerce, en UNA sola pasada de purga, las tres invariantes
/// que se clasifican por época (viva, superseída sin drenar, dentro de ventana) junto con una
/// marcada sin protección de recencia y dos antiguas sanas. La CUARTA invariante (exclusión mutua
/// sobre el destino de reversión) es estructuralmente incompatible con esta pasada: si un
/// `GuardianDePromocion` externo estuviera vivo, la purga entera se rechazaría con
/// `PromocionEnCurso` antes de clasificar nada — ya probado en aislamiento por
/// `verificar_guarda_1_exclusion_mutua_...` — así que no se repite aquí con una época dedicada.
#[test]
fn verificar_ac4_compuesta_cuatro_invariantes_a_la_vez_purga_solo_las_dos_antiguas() {
    let temp = DirectorioTemporal::nuevo("ac4-compuesta-cuatro-invariantes");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Seis épocas selladas sanas de partida; la 3 se marcará sospechosa después.
    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db"); // antigua sana -> purgada
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db"); // antigua sana -> purgada
    crear_epoca_sellada(temp.ruta(), 3, "knowledge_epoch_3.db"); // marcada -> purgada (sin recencia)
    crear_epoca_sellada(temp.ruta(), 4, "knowledge_epoch_4.db"); // dentro de ventana -> conservada
    crear_epoca_sellada(temp.ruta(), 5, "knowledge_epoch_5.db"); // superseída sin drenar -> conservada
    crear_epoca_sellada(temp.ruta(), 6, "knowledge_epoch_6.db"); // viva -> conservada

    // Invariante 1: época viva.
    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_6.db",
    )
    .expect("reasignar symlink a época viva");

    // Invariante 2: superseída sin drenar, registrada directamente (sin pasar por promover_epoca,
    // ya cubierto por GUARD-3) para aislar la protección del registro en sí.
    gestor.registrar_epoca_en_uso(5, temp.ruta().join("knowledge_epoch_5.db"));

    // Marca de sospecha sobre la época 3 (GUARD-11): sin protección de recencia, queda excluida
    // de la competencia por plaza de ventana.
    escribir_marca_de_epoca_sospechosa(temp.ruta(), 3, "sospechosa de defecto", "2026-08-31")
        .expect("escribir marca");

    // Invariante 3, ventana = 2: candidatas sanas no vivas [5, 4, 2, 1] (la 3 marcada queda fuera
    // del cómputo); las dos plazas las toman 5 y 4, pero 5 ya está protegida por en_uso, así que
    // la plaza que efectivamente preserva por recencia es la de la época 4.
    let desenlace =
        purgar_epocas_retiradas(&gestor, temp.ruta(), 2).expect("purgar con las cuatro a la vez");

    let numeros_purgados: std::collections::BTreeSet<i64> = desenlace
        .epocas_purgadas
        .iter()
        .map(|p| p.numero_de_epoca)
        .collect();
    assert_eq!(
        numeros_purgados,
        std::collections::BTreeSet::from([1, 2, 3]),
        "deben purgarse exactamente las dos antiguas (1, 2) y la marcada sin protección (3)"
    );

    let mapa_conservadas: std::collections::BTreeMap<i64, MotivoDeConservacion> = desenlace
        .epocas_conservadas
        .into_iter()
        .map(|c| (c.numero_de_epoca, c.motivo))
        .collect();
    assert_eq!(mapa_conservadas.len(), 3);
    let ruta = temp.ruta();
    let existe = |n: i64| ruta.join(format!("knowledge_epoch_{n}.db")).exists();
    for (n, m) in [
        (6, MotivoDeConservacion::EsLaEpocaViva),
        (5, MotivoDeConservacion::SuperseidaSinDrenar),
        (4, MotivoDeConservacion::DentroDeLaVentanaDeRetencion),
    ] {
        assert_eq!(mapa_conservadas.get(&n), Some(&m));
        assert!(existe(n));
    }
    for n in [1, 2, 3] {
        assert!(!existe(n));
    }
    // La marca de la época 3 sobrevive a su propia purga: es evidencia forense, nunca se borra.
    assert!(temp.ruta().join("knowledge_epoch_3.sospechosa").exists());
}

#[test]
fn verificar_guarda_6_preservacion_de_evidencia_wal_no_vacio_no_se_borra() {
    let temp = DirectorioTemporal::nuevo("guarda-6-wal-no-vacio");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");

    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_2.db",
    )
    .unwrap();

    // Crear un archivo WAL con 4096 bytes no consolidados en época 1
    let ruta_wal_1 = temp.ruta().join("knowledge_epoch_1.db-wal");
    fs::write(&ruta_wal_1, vec![0xEFu8; 4096]).unwrap();

    // Purga con ventana 0: época 1 debería purgarse pero su WAL > 0 la conserva
    let desenlace =
        purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar con WAL no vacio");

    assert!(desenlace.epocas_purgadas.is_empty());
    let conservada_1 = desenlace
        .epocas_conservadas
        .iter()
        .find(|c| c.numero_de_epoca == 1)
        .expect("epoca 1 conservada por evidencia");

    assert_eq!(
        conservada_1.motivo,
        MotivoDeConservacion::DiarioConDatosSinConsolidar { bytes: 4096 }
    );
    assert!(temp.ruta().join("knowledge_epoch_1.db").exists());
    assert!(ruta_wal_1.exists());
}

#[test]
fn verificar_guarda_6_wal_vacio_y_shm_se_eliminan_junto_al_db() {
    let temp = DirectorioTemporal::nuevo("guarda-6-wal-vacio-y-shm");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");

    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_2.db",
    )
    .unwrap();

    let ruta_wal_1 = temp.ruta().join("knowledge_epoch_1.db-wal");
    let ruta_shm_1 = temp.ruta().join("knowledge_epoch_1.db-shm");
    fs::write(&ruta_wal_1, b"").unwrap();
    fs::write(&ruta_shm_1, vec![0u8; 32768]).unwrap();

    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar");

    assert_eq!(desenlace.epocas_purgadas.len(), 1);
    assert_eq!(desenlace.epocas_purgadas[0].numero_de_epoca, 1);
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());
    assert!(!ruta_wal_1.exists(), "-wal de 0 bytes debió eliminarse");
    assert!(!ruta_shm_1.exists(), "-shm debió eliminarse");
}

#[test]
fn verificar_guarda_7_la_marca_es_intocable_tras_purga() {
    let temp = DirectorioTemporal::nuevo("guarda-7-marca-intocable");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");

    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_2.db",
    )
    .unwrap();

    let ruta_marca_1 =
        escribir_marca_de_epoca_sospechosa(temp.ruta(), 1, "sospechosa de defecto", "2026-08-31")
            .expect("escribir marca");

    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar");

    assert_eq!(desenlace.epocas_purgadas.len(), 1);
    assert_eq!(desenlace.epocas_purgadas[0].numero_de_epoca, 1);
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());
    assert!(
        ruta_marca_1.exists(),
        "el archivo .sospechosa NUNCA debe ser eliminado por la purga"
    );
}

#[test]
fn verificar_guarda_11_sospechosa_sin_proteccion_de_recencia_se_purga() {
    let temp = DirectorioTemporal::nuevo("guarda-11-sospechosa-sin-recencia");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Tres épocas: 1, 2, 3.
    // Época viva es 1 (tras reversión desde época 3).
    // Época 3 porta marca sospechosa.
    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");
    crear_epoca_sellada(temp.ruta(), 3, "knowledge_epoch_3.db");

    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_1.db",
    )
    .unwrap();

    let ruta_marca_3 =
        escribir_marca_de_epoca_sospechosa(temp.ruta(), 3, "revertida por defecto", "2026-08-31")
            .expect("escribir marca 3");

    // Con ventana 1:
    // - Época 1: EsLaEpocaViva
    // - Época 2 (sana): DentroDeLaVentanaDeRetencion (ocupa la única plaza disponible)
    // - Época 3 (marcada): NO ocupa plaza y SE PURGA a pesar de tener número mayor que 2
    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 1).expect("purgar con ventana 1");

    assert_eq!(desenlace.epocas_purgadas.len(), 1);
    assert_eq!(desenlace.epocas_purgadas[0].numero_de_epoca, 3);
    assert!(!temp.ruta().join("knowledge_epoch_3.db").exists());
    assert!(ruta_marca_3.exists(), "la marca 3 sobrevive");

    let mapa_conservadas: std::collections::BTreeMap<i64, MotivoDeConservacion> = desenlace
        .epocas_conservadas
        .into_iter()
        .map(|c| (c.numero_de_epoca, c.motivo))
        .collect();

    assert_eq!(
        mapa_conservadas.get(&1),
        Some(&MotivoDeConservacion::EsLaEpocaViva)
    );
    assert_eq!(
        mapa_conservadas.get(&2),
        Some(&MotivoDeConservacion::DentroDeLaVentanaDeRetencion)
    );
}

/// Aísla el brazo NUMÉRICO del OR de `es_viva` (`Some(num_epoca) == numero_vivo_intrinseco`) del
/// brazo de RUTA CANÓNICA (`ruta_canonica == ruta_live_canonica`): para la época viva real ambos
/// son siempre verdaderos a la vez (mismo archivo, misma fila), así que uno enmascara al otro. Se
/// fabrica un ARCHIVO DISTINTO, con ruta diferente al enlace vivo, que por corrupción quedó
/// grabado con el MISMO número intrínseco que la época viva: el brazo de ruta es falso ahí, solo
/// el numérico lo protege. Sin ese brazo entraría a la ventana como candidato más y se purgaría.
#[test]
fn verificar_es_viva_brazo_numerico_protege_duplicado_con_ruta_distinta() {
    let temp = DirectorioTemporal::nuevo("es-viva-brazo-numerico");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Época viva real: número 4, en su archivo canónico.
    crear_epoca_sellada(temp.ruta(), 4, "knowledge_epoch_4.db");
    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_4.db",
    )
    .unwrap();

    // Archivo DISTINTO, no destino del enlace (brazo de ruta falso), grabado con el MISMO número
    // intrínseco 4 por corrupción.
    crear_epoca_sellada(temp.ruta(), 4, "knowledge_epoch_4_duplicado.db");

    // Control: época antigua sana, debe purgarse igual que siempre con ventana 0.
    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");

    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar");

    assert_eq!(desenlace.epocas_purgadas.len(), 1);
    assert_eq!(desenlace.epocas_purgadas[0].numero_de_epoca, 1);
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());

    // Ambos archivos de número 4 sobreviven, clasificados como EsLaEpocaViva: el real por brazo
    // de ruta, el duplicado únicamente por brazo numérico.
    let conservadas_4: Vec<&EpocaConservada> = desenlace
        .epocas_conservadas
        .iter()
        .filter(|c| c.numero_de_epoca == 4)
        .collect();
    assert_eq!(
        conservadas_4.len(),
        2,
        "ambos archivos de número 4 deben sobrevivir"
    );
    assert!(
        conservadas_4
            .iter()
            .all(|c| c.motivo == MotivoDeConservacion::EsLaEpocaViva)
    );
    assert!(temp.ruta().join("knowledge_epoch_4.db").exists());
    assert!(temp.ruta().join("knowledge_epoch_4_duplicado.db").exists());
}

#[test]
fn verificar_guarda_12_enlace_vivo_colgante_aborta_sin_purgar_nada() {
    let temp = DirectorioTemporal::nuevo("guarda-12-enlace-colgante");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_1.db",
    )
    .unwrap();

    // Eliminar knowledge_epoch_1.db para dejar knowledge_live.db colgante
    fs::remove_file(temp.ruta().join("knowledge_epoch_1.db")).unwrap();

    // Crear otra época 2
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");

    let resultado = purgar_epocas_retiradas(&gestor, temp.ruta(), 0);
    match resultado {
        Err(ErrorDeAlmacen::EnlaceVivoColgante { .. }) => {}
        otro => panic!("se esperaba Err(EnlaceVivoColgante), se obtuvo: {otro:?}"),
    }

    // Época 2 no debe haber sido tocada
    assert!(temp.ruta().join("knowledge_epoch_2.db").exists());
}

#[test]
fn verificar_punto_ciego_a_identidad_intrinseca_del_archivo_prevalece_sobre_nombre() {
    let temp = DirectorioTemporal::nuevo("punto-ciego-a-identidad-intrinseca");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    // Archivo nombrado "knowledge_epoch_99.db" pero con numero_de_epoca = 3 grabado adentro
    crear_epoca_sellada(temp.ruta(), 3, "knowledge_epoch_99.db");
    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 4, "knowledge_epoch_4.db");

    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_4.db",
    )
    .unwrap();

    // Con ventana 1:
    // - Viva: 4
    // - Ventana 1 toma la de número intrínseco más alto: 3 (archivo knowledge_epoch_99.db)
    // - Época 1: Purgada
    let desenlace = purgar_epocas_retiradas(&gestor, temp.ruta(), 1).expect("purgar");

    assert_eq!(desenlace.epocas_purgadas.len(), 1);
    assert_eq!(desenlace.epocas_purgadas[0].numero_de_epoca, 1);
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());

    let conservada_3 = desenlace
        .epocas_conservadas
        .iter()
        .find(|c| c.numero_de_epoca == 3)
        .expect("epoca intrinseca 3 conservada");
    assert_eq!(
        conservada_3.motivo,
        MotivoDeConservacion::DentroDeLaVentanaDeRetencion
    );
    assert_eq!(
        conservada_3.ruta_del_archivo,
        temp.ruta().join("knowledge_epoch_99.db")
    );
    assert!(temp.ruta().join("knowledge_epoch_99.db").exists());
}

#[test]
fn verificar_punto_ciego_b_marca_con_numero_discrepante_aborta_toda_la_purga() {
    let temp = DirectorioTemporal::nuevo("punto-ciego-b-marca-discrepante");
    let gestor = GestorDePools::abrir(temp.ruta()).expect("abrir gestor");

    crear_epoca_sellada(temp.ruta(), 1, "knowledge_epoch_1.db");
    crear_epoca_sellada(temp.ruta(), 2, "knowledge_epoch_2.db");
    hexcell_storage::promocion::reasignar_enlace_simbolico_vivo(
        temp.ruta(),
        "knowledge_epoch_2.db",
    )
    .unwrap();

    // Escribir archivo de marca con nombre "knowledge_epoch_1.sospechosa" pero contenido "numero_de_epoca: 99"
    let ruta_marca = temp.ruta().join("knowledge_epoch_1.sospechosa");
    fs::write(
        &ruta_marca,
        "numero_de_epoca: 99\nmotivo: corrupto\nfecha_absoluta: 2026-08-31\n",
    )
    .unwrap();

    let resultado = purgar_epocas_retiradas(&gestor, temp.ruta(), 0);
    match resultado {
        Err(ErrorDeAlmacen::NumeroDeMarcaDiscrepante {
            ruta,
            numero_en_nombre,
            numero_en_contenido,
        }) => {
            assert_eq!(ruta, ruta_marca);
            assert_eq!(numero_en_nombre, 1);
            assert_eq!(numero_en_contenido, 99);
        }
        otro => panic!("se esperaba Err(NumeroDeMarcaDiscrepante), se obtuvo: {otro:?}"),
    }

    // Ningún archivo de época fue purgado
    assert!(temp.ruta().join("knowledge_epoch_1.db").exists());
    assert!(temp.ruta().join("knowledge_epoch_2.db").exists());
}
