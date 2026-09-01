//! Pruebas de integración para la orquestación asíncrona de promoción de épocas en `hexcell`.

mod comun;

use std::time::SystemTime;

use comun::{DirectorioTemporal, abrir_persistencia};
use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell::ingesta::ejecutar_ingesta;
use hexcell::promocion::{
    HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS, drenar_epoca_superseida_de_conocimiento,
    promover_epoca_de_conocimiento, purgar_epocas_de_conocimiento, revertir_epoca_de_conocimiento,
    ventana_de_retencion_de_epocas_desde_entorno,
};
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::DocumentoDeIngesta;
use hexcell_storage::drenaje::DesenlaceDeDrenaje;
use hexcell_storage::promocion::DesenlaceDePromocion;
use hexcell_storage::reversion::DesenlaceDeReversion;

#[tokio::test]
async fn verificar_orquestacion_asincrona_de_promocion_exitosa() {
    let temp = DirectorioTemporal::nuevo("promocion-asincrona");
    let (gestor, repositorio) = abrir_persistencia(temp.ruta());
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let config_fragmentacion = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 32,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    let documento = DocumentoDeIngesta {
        referencia_externa: "doc_test_1".to_string(),
        titulo: "Documento de prueba".to_string(),
        contenido: "Texto completo del documento para ingesta y conmutación asíncrona.".to_string(),
        actualizado_ms: 1000,
    };

    let resumen_ingesta = ejecutar_ingesta(
        documento,
        config_fragmentacion.clone(),
        &servicio,
        temp.ruta(),
        "consulta de sonda",
        0.0,
        || false,
    )
    .await
    .expect("ejecutar ingesta previa");

    assert!(resumen_ingesta.fragmentos_escritos > 0);

    let ahora_ms = 1_700_000_000_000i64;
    let desenlace =
        promover_epoca_de_conocimiento(&gestor, temp.ruta(), &config_fragmentacion, ahora_ms)
            .await
            .expect("promover epoca asincrona");

    match desenlace {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            ruta_del_archivo,
            duracion_de_conmutacion_ms,
            ..
        } => {
            assert_eq!(numero_de_epoca, 1);
            assert!(ruta_del_archivo.exists());
            assert!(duracion_de_conmutacion_ms >= 0.0);
            assert!(duracion_de_conmutacion_ms < 10.0);

            // Validar que el gestor activo sirve lecturas sobre el nuevo archivo
            assert_eq!(gestor.conocimiento().ruta(), ruta_del_archivo);
        }
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción no debió abortar: {motivo:?}");
        }
    }
}

#[tokio::test]
async fn verificar_orquestacion_asincrona_de_drenaje_exitoso() {
    let temp = DirectorioTemporal::nuevo("drenaje-asincrono");
    let (gestor, repositorio) = abrir_persistencia(temp.ruta());
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let config_fragmentacion = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 32,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    let documento = DocumentoDeIngesta {
        referencia_externa: "doc_test_drenaje".to_string(),
        titulo: "Documento de prueba para drenaje".to_string(),
        contenido: "Texto completo del documento para prueba de drenaje asincrono.".to_string(),
        actualizado_ms: 1000,
    };

    let resumen_ingesta = ejecutar_ingesta(
        documento,
        config_fragmentacion.clone(),
        &servicio,
        temp.ruta(),
        "consulta de sonda",
        0.0,
        || false,
    )
    .await
    .expect("ejecutar ingesta previa");

    assert!(resumen_ingesta.fragmentos_escritos > 0);

    let ahora_ms = 1_700_000_000_000i64;
    let desenlace_promocion =
        promover_epoca_de_conocimiento(&gestor, temp.ruta(), &config_fragmentacion, ahora_ms)
            .await
            .expect("promover epoca asincrona");

    let epoca_superseida = match desenlace_promocion {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción no debió abortar: {motivo:?}");
        }
    };

    let desenlace_drenaje = drenar_epoca_superseida_de_conocimiento(&gestor, epoca_superseida)
        .await
        .expect("drenar epoca superseida");

    match desenlace_drenaje {
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

    // Aísla en runtime la compuerta que retira la entrada del registro tras un drenaje exitoso:
    // si el envoltorio asíncrono omitiera `gestor.retirar_epoca_en_uso(&constancia)`, el registro
    // quedaría vacío igual (esta época base nunca se registró en epocas_en_uso porque no vino de
    // una promoción con superseída real), así que esta prueba por sí sola no basta para esa
    // compuerta — se retoma con una época efectivamente registrada más abajo.
    assert!(gestor.epocas_en_uso().is_empty());
}

