//! Tests de persistencia a nivel de motor completo: AC-6, AC-7 y AC-8.
//!
//! Los tests de reinicio son la razón de ser de esta tarea, y por eso **sueltan de verdad** el
//! primer motor y sus pools antes de abrir los segundos sobre el mismo archivo. Un test de
//! reinicio que mantuviera viva la primera conexión no demostraría nada sobre la persistencia y
//! pasaría por encima de una implementación rota.

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::{DirectorioTemporal, abrir_persistencia};
use hexcell::apagado::SenalDeApagado;
use hexcell::conversaciones::EventoDeHistorial;
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeEco;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, Reloj, RelojDePrueba};
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use hexcell_storage::SalienteHistorico;

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

/// Deja correr el motor lo suficiente para drenar lo que ya está encolado y lo cancela, soltando
/// el motor —y con él su referencia a los pools— antes de devolver el control.
async fn drenar_y_soltar(motor: Motor<AdaptadorQueDelegaEnArc, ProcesadorDeEco>) {
    let mut motor = motor;
    let manejador = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    manejador.abort();
    let _ = manejador.await;
}

#[tokio::test]
async fn un_evento_procesado_deja_filas_escritas_en_sessions_db() {
    let directorio = DirectorioTemporal::nuevo("persistencia-filas");
    let (pools, repositorio) = abrir_persistencia(directorio.ruta());

    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);
    let conversacion = IdConversacion::nuevo("conversacion-con-filas");

    adaptador
        .inyectar(evento(&conversacion, "hola", "dedup-filas", reloj.ahora()))
        .await
        .expect("el canal recién creado debe aceptar el evento");

    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        Arc::clone(&repositorio),
    );
    drenar_y_soltar(motor).await;

    let cuentas = pools
        .sesiones()
        .con_lectura(|conexion| {
            let contar = |consulta: &str| -> i64 {
                conexion
                    .query_row(consulta, [], |fila| fila.get(0))
                    .expect("contar filas de sessions.db")
            };
            Ok((
                contar("SELECT count(*) FROM contactos"),
                contar("SELECT count(*) FROM conversaciones"),
                contar("SELECT count(*) FROM mensajes"),
                contar("SELECT count(*) FROM deduplicacion"),
            ))
        })
        .expect("leer las cuentas de sessions.db");

    assert_eq!(cuentas.0, 1, "el contacto debe quedar registrado");
    assert_eq!(cuentas.1, 1, "la conversación debe quedar registrada");
    assert_eq!(
        cuentas.2, 2,
        "deben quedar dos mensajes: el entrante y el eco saliente"
    );
    assert_eq!(
        cuentas.3, 1,
        "el identificador de deduplicación debe quedar retenido"
    );
}

#[tokio::test]
async fn la_deduplicacion_sobrevive_a_un_reinicio_del_proceso() {
    let directorio = DirectorioTemporal::nuevo("persistencia-dedup-reinicio");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let conversacion = IdConversacion::nuevo("conversacion-dedup-reinicio");
    let identificador_repetido = "dedup-que-debe-sobrevivir";

    // Primer «proceso»: procesa el evento y responde.
    {
        let (pools, repositorio) = abrir_persistencia(directorio.ruta());
        let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
        let adaptador = Arc::new(adaptador);

        adaptador
            .inyectar(evento(
                &conversacion,
                "hola",
                identificador_repetido,
                reloj.ahora(),
            ))
            .await
            .expect("el canal recién creado debe aceptar el evento");

        let motor = Motor::nuevo(
            AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
            ProcesadorDeEco,
            receptor_eventos,
            Duration::from_secs(3600),
            repositorio,
        );
        drenar_y_soltar(motor).await;

        assert_eq!(
            adaptador.envios_capturados().len(),
            1,
            "el primer proceso debe responder una vez"
        );

        // Se sueltan los pools del primer proceso: sin esto, el test no probaría persistencia,
        // solo que dos motores comparten memoria.
        drop(pools);
    }

    // Segundo «proceso»: pools nuevos sobre el mismo archivo, mismo identificador.
    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
    let adaptador = Arc::new(adaptador);

    adaptador
        .inyectar(evento(
            &conversacion,
            "hola",
            identificador_repetido,
            reloj.ahora(),
        ))
        .await
        .expect("el canal debe aceptar el evento");

    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        repositorio,
    );
    drenar_y_soltar(motor).await;

    assert!(
        adaptador.envios_capturados().is_empty(),
        "tras el reinicio, la reentrega del mismo identificador debe descartarse sin responder"
    );
}

#[tokio::test]
async fn el_historial_sobrevive_a_un_reinicio_y_conserva_su_orden() {
    let directorio = DirectorioTemporal::nuevo("persistencia-historial-reinicio");
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let conversacion = IdConversacion::nuevo("conversacion-historial-reinicio");

    {
        let (pools, repositorio) = abrir_persistencia(directorio.ruta());
        let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
        let adaptador = Arc::new(adaptador);

        for (contenido, dedup) in [("primero", "dedup-uno"), ("segundo", "dedup-dos")] {
            adaptador
                .inyectar(evento(&conversacion, contenido, dedup, reloj.ahora()))
                .await
                .expect("el canal debe aceptar el evento");
        }

        let motor = Motor::nuevo(
            AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
            ProcesadorDeEco,
            receptor_eventos,
            Duration::from_secs(3600),
            repositorio,
        );
        drenar_y_soltar(motor).await;
        drop(pools);
    }

    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let historial = repositorio
        .historial(&conversacion)
        .expect("el historial debe poder leerse tras el reinicio");

    assert_eq!(
        historial,
        vec![
            EventoDeHistorial::Entrante("primero".to_string()),
            EventoDeHistorial::Saliente(SalienteHistorico::RespuestaLibre {
                texto: "primero".to_string(),
            }),
            EventoDeHistorial::Entrante("segundo".to_string()),
            EventoDeHistorial::Saliente(SalienteHistorico::RespuestaLibre {
                texto: "segundo".to_string(),
            }),
        ],
        "el historial debe reconstruirse completo y en el mismo orden que se escribió"
    );
}
