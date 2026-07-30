//! Tests del motor de mensajería sobre el adaptador simulado, concreto a `AdaptadorSimulado`.
//!
//! Una batería genérica sobre cualquier `ChannelAdapter` es la etapa siguiente (HEX-005); estos
//! tests nombran el tipo concreto a propósito, sin adelantar ese alcance.
//!
//! `Motor::ejecutar` es un bucle que solo termina cuando el canal de eventos se cierra, y en
//! producción eso no ocurre nunca mientras la célula vive. Estos tests lo corren en una tarea de
//! fondo (`tokio::spawn`), dejan que procese los eventos ya inyectados y la cancelan
//! (`JoinHandle::abort`) en vez de esperar un cierre de canal que estos tests no necesitan
//! provocar; lo que importa comprobar es lo que el adaptador capturó mientras tanto.

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeEco;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, Reloj, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

/// El identificador de deduplicación se deriva de la propia conversación para que, en los tests
/// de este archivo que inyectan varias conversaciones a la vez, cada evento tenga un
/// identificador distinto: el motor ahora deduplica de verdad (`crate::deduplicacion`), y un
/// identificador repetido entre conversaciones distintas se trataría como el mismo evento.
fn evento_de_prueba(conversacion: &IdConversacion, marca_temporal: SystemTime) -> EventoEntrante {
    EventoEntrante {
        remitente: IdRemitente::nuevo("remitente-de-prueba"),
        conversacion: conversacion.clone(),
        contenido: "eco de prueba".to_string(),
        marca_temporal,
        deduplicacion: IdDeduplicacion::nuevo(format!("dedup-{}", conversacion.como_str())),
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

/// Deja correr el motor lo suficiente para drenar lo que ya está encolado y lo cancela.
async fn drenar_y_cancelar(motor: Motor<AdaptadorQueDelegaEnArc, ProcesadorDeEco>) {
    let mut motor = motor;
    let manejador = tokio::spawn(async move {
        motor.ejecutar().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;
}

#[tokio::test]
async fn un_evento_inyectado_se_procesa_y_la_respuesta_se_envia_por_send() {
    let directorio = DirectorioTemporal::nuevo("motor-eco");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-motor-eco");

    adaptador
        .inyectar(evento_de_prueba(&conversacion, reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");

    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        repositorio_temporal(directorio.ruta()),
    );
    drenar_y_cancelar(motor).await;

    let capturas = adaptador.envios_capturados();
    assert_eq!(capturas.len(), 1);
    assert_eq!(capturas[0].0, conversacion);
    assert_eq!(
        capturas[0].1,
        MensajeSaliente::RespuestaLibre("eco de prueba".to_string())
    );
    assert_eq!(capturas[0].2, ResultadoEnvio::Aceptado);
}

#[tokio::test]
async fn el_motor_no_se_detiene_ante_cada_variante_de_resultado_ni_ante_una_averia() {
    let directorio = DirectorioTemporal::nuevo("motor-variantes");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);

    let conversaciones: Vec<IdConversacion> = (0..6)
        .map(|indice| IdConversacion::nuevo(format!("conversacion-variante-{indice}")))
        .collect();

    for conversacion in &conversaciones {
        adaptador
            .inyectar(evento_de_prueba(conversacion, reloj.ahora()))
            .await
            .expect("el canal recién creado debe aceptar el evento");
    }

    // `forzar_averia` no se ata a una conversación: se consume en la próxima llamada a `send`
    // que ocurra, sea cual sea. El motor procesa el canal en orden de llegada (FIFO), así que esa
    // próxima llamada es la de conversaciones[0], la primera en entrar por `inyectar`.
    adaptador.forzar_averia();
    adaptador.forzar(&conversaciones[1], ResultadoEnvio::FueraDeVentana);
    adaptador.forzar(&conversaciones[2], ResultadoEnvio::PlantillaRequerida);
    adaptador.forzar(&conversaciones[3], ResultadoEnvio::LimiteDeTasa);
    adaptador.forzar(&conversaciones[4], ResultadoEnvio::DestinatarioInvalido);
    // conversaciones[5] queda como Aceptado natural: ventana abierta, sin nada forzado.

    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        repositorio_temporal(directorio.ruta()),
    );
    // Si el motor entrara en pánico o dejara de consumir ante cualquiera de las seis variantes,
    // la tarea de fondo habría terminado sola en vez de necesitar `abort()`; en cualquier caso lo
    // que se comprueba es la lista de capturas de más abajo, no cómo terminó la tarea.
    drenar_y_cancelar(motor).await;

    let capturas = adaptador.envios_capturados();
    // conversaciones[0] no deja captura: la avería forzada hace que `send` devuelva `Err` antes
    // de registrar nada, y el motor la trata sin `unwrap()` y sigue consumiendo el resto.
    assert_eq!(capturas.len(), 5);
    assert_eq!(capturas[0].2, ResultadoEnvio::FueraDeVentana);
    assert_eq!(capturas[1].2, ResultadoEnvio::PlantillaRequerida);
    assert_eq!(capturas[2].2, ResultadoEnvio::LimiteDeTasa);
    assert_eq!(capturas[3].2, ResultadoEnvio::DestinatarioInvalido);
    assert_eq!(capturas[4].2, ResultadoEnvio::Aceptado);
}

#[test]
fn el_archivo_del_motor_no_asume_aceptado_y_trata_las_cinco_variantes() {
    let contenido = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/motor.rs"))
        .expect("el archivo del motor debe existir para esta comprobación léxica");
    assert!(contenido.contains("ResultadoEnvio::Aceptado"));
    assert!(contenido.contains("ResultadoEnvio::FueraDeVentana"));
    assert!(contenido.contains("ResultadoEnvio::PlantillaRequerida"));
    assert!(contenido.contains("ResultadoEnvio::LimiteDeTasa"));
    assert!(contenido.contains("ResultadoEnvio::DestinatarioInvalido"));
    assert!(contenido.contains("Err("));
    assert!(!contenido.contains(".unwrap("));
    assert!(!contenido.contains(".expect("));
}
