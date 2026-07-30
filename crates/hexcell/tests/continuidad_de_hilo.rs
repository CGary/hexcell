//! Test de continuidad de hilo (AC-5, AC-6) a nivel de motor completo.
//!
//! Inyecta desde un contacto simulado, deja que el motor construya historial, re-empareja el
//! adaptador con un dispositivo nuevo e inyecta de nuevo desde el mismo contacto: el motor debe
//! asignar el segundo evento al mismo `IdConversacion` que el primero, con el historial anterior
//! intacto y continuado en el mismo hilo (`crate::conversaciones::EstadoDeConversaciones`).
//!
//! No se usa `tokio::spawn` + `abort()` aquí como en otros tests de este directorio: este test
//! necesita inspeccionar `Motor::historial` entre una tanda de eventos y la siguiente, y mover el
//! motor a una tarea de fondo impediría recuperarlo. En su lugar, `motor.ejecutar()` se corre
//! dentro de un `tokio::select!` contra un `sleep` corto: el préstamo `&mut motor` termina cuando
//! el `select!` termina, y el `motor`, que nunca se movió, sigue disponible para inspeccionarlo.

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeEco;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, RelojDePrueba};
use hexcell_core::canal::{ChannelAdapter, EstadoVentanaServicio, MensajeSaliente, ResultadoEnvio};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion};

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

/// Deja avanzar `motor.ejecutar()` un tiempo corto sin moverlo de sitio, para poder seguir
/// inspeccionándolo después.
async fn dejar_procesar<A, P>(motor: &mut Motor<A, P>)
where
    A: ChannelAdapter,
    P: hexcell::procesador::ProcesadorDeMensajes,
{
    tokio::select! {
        () = motor.ejecutar() => {}
        () = tokio::time::sleep(Duration::from_millis(50)) => {}
    }
}

#[tokio::test]
async fn el_hilo_sobrevive_a_un_re_emparejamiento_con_su_historial_intacto() {
    let directorio = DirectorioTemporal::nuevo("continuidad-de-hilo");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj), 8);
    let adaptador = Arc::new(adaptador);

    let conversacion_antes = adaptador
        .inyectar_desde_contacto(
            "contacto-hilo",
            "hola",
            IdDeduplicacion::nuevo("dedup-hilo-1"),
        )
        .await
        .expect("el canal recién creado debe aceptar el evento");

    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        repositorio_temporal(directorio.ruta()),
    );
    dejar_procesar(&mut motor).await;

    let historial_antes = motor
        .historial(&conversacion_antes)
        .expect("leer el historial persistido");
    assert!(
        !historial_antes.is_empty(),
        "el primer evento debe haber dejado algo en el historial"
    );

    // Re-emparejamiento: cambia el dispositivo vinculado, el mapa de contactos queda intacto.
    adaptador.re_emparejar("dispositivo-nuevo");

    let conversacion_despues = adaptador
        .inyectar_desde_contacto(
            "contacto-hilo",
            "hola de nuevo",
            IdDeduplicacion::nuevo("dedup-hilo-2"),
        )
        .await
        .expect("el canal debe seguir aceptando eventos tras el re-emparejamiento");

    assert_eq!(
        conversacion_antes, conversacion_despues,
        "el mismo contacto debe seguir resolviendo a la misma conversación tras re-emparejar"
    );

    dejar_procesar(&mut motor).await;

    let historial_despues = motor
        .historial(&conversacion_despues)
        .expect("leer el historial persistido tras el re-emparejamiento");
    assert!(
        historial_despues.len() > historial_antes.len(),
        "el historial debe continuar, no reiniciarse, tras el re-emparejamiento"
    );
    assert_eq!(
        &historial_despues[..historial_antes.len()],
        historial_antes.as_slice(),
        "el historial previo al re-emparejamiento debe seguir intacto y en el mismo orden"
    );
}
