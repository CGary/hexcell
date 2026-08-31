//! Pruebas de integración para la orquestación asíncrona de promoción de épocas en `hexcell`.

mod comun;

use std::time::SystemTime;

use comun::{DirectorioTemporal, abrir_persistencia};
use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell::ingesta::ejecutar_ingesta;
use hexcell::promocion::{
    drenar_epoca_superseida_de_conocimiento, promover_epoca_de_conocimiento,
    revertir_epoca_de_conocimiento,
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

    let desenlace_drenaje = drenar_epoca_superseida_de_conocimiento(epoca_superseida)
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

    // 4. Drenar la época superseída (época 2)
    let desenlace_drenaje = drenar_epoca_superseida_de_conocimiento(epoca_superseida)
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
}
