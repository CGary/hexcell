//! Tests de idempotencia de entrega: AC-7 (a nivel de motor) y AC-8/AC-9 (a nivel de registro).
//!
//! Los escenarios de AC-8 y AC-9 se escriben contra `RegistroDeDeduplicacion` directamente, con
//! marcas temporales explícitas suministradas por el propio test: el registro no consulta ninguna
//! hora del sistema, así que estos tests tampoco la necesitan, no duermen nunca y su veredicto es
//! idéntico en cada ejecución. Desde HEX-006 el registro se respalda en una `sessions.db` real, y
//! por eso cada test abre la suya en un directorio temporal propio.

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::apagado::SenalDeApagado;
use hexcell::deduplicacion::{RegistroDeDeduplicacion, VeredictoDeDeduplicacion};
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeEco;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, Reloj, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

#[test]
fn un_duplicado_dentro_de_la_ventana_se_descarta_incluso_justo_en_su_borde() {
    let directorio = DirectorioTemporal::nuevo("dedup-borde");
    let ventana = Duration::from_secs(3600);
    let mut registro =
        RegistroDeDeduplicacion::nuevo(repositorio_temporal(directorio.ruta()), ventana);
    let id = IdDeduplicacion::nuevo("id-repetido");
    let primera_llegada = SystemTime::UNIX_EPOCH;

    assert_eq!(
        registro
            .procesar(id.clone(), primera_llegada)
            .expect("la primera llegada debe registrarse"),
        VeredictoDeDeduplicacion::Nuevo
    );

    // Un segundo justo antes de que la ventana de una hora expire: sigue siendo un duplicado.
    let justo_antes_del_borde = primera_llegada + ventana - Duration::from_secs(1);
    assert_eq!(
        registro
            .procesar(id, justo_antes_del_borde)
            .expect("la segunda llegada debe consultarse"),
        VeredictoDeDeduplicacion::Duplicado
    );
}

#[test]
fn una_reentrega_mas_alla_de_la_ventana_se_procesa_de_nuevo_como_evento_nuevo() {
    let directorio = DirectorioTemporal::nuevo("dedup-tardio");
    let ventana = Duration::from_secs(3600);
    let mut registro =
        RegistroDeDeduplicacion::nuevo(repositorio_temporal(directorio.ruta()), ventana);
    let id = IdDeduplicacion::nuevo("id-tardio");
    let primera_llegada = SystemTime::UNIX_EPOCH;

    assert_eq!(
        registro
            .procesar(id.clone(), primera_llegada)
            .expect("la primera llegada debe registrarse"),
        VeredictoDeDeduplicacion::Nuevo
    );

    // Un evento no relacionado, más allá de la ventana, adelanta el horizonte monótono del
    // registro y poda la entrada original.
    let mas_alla_de_la_ventana = primera_llegada + ventana + Duration::from_secs(1);
    registro
        .procesar(
            IdDeduplicacion::nuevo("evento-que-adelanta-el-horizonte"),
            mas_alla_de_la_ventana,
        )
        .expect("el evento que adelanta el horizonte debe registrarse");

    // AC-9: la reentrega tardía del mismo identificador ya fue podada, así que se procesa como
    // un evento nuevo, deliberadamente, sin panic y sin un tercer camino no especificado.
    assert_eq!(
        registro
            .procesar(id, mas_alla_de_la_ventana)
            .expect("la reentrega tardía debe consultarse"),
        VeredictoDeDeduplicacion::Nuevo
    );
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
async fn el_motor_procesa_un_evento_duplicado_una_sola_vez_dentro_de_la_ventana() {
    let directorio = DirectorioTemporal::nuevo("dedup-motor");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-duplicado-motor");
    let deduplicacion = IdDeduplicacion::nuevo("dedup-repetido-motor");

    let construir_evento = |contenido: &str| EventoEntrante {
        remitente: IdRemitente::nuevo("remitente-de-prueba"),
        conversacion: conversacion.clone(),
        contenido: contenido.to_string(),
        marca_temporal: reloj.ahora(),
        deduplicacion: deduplicacion.clone(),
    };

    adaptador
        .inyectar(construir_evento("hola"))
        .await
        .expect("el canal recién creado debe aceptar el evento");
    adaptador
        .inyectar(construir_evento("hola"))
        .await
        .expect("el canal recién creado debe aceptar el evento");

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
    tokio::time::sleep(Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;

    let capturas = adaptador.envios_capturados();
    assert_eq!(
        capturas.len(),
        1,
        "el segundo evento, idéntico y dentro de la ventana, no debe generar una segunda \
         respuesta ni un segundo despacho al procesador"
    );
}
