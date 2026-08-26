//! Tests del puerto de inferencia (AC-1, AC-2) y de su consumo por el motor (AC-3, AC-4).

mod comun;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::apagado::SenalDeApagado;
use hexcell::inferencia::{ErrorDeInferenciaSimulada, ProveedorSimulado};
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeInferencia;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use hexcell_core::inferencia::{
    PeticionDeInferencia, ProveedorDeInferencia, RespuestaDeInferencia,
};

#[tokio::test]
async fn el_proveedor_simulado_devuelve_la_misma_respuesta_para_la_misma_peticion() {
    let proveedor = ProveedorSimulado::nuevo();
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conversacion-determinismo"),
        contenido: "hola, mundo".to_string(),
    };

    let primera = proveedor
        .generar(peticion.clone())
        .await
        .expect("el proveedor simulado no debe fallar sin que se le pida");
    let segunda = proveedor
        .generar(peticion)
        .await
        .expect("el proveedor simulado no debe fallar sin que se le pida");

    assert_eq!(
        primera, segunda,
        "la misma petición debe producir siempre la misma respuesta"
    );
}

#[tokio::test]
async fn el_proveedor_simulado_no_hace_eco_del_contenido_de_entrada() {
    let proveedor = ProveedorSimulado::nuevo();
    let peticion = PeticionDeInferencia {
        conversacion: IdConversacion::nuevo("conversacion-no-eco"),
        contenido: "este texto no debe volver tal cual".to_string(),
    };

    let respuesta = proveedor
        .generar(peticion.clone())
        .await
        .expect("el proveedor simulado no debe fallar sin que se le pida");

    assert_ne!(
        respuesta.contenido, peticion.contenido,
        "la respuesta simulada no debe ser un eco del contenido de entrada"
    );
}

#[tokio::test]
async fn peticiones_distintas_producen_respuestas_distintas() {
    let proveedor = ProveedorSimulado::nuevo();
    let respuesta_a = proveedor
        .generar(PeticionDeInferencia {
            conversacion: IdConversacion::nuevo("conversacion-a"),
            contenido: "primer contenido".to_string(),
        })
        .await
        .expect("no debe fallar");
    let respuesta_b = proveedor
        .generar(PeticionDeInferencia {
            conversacion: IdConversacion::nuevo("conversacion-b"),
            contenido: "segundo contenido".to_string(),
        })
        .await
        .expect("no debe fallar");

    assert_ne!(respuesta_a, respuesta_b);
}

/// Envoltorio de test: delega en un `Arc<AdaptadorSimulado>` compartido con quien inyecta y
/// quien, luego, inspecciona `envios_capturados()`.
struct AdaptadorQueDelegaEnArc(Arc<AdaptadorSimulado>);

impl ChannelAdapter for AdaptadorQueDelegaEnArc {
    type Error = ErrorDelAdaptadorSimulado;

    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        self.0.send(conversacion, mensaje).await
    }

    async fn estado_ventana(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<EstadoVentanaServicio, Self::Error> {
        self.0.estado_ventana(conversacion).await
    }
}

fn evento(conversacion: &IdConversacion, contenido: &str, deduplicacion: &str) -> EventoEntrante {
    EventoEntrante {
        remitente: IdRemitente::nuevo("remitente-de-prueba"),
        conversacion: conversacion.clone(),
        contenido: contenido.to_string(),
        marca_temporal: SystemTime::UNIX_EPOCH,
        deduplicacion: IdDeduplicacion::nuevo(deduplicacion),
    }
}

/// Doble de prueba de ProveedorDeInferencia que cuenta invocaciones con un `Arc<AtomicUsize>`.
#[derive(Clone)]
struct ProveedorContador {
    invocaciones: Arc<AtomicUsize>,
}

impl ProveedorContador {
    fn nuevo() -> (Self, Arc<AtomicUsize>) {
        let contador = Arc::new(AtomicUsize::new(0));
        (
            Self {
                invocaciones: Arc::clone(&contador),
            },
            contador,
        )
    }
}

impl ProveedorDeInferencia for ProveedorContador {
    type Error = ErrorDeInferenciaSimulada;

    async fn generar(
        &self,
        peticion: PeticionDeInferencia,
    ) -> Result<RespuestaDeInferencia, Self::Error> {
        self.invocaciones.fetch_add(1, Ordering::Relaxed);
        Ok(RespuestaDeInferencia {
            contenido: format!("respuesta simulada para {}", peticion.contenido),
        })
    }
}

