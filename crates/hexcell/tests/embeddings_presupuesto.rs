//! Tests de integración de ServicioDeEmbeddings con la contabilidad de dos fases (AC-4, AC-5).

use std::sync::Arc;
use std::time::SystemTime;

use hexcell::embeddings::{
    ErrorDeServicioDeEmbeddings, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell_core::embeddings::{LoteDeEmbeddings, PeticionDeEmbeddings};
use hexcell_core::presupuesto::estimar_coste_de_lote;
use hexcell_storage::{GestorDePools, RepositorioDeSesiones};

mod comun;
use comun::DirectorioTemporal;

fn preparar_repositorio(
    dir: &DirectorioTemporal,
    saldo_inicial: u64,
) -> Arc<RepositorioDeSesiones> {
    let pools = Arc::new(GestorDePools::abrir(dir.ruta()).expect("abrir pools de prueba"));
    let repo = Arc::new(RepositorioDeSesiones::nuevo(pools));
    if saldo_inicial > 0 {
        repo.aportar_presupuesto(saldo_inicial, SystemTime::UNIX_EPOCH)
            .expect("aportar saldo inicial");
    }
    repo
}

#[tokio::test]
async fn reserva_rechazada_por_saldo_insuficiente_aborta_sin_llamar_proveedor() {
    let dir = DirectorioTemporal::nuevo("emb-sin-saldo");
    let repo = preparar_repositorio(&dir, 0); // Saldo 0

    let proveedor = ProveedorDeEmbeddingsSimulado::nuevo();
    let servicio = ServicioDeEmbeddings::nuevo(proveedor, Arc::clone(&repo));

    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto que requiere presupuesto".to_string()],
    };

    let error = servicio
        .incrustar_lote(peticion, SystemTime::UNIX_EPOCH)
        .await
        .expect_err("debe fallar por falta de saldo");

    match error {
        ErrorDeServicioDeEmbeddings::PresupuestoAgotado {
            disponible,
            requerido,
        } => {
            assert_eq!(disponible, 0);
            assert!(requerido > 0);
        }
        otro => panic!("se esperaba PresupuestoAgotado, se obtuvo {otro:?}"),
    }

    let saldo = repo.saldo().expect("consultar saldo");
    assert_eq!(saldo.disponible, 0);
    assert_eq!(saldo.reservado, 0);
}

#[tokio::test]
async fn llamada_exitosa_reserva_y_concilia_uso_real() {
    let dir = DirectorioTemporal::nuevo("emb-exito");
    let saldo_inicial = 1000;
    let repo = preparar_repositorio(&dir, saldo_inicial);

    let unidades_reales = 25;
    let proveedor =
        ProveedorDeEmbeddingsSimulado::nuevo().con_consumo_personalizado(unidades_reales);
    let servicio = ServicioDeEmbeddings::nuevo(proveedor, Arc::clone(&repo));

    let peticion = PeticionDeEmbeddings {
        textos: vec!["fragmento de prueba".to_string()],
    };

    let respuesta = servicio
        .incrustar_lote(peticion, SystemTime::UNIX_EPOCH)
        .await
        .expect("debe ser exitoso");

    assert_eq!(respuesta.unidades_consumidas, unidades_reales);
    assert_eq!(respuesta.vectores.len(), 1);

    let saldo = repo.saldo().expect("consultar saldo");
    assert_eq!(saldo.reservado, 0, "no debe quedar saldo reservado activo");
    assert_eq!(
        saldo.disponible,
        (saldo_inicial as i64) - (unidades_reales as i64),
        "el saldo disponible debe haberse debitado por el consumo real"
    );
}

#[tokio::test]
async fn llamada_sin_metadatos_de_uso_concilia_contra_estimacion_previa() {
    let dir = DirectorioTemporal::nuevo("emb-sin-uso");
    let saldo_inicial = 1000;
    let repo = preparar_repositorio(&dir, saldo_inicial);

    // Proveedor que reporta 0 unidades consumidas (emulando respuesta sin metadatos de uso)
    let proveedor = ProveedorDeEmbeddingsSimulado::nuevo().con_consumo_personalizado(0);
    let servicio = ServicioDeEmbeddings::nuevo(proveedor, Arc::clone(&repo));

    let textos = vec!["texto largo para verificar estimación previa".to_string()];
    let estimacion = estimar_coste_de_lote(&textos);
    assert!(estimacion > 0);

    let peticion = PeticionDeEmbeddings { textos };

    let respuesta = servicio
        .incrustar_lote(peticion, SystemTime::UNIX_EPOCH)
        .await
        .expect("debe ser exitoso");

    assert_eq!(respuesta.unidades_consumidas, 0);

    let saldo = repo.saldo().expect("consultar saldo");
    assert_eq!(saldo.reservado, 0, "no debe quedar saldo reservado activo");
    assert_eq!(
        saldo.disponible,
        (saldo_inicial as i64) - (estimacion as i64),
        "ante ausencia de uso, debe conciliar contra la estimación previa (piso financiero)"
    );
}

