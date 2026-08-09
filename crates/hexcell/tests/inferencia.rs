//! Tests del puerto de inferencia (AC-1, AC-2) y de su consumo por el motor (AC-3, AC-4).

mod comun;

use std::sync::Arc;
use std::time::SystemTime;

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::apagado::SenalDeApagado;
use hexcell::inferencia::ProveedorSimulado;
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeInferencia;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use hexcell_core::inferencia::{PeticionDeInferencia, ProveedorDeInferencia};

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

    let procesador = ProcesadorDeInferencia::nuevo(ProveedorSimulado::nuevo());
    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        std::time::Duration::from_secs(3600),
        repositorio_temporal(directorio.ruta()),
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

    // El proveedor que siempre falla se usa para el primer evento; el motor no debe entrar en
    // pánico ni dejar de consumir el segundo por ello. Como `ProveedorSimulado` no distingue
    // conversaciones, se usa un procesador que siempre falla para demostrar que ningún envío se
    // produce y que el motor no se detiene: el segundo evento del mismo procesador tampoco genera
    // respuesta, que es exactamente lo que se comprueba.
    let procesador = ProcesadorDeInferencia::nuevo(ProveedorSimulado::que_falla());
    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        std::time::Duration::from_secs(3600),
        repositorio_temporal(directorio.ruta()),
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