#[tokio::test]
async fn verificar_orquestacion_asincrona_de_reversion_y_drenaje_exitoso() {
    let temp = DirectorioTemporal::nuevo("reversion-asincrona");
    let (gestor, repositorio) = abrir_persistencia(temp.ruta());
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let config_fragmentacion = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 32,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    // 1. Ingestar y promover primera época
    let documento1 = DocumentoDeIngesta {
        referencia_externa: "doc_test_1".to_string(),
        titulo: "Doc 1".to_string(),
        contenido: "Texto doc 1 para la primera epoca de conocimiento.".to_string(),
        actualizado_ms: 1000,
    };
    ejecutar_ingesta(
        documento1,
        config_fragmentacion.clone(),
        &servicio,
        temp.ruta(),
        "sonda 1",
        0.0,
        || false,
    )
    .await
    .unwrap();

    promover_epoca_de_conocimiento(&gestor, temp.ruta(), &config_fragmentacion, 10_000)
        .await
        .unwrap();

    // 2. Ingestar y promover segunda época
    let documento2 = DocumentoDeIngesta {
        referencia_externa: "doc_test_2".to_string(),
        titulo: "Doc 2".to_string(),
        contenido: "Texto doc 2 para la segunda epoca de conocimiento.".to_string(),
        actualizado_ms: 2000,
    };
    ejecutar_ingesta(
        documento2,
        config_fragmentacion.clone(),
        &servicio,
        temp.ruta(),
        "sonda 2",
        0.0,
        || false,
    )
    .await
    .unwrap();

    promover_epoca_de_conocimiento(&gestor, temp.ruta(), &config_fragmentacion, 20_000)
        .await
        .unwrap();

    // 3. Revertir asíncronamente a la época 1
    let desenlace_reversion =
        revertir_epoca_de_conocimiento(&gestor, temp.ruta(), &config_fragmentacion, 1)
            .await
            .expect("revertir epoca asincrona");

    let epoca_superseida = match desenlace_reversion {
        DesenlaceDeReversion::Revertida {
            numero_de_epoca,
            ruta_del_archivo,
            epoca_superseida,
            ..
        } => {
            assert_eq!(numero_de_epoca, 1);
            assert_eq!(ruta_del_archivo, temp.ruta().join("knowledge_epoch_1.db"));
            assert_eq!(epoca_superseida.numero_de_epoca(), Some(2));
            // Precondición HEX-056: ruta_del_archivo es el archivo resuelto de la época 2, no el enlace
            assert_eq!(
                epoca_superseida.ruta_del_archivo(),
                std::fs::canonicalize(temp.ruta().join("knowledge_epoch_2.db")).unwrap()
            );
            epoca_superseida
        }
        DesenlaceDeReversion::Rechazada { motivo } => {
            panic!("la reversión no debió ser rechazada: {motivo:?}");
        }
    };

    // La época 2 quedó registrada en epocas_en_uso al superseerse por la reversión (GUARD-3):
    // sin drenar todavía, la purga la protegería indefinidamente.
    assert!(gestor.epocas_en_uso().contains_key(&2));

    // 4. Drenar la época superseída (época 2)
    let desenlace_drenaje = drenar_epoca_superseida_de_conocimiento(&gestor, epoca_superseida)
        .await
        .expect("drenar epoca superseida tras reversion");

    match desenlace_drenaje {
        DesenlaceDeDrenaje::Drenada {
            numero_de_epoca,
            ruta_del_archivo,
            ..
        } => {
            assert_eq!(numero_de_epoca, Some(2));
            assert!(ruta_del_archivo.exists());
        }
        otro => panic!("se esperaba Drenada tras reversion, se obtuvo: {otro:?}"),
    }

    // Aísla en runtime la compuerta de retiro del registro: esta época SÍ estaba efectivamente
    // registrada antes del drenaje (a diferencia de la época base del primer test), así que si el
    // envoltorio asíncrono omitiera `gestor.retirar_epoca_en_uso(&constancia)` tras un drenaje
    // exitoso, esta aserción por sí sola fallaría y ninguna otra prueba de este archivo lo haría.
    assert!(
        !gestor.epocas_en_uso().contains_key(&2),
        "drenar_epoca_superseida_de_conocimiento debe retirar la entrada del registro tras un drenaje exitoso"
    );
}

/// Único test que toca `HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS`: se ejercen los tres casos
/// (válido, no numérico, negativo) en la misma función para no arriesgar una carrera con otro
/// test que leyera la misma variable de entorno de proceso en paralelo — ninguna otra prueba de
/// este archivo la toca, así que basta con no repartir los casos en funciones separadas.
#[test]
fn verificar_ventana_de_retencion_desde_entorno_con_valor_valido_no_numerico_y_negativo() {
    // Caso 1: valor numérico válido se respeta tal cual.
    unsafe {
        std::env::set_var(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS, "5");
    }
    assert_eq!(ventana_de_retencion_de_epocas_desde_entorno(), 5);

    // Caso 2: valor no numérico cae al valor por omisión en vez de entrar en pánico.
    unsafe {
        std::env::set_var(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS, "no-es-un-numero");
    }
    assert_eq!(
        ventana_de_retencion_de_epocas_desde_entorno(),
        hexcell_storage::retencion::VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO
    );

    // Caso 3: valor negativo tampoco parsea como usize y cae al mismo valor por omisión.
    unsafe {
        std::env::set_var(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS, "-1");
    }
    assert_eq!(
        ventana_de_retencion_de_epocas_desde_entorno(),
        hexcell_storage::retencion::VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO
    );

    // Caso 4 (variable ausente): mismo valor por omisión, para no dejar la limpieza final como la
    // única prueba de este caso.
    unsafe {
        std::env::remove_var(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS);
    }
    assert_eq!(
        ventana_de_retencion_de_epocas_desde_entorno(),
        hexcell_storage::retencion::VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO
    );
}

