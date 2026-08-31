//! Pruebas de integración para la orquestación asíncrona de promoción de épocas en `hexcell`.

mod comun;

use std::time::SystemTime;

use comun::{DirectorioTemporal, abrir_persistencia};
use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell::ingesta::ejecutar_ingesta;
use hexcell::promocion::{drenar_epoca_superseida_de_conocimiento, promover_epoca_de_conocimiento};
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::DocumentoDeIngesta;
use hexcell_storage::drenaje::DesenlaceDeDrenaje;
use hexcell_storage::promocion::DesenlaceDePromocion;

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
