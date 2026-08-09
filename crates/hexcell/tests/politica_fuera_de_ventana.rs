//! Tests de la política del motor ante `FueraDeVentana` (AC-3, AC-4), a nivel de motor completo.
//!
//! Este test vive aquí y no en la batería reutilizable de `hexcell-canal-contrato` a propósito:
//! la batería describe lo que **cualquier puerto** debe hacer y tiene que seguir siendo usable
//! por cualquier adaptador futuro (incluida la Cloud API real de la etapa B-1), mientras que este
//! archivo describe lo que **el núcleo** hace en respuesta — una política interna del motor que
//! solo tiene sentido preguntada contra `Motor`, no contra un adaptador aislado. Meterla en la
//! batería obligaría a la etapa B-1 a depender del binario de la célula para ejercitar el
//! contrato del puerto, exactamente la dependencia que la batería existe para evitar.

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

fn evento(
    conversacion: &IdConversacion,
    contenido: &str,
    deduplicacion: &str,
    marca_temporal: SystemTime,
) -> EventoEntrante {
    EventoEntrante {
        remitente: IdRemitente::nuevo("remitente-de-prueba"),
        conversacion: conversacion.clone(),
        contenido: contenido.to_string(),
        marca_temporal,
        deduplicacion: IdDeduplicacion::nuevo(deduplicacion),
    }
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

#[tokio::test]
async fn una_respuesta_fuera_de_ventana_se_difiere_y_se_reenvia_antes_que_la_nueva() {
    let directorio = DirectorioTemporal::nuevo("fuera-de-ventana");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-diferida");

    adaptador
        .inyectar(evento(&conversacion, "hola", "dedup-uno", reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");

    // Fuerza que el eco de este primer evento choque con la ventana cerrada.
    adaptador.forzar(&conversacion, ResultadoEnvio::FueraDeVentana);

    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        repositorio_temporal(directorio.ruta()),
    );
    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });

    // Deja que el motor procese el primer evento: la política ante `FueraDeVentana` debe diferir
    // la respuesta en vez de descartarla o tratarla como un error genérico.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let capturas_tras_diferir = adaptador.envios_capturados();
    assert_eq!(
        capturas_tras_diferir.len(),
        1,
        "la respuesta al primer evento se intenta enviar una vez, y esa llamada es la forzada"
    );
    assert_eq!(capturas_tras_diferir[0].2, ResultadoEnvio::FueraDeVentana);

    // El cliente vuelve a escribir: esto es exactamente lo que reabre la ventana en el adaptador
    // simulado y lo que debe disparar el reenvío de la respuesta diferida.
    adaptador
        .inyectar(evento(
            &conversacion,
            "hola de nuevo",
            "dedup-dos",
            reloj.ahora(),
        ))
        .await
        .expect("el canal debe seguir aceptando eventos");
    tokio::time::sleep(Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;

    let capturas = adaptador.envios_capturados();
    assert_eq!(
        capturas.len(),
        3,
        "se esperan tres llamadas a send: el intento diferido, su reintento y la respuesta nueva"
    );
    assert_eq!(capturas[0].2, ResultadoEnvio::FueraDeVentana);
    let MensajeSaliente::RespuestaLibre { texto, .. } = &capturas[1].1 else {
        panic!("el reintento debe ser una respuesta libre");
    };
    assert_eq!(
        texto, "hola",
        "el reintento debe ser la respuesta diferida del primer evento, no la del segundo"
    );
    assert_eq!(capturas[1].2, ResultadoEnvio::Aceptado);
    let MensajeSaliente::RespuestaLibre { texto, .. } = &capturas[2].1 else {
        panic!("el tercer envío debe ser una respuesta libre");
    };
    assert_eq!(texto, "hola de nuevo");
    assert_eq!(capturas[2].2, ResultadoEnvio::Aceptado);
}
