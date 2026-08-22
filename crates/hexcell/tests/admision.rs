//! Tests de integración para el control de admisión GCRA en el motor de mensajería (AC-1, AC-2, AC-3, FR-08).

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::apagado::SenalDeApagado;
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeEco;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, Reloj, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

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

fn evento_de_prueba(
    conversacion: &IdConversacion,
    id_dedup: &str,
    marca_temporal: SystemTime,
) -> EventoEntrante {
    EventoEntrante {
        remitente: IdRemitente::nuevo("remitente-admision"),
        conversacion: conversacion.clone(),
        contenido: "mensaje de prueba admision".to_string(),
        marca_temporal,
        deduplicacion: IdDeduplicacion::nuevo(id_dedup),
    }
}

async fn drenar_y_cancelar(motor: Motor<AdaptadorQueDelegaEnArc, ProcesadorDeEco>) {
    let mut motor = motor;
    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;
}

#[tokio::test]
async fn ac_1_un_evento_dentro_del_presupuesto_es_admitido_y_procesado_sin_cambios() {
    let directorio = DirectorioTemporal::nuevo("admision-ac1");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-admision-ac1");

    adaptador
        .inyectar(evento_de_prueba(
            &conversacion,
            "dedup-ac1-1",
            reloj.ahora(),
        ))
        .await
        .expect("el canal debe aceptar el evento");

    let repositorio = repositorio_temporal(directorio.ruta());
    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        Arc::clone(&repositorio),
    );
    drenar_y_cancelar(motor).await;

    let capturas = adaptador.envios_capturados();
    assert_eq!(capturas.len(), 1, "el evento admitido debe enviarse");
    assert_eq!(capturas[0].0, conversacion);

    let motor_consulta = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        AdaptadorSimulado::nuevo(Arc::new(reloj), 8).1,
        Duration::from_secs(3600),
        repositorio,
    );
    let historial = motor_consulta
        .historial(&conversacion)
        .expect("historial consultable");
    assert_eq!(
        historial.len(),
        2,
        "el historial debe registrar entrante y saliente"
    );
}

#[tokio::test]
async fn ac_2_y_ac_3_evento_en_exceso_es_descartado_antes_de_cargar_contexto() {
    // ConfiguracionGcra::default(): tasa 0.5 req/s, ráfaga 3.
    // Permite exactamente N+1 = 4 peticiones consecutivas sin avanzar el reloj.
    // Injectamos 5 eventos con distintos deduplicacion IDs para la misma conversación.
    let directorio = DirectorioTemporal::nuevo("admision-ac2-ac3");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 16);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-admision-ac2");

    for i in 1..=5 {
        adaptador
            .inyectar(evento_de_prueba(
                &conversacion,
                &format!("dedup-ac2-{i}"),
                reloj.ahora(),
            ))
            .await
            .expect("el canal debe aceptar el evento");
    }

    let repositorio = repositorio_temporal(directorio.ruta());
    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        Arc::clone(&repositorio),
    );
    drenar_y_cancelar(motor).await;

    let capturas = adaptador.envios_capturados();
    assert_eq!(
        capturas.len(),
        4,
        "deben enviarse exactamente 4 respuestas, el 5º evento debe ser descartado por GCRA"
    );

    let motor_consulta = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        AdaptadorSimulado::nuevo(Arc::new(reloj), 8).1,
        Duration::from_secs(3600),
        repositorio,
    );
    let historial = motor_consulta
        .historial(&conversacion)
        .expect("historial consultable");

    // 4 eventos admitidos -> 4 entrantes + 4 salientes = 8 entradas en historial.
    // El 5º evento descartado no deja NINGUNA traza en historial (demuestra descarte antes de registrar_entrante / context loading).
    assert_eq!(
        historial.len(),
        8,
        "el 5º evento descartado no debe dejar rastro en el historial conversacional"
    );
}