#[tokio::test]
async fn fallo_del_proveedor_libera_la_reserva_por_completo() {
    let dir = DirectorioTemporal::nuevo("emb-fallo");
    let saldo_inicial = 1000;
    let repo = preparar_repositorio(&dir, saldo_inicial);

    let proveedor = ProveedorDeEmbeddingsSimulado::que_falla();
    let servicio = ServicioDeEmbeddings::nuevo(proveedor, Arc::clone(&repo));

    let peticion = PeticionDeEmbeddings {
        textos: vec!["texto que fallará".to_string()],
    };

    let error = servicio
        .incrustar_lote(peticion, SystemTime::UNIX_EPOCH)
        .await
        .expect_err("debe fallar por avería del proveedor");

    assert!(matches!(error, ErrorDeServicioDeEmbeddings::Proveedor(_)));

    let saldo = repo.saldo().expect("consultar saldo");
    assert_eq!(saldo.reservado, 0, "no debe quedar saldo reservado activo");
    assert_eq!(
        saldo.disponible, saldo_inicial as i64,
        "el saldo disponible debe permanecer íntegro tras liberar la reserva"
    );
}

#[tokio::test]
async fn reanudacion_de_lote_con_reserva_y_conciliacion_exacta() {
    let dir = DirectorioTemporal::nuevo("emb-resuncion");
    let saldo_inicial = 1000;
    let repo = preparar_repositorio(&dir, saldo_inicial);

    let textos = vec!["fragmento alfa".to_string(), "fragmento beta".to_string()];
    let mut lote = LoteDeEmbeddings::nuevo(textos);

    // Primer intento: el proveedor solo resuelve el primer fragmento (límite 1)
    let proveedor_parcial = ProveedorDeEmbeddingsSimulado::nuevo()
        .con_limite_elementos(1)
        .con_consumo_personalizado(10);
    let servicio_1 = ServicioDeEmbeddings::nuevo(proveedor_parcial, Arc::clone(&repo));

    let (peticion_1, indices_1) = lote.peticion_pendiente().expect("fragmentos pendientes");
    assert_eq!(indices_1, vec![0, 1]);

    let respuesta_1 = servicio_1
        .incrustar_lote(peticion_1, SystemTime::UNIX_EPOCH)
        .await
        .expect("primera llamada");
    lote.integrar(&indices_1, respuesta_1)
        .expect("integrar primera respuesta");

    assert_eq!(lote.pendientes(), 1);
    assert!(!lote.esta_completo());

    let saldo_intermedio = repo.saldo().expect("consultar saldo intermedio");
    assert_eq!(saldo_intermedio.reservado, 0);
    assert_eq!(saldo_intermedio.disponible, 990);

    // Segundo intento: se solicita solo el fragmento pendiente (índice 1)
    let proveedor_completo = ProveedorDeEmbeddingsSimulado::nuevo().con_consumo_personalizado(5);
    let servicio_2 = ServicioDeEmbeddings::nuevo(proveedor_completo, Arc::clone(&repo));

    let (peticion_2, indices_2) = lote
        .peticion_pendiente()
        .expect("fragmento pendiente restante");
    assert_eq!(indices_2, vec![1]);
    assert_eq!(peticion_2.textos, vec!["fragmento beta".to_string()]);

    let respuesta_2 = servicio_2
        .incrustar_lote(peticion_2, SystemTime::UNIX_EPOCH)
        .await
        .expect("segunda llamada");
    lote.integrar(&indices_2, respuesta_2)
        .expect("integrar segunda respuesta");

    assert_eq!(lote.pendientes(), 0);
    assert!(lote.esta_completo());

    let saldo_final = repo.saldo().expect("consultar saldo final");
    assert_eq!(saldo_final.reservado, 0);
    assert_eq!(
        saldo_final.disponible, 985,
        "el gasto total (10 + 5 = 15) debe reflejar exactamente la suma de ambas llamadas"
    );

    let vectores = lote.completo().expect("vectores finales");
    assert_eq!(vectores.len(), 2);
}