/// Orquestación asíncrona de la purga (AC-4, orquestación en `crates/hexcell`): promueve tres
/// épocas, drena la primera superseída para que dejar de estar protegida por `epocas_en_uso`, fija
/// la ventana en 1 vía entorno y confirma que `purgar_epocas_de_conocimiento` elimina exactamente
/// la época que queda fuera de la viva, la ventana y el registro.
#[tokio::test]
async fn verificar_orquestacion_asincrona_de_purga_de_epocas() {
    let temp = DirectorioTemporal::nuevo("purga-asincrona");
    let (gestor, repositorio) = abrir_persistencia(temp.ruta());
    repositorio
        .aportar_presupuesto(1000, SystemTime::now())
        .unwrap();

    let config_fragmentacion = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 32,
        solapamiento: 0,
    };

    let proveedor = ProveedorDeEmbeddingsSimulado::con_dimension(4).con_tamano_de_lote(2);
    let servicio = ServicioDeEmbeddings::nuevo(
        ProveedorDeEmbeddingsDeCelula::Simulado(proveedor),
        repositorio,
    );

    async fn ingestar_y_promover(
        servicio: &ServicioDeEmbeddings<hexcell::embeddings::ProveedorDeEmbeddingsDeCelula>,
        gestor: &hexcell_storage::GestorDePools,
        ruta_datos: &std::path::Path,
        config: &ConfiguracionDeFragmentacion,
        referencia: &str,
        ahora_ms: i64,
    ) -> DesenlaceDePromocion {
        let documento = DocumentoDeIngesta {
            referencia_externa: referencia.to_string(),
            titulo: format!("Documento {referencia}"),
            contenido: "Texto de contenido para orquestación asíncrona de purga.".to_string(),
            actualizado_ms: ahora_ms,
        };
        ejecutar_ingesta(
            documento,
            config.clone(),
            servicio,
            ruta_datos,
            "sonda de purga",
            0.0,
            || false,
        )
        .await
        .expect("ejecutar ingesta previa a la promoción");

        promover_epoca_de_conocimiento(gestor, ruta_datos, config, ahora_ms)
            .await
            .expect("promover epoca asincrona")
    }

    // Época 1
    ingestar_y_promover(
        &servicio,
        &gestor,
        temp.ruta(),
        &config_fragmentacion,
        "doc_purga_1",
        10_000,
    )
    .await;

    // Época 2 (supersede a la 1)
    let desenlace_2 = ingestar_y_promover(
        &servicio,
        &gestor,
        temp.ruta(),
        &config_fragmentacion,
        "doc_purga_2",
        20_000,
    )
    .await;
    let epoca_1_superseida = match desenlace_2 {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        DesenlaceDePromocion::Abortada { motivo } => panic!("no debió abortar: {motivo:?}"),
    };

    // Época 3 (viva final, supersede a la 2)
    ingestar_y_promover(
        &servicio,
        &gestor,
        temp.ruta(),
        &config_fragmentacion,
        "doc_purga_3",
        30_000,
    )
    .await;

    // Drenar la época 1 para que quede disponible a purga (ya no protegida por epocas_en_uso).
    drenar_epoca_superseida_de_conocimiento(&gestor, epoca_1_superseida)
        .await
        .expect("drenar epoca 1 superseida");
    assert!(!gestor.epocas_en_uso().contains_key(&1));

    // Ventana de retención = 1 vía entorno: solo la época 2 (la más reciente no viva) se conserva
    // por recencia; la época 1, ya drenada, queda fuera de las cuatro invariantes y se purga.
    unsafe {
        std::env::set_var(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS, "1");
    }
    let desenlace_purga = purgar_epocas_de_conocimiento(&gestor, temp.ruta())
        .await
        .expect("purgar epocas de conocimiento");
    unsafe {
        std::env::remove_var(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS);
    }

    let numeros_purgados: std::collections::BTreeSet<i64> = desenlace_purga
        .epocas_purgadas
        .iter()
        .map(|p| p.numero_de_epoca)
        .collect();
    assert_eq!(
        numeros_purgados,
        std::collections::BTreeSet::from([1]),
        "debe purgarse exactamente la época 1, ya drenada y fuera de ventana"
    );
    assert!(!temp.ruta().join("knowledge_epoch_1.db").exists());
    assert!(temp.ruta().join("knowledge_epoch_2.db").exists());
    assert!(temp.ruta().join("knowledge_epoch_3.db").exists());
}