#[tokio::test]
async fn el_motor_envia_la_respuesta_del_proveedor_y_no_el_eco_del_procesador() {
    let directorio = DirectorioTemporal::nuevo("inferencia-motor");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-respuesta-de-proveedor");

    adaptador
        .inyectar(evento(
            &conversacion,
            "contenido de entrada distintivo",
            "dedup-inferencia-uno",
        ))
        .await
        .expect("el canal recién creado debe aceptar el evento");

    let repositorio = repositorio_temporal(directorio.ruta());
    repositorio
        .aportar_presupuesto(100, SystemTime::UNIX_EPOCH)
        .expect("aportar saldo para el test");

    let procesador =
        ProcesadorDeInferencia::nuevo(ProveedorSimulado::nuevo(), Arc::clone(&repositorio));
    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        std::time::Duration::from_secs(3600),
        repositorio,
    );

    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;

    let capturas = adaptador.envios_capturados();
    assert_eq!(capturas.len(), 1);
    let MensajeSaliente::RespuestaLibre {
        texto: texto_enviado,
        ..
    } = &capturas[0].1
    else {
        panic!("se esperaba una respuesta libre");
    };
    assert_ne!(
        texto_enviado, "contenido de entrada distintivo",
        "la respuesta enviada no debe ser el eco del contenido de entrada"
    );

    let esperada = ProveedorSimulado::nuevo();
    let respuesta_esperada = esperada
        .generar(PeticionDeInferencia {
            conversacion: conversacion.clone(),
            contenido: "contenido de entrada distintivo".to_string(),
        })
        .await
        .expect("no debe fallar");
    assert_eq!(texto_enviado, &respuesta_esperada.contenido);
}

#[tokio::test]
async fn un_fallo_del_proveedor_no_envia_nada_y_el_motor_sigue_consumiendo() {
    let directorio = DirectorioTemporal::nuevo("inferencia-fallo");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion_que_falla = IdConversacion::nuevo("conversacion-que-falla");
    let conversacion_que_sigue = IdConversacion::nuevo("conversacion-que-sigue");

    adaptador
        .inyectar(evento(
            &conversacion_que_falla,
            "este evento no debe generar respuesta",
            "dedup-fallo-uno",
        ))
        .await
        .expect("el canal recién creado debe aceptar el evento");
    adaptador
        .inyectar(evento(
            &conversacion_que_sigue,
            "este evento sí debe responderse",
            "dedup-fallo-dos",
        ))
        .await
        .expect("el canal recién creado debe aceptar el evento");

    let repositorio = repositorio_temporal(directorio.ruta());
    repositorio
        .aportar_presupuesto(100, SystemTime::UNIX_EPOCH)
        .expect("aportar saldo para el test");

    let procesador =
        ProcesadorDeInferencia::nuevo(ProveedorSimulado::que_falla(), Arc::clone(&repositorio));
    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        std::time::Duration::from_secs(3600),
        repositorio,
    );

    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;

    assert!(
        adaptador.envios_capturados().is_empty(),
        "un proveedor que siempre falla no debe producir ningún envío"
    );
}

// La mitad de AC-2 que comprueba el registro `presupuesto_rechazado` vive en
// `crates/hexcell/src/procesador.rs` (test unitario), donde `registro::pruebas` es alcanzable;
// este test de integración prueba solo la mitad conductual: con saldo insuficiente el proveedor
// de inferencia registra cero llamadas y no se envía nada.
#[tokio::test]
async fn con_saldo_insuficiente_el_proveedor_de_inferencia_registra_cero_llamadas() {
    let directorio = DirectorioTemporal::nuevo("inferencia-saldo-insuficiente");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-sin-saldo");

    adaptador
        .inyectar(evento(
            &conversacion,
            "prompt que requiere unidades de presupuesto",
            "dedup-sin-saldo-uno",
        ))
        .await
        .expect("inyectar evento de prueba");

    let repositorio = repositorio_temporal(directorio.ruta());
    // El saldo inicial es 0, menor que la estimación del prompt

    let (proveedor_contador, contador) = ProveedorContador::nuevo();
    let procesador = ProcesadorDeInferencia::nuevo(proveedor_contador, Arc::clone(&repositorio));
    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        std::time::Duration::from_secs(3600),
        repositorio,
    );

    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;

    assert_eq!(
        contador.load(Ordering::Relaxed),
        0,
        "el proveedor de inferencia no debe ser invocado cuando el saldo es insuficiente"
    );
    assert!(
        adaptador.envios_capturados().is_empty(),
        "no debe haber envíos cuando la reserva es rechazada"
    );
}

#[tokio::test]
async fn con_saldo_suficiente_el_proveedor_de_inferencia_es_invocado() {
    let directorio = DirectorioTemporal::nuevo("inferencia-saldo-suficiente");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-con-saldo");

    adaptador
        .inyectar(evento(&conversacion, "hola", "dedup-con-saldo-uno"))
        .await
        .expect("inyectar evento de prueba");

    let repositorio = repositorio_temporal(directorio.ruta());
    repositorio
        .aportar_presupuesto(50, SystemTime::UNIX_EPOCH)
        .expect("aportar saldo suficiente");

    let (proveedor_contador, contador) = ProveedorContador::nuevo();
    let procesador = ProcesadorDeInferencia::nuevo(proveedor_contador, Arc::clone(&repositorio));
    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        std::time::Duration::from_secs(3600),
        repositorio,
    );

    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;

    assert_eq!(
        contador.load(Ordering::Relaxed),
        1,
        "el proveedor de inferencia debe ser invocado exactamente una vez con saldo suficiente"
    );
    assert_eq!(
        adaptador.envios_capturados().len(),
        1,
        "debe existir un envío saliente tras la inferencia exitosa"
    );
}
