//! Apagado ordenado: captura de señales, límite de drenaje y la señal que recibe el motor.
//!
//! `Apagado::instalar` registra `SIGTERM` **y** `SIGINT` con `tokio::signal::unix::signal`: `SIGINT`
//! porque quien lanza el binario a mano desde una terminal merece la misma salida ordenada que el
//! orquestador que envía `SIGTERM`, y cuesta tres líneas más. Se registran nada más analizar la
//! configuración, antes de abrir la persistencia o vincular cualquier puerto, para que una señal
//! que llegue durante el arranque quede capturada en vez de matar el proceso con la acción por
//! defecto del sistema operativo.
//!
//! # Por qué no se usa `tokio-util` con `CancellationToken`
//!
//! `tokio::sync::watch` ya está habilitado en la característica `sync` que este crate ya declara, y
//! expresa exactamente lo que aquí hace falta: un valor compartido que cambia una vez y que
//! cualquier receptor puede observar. `CancellationToken` duplicaría esa expresividad a cambio de
//! una dependencia nueva; el descarte está registrado como D-18 en
//! `docs/bitacora-de-descartes.md`.
//!
//! # Por qué [`SenalDeApagado`] no guarda su propio emisor
//!
//! Un receptor de `watch` cuyo emisor se ha destruido devuelve `Err` desde `changed()` de
//! inmediato. Si [`SenalDeApagado`] retuviera el emisor dentro de sí misma, cada instancia
//! devuelta por [`SenalDeApagado::nunca`] apagaría el motor al primer sondeo en vez de no
//! apagarlo nunca — justo lo que necesitan los seis sitios de prueba existentes que construyen un
//! `Motor` sin ningún apagado en marcha. El emisor real vive dentro de [`Apagado`], que
//! `main.rs` mantiene con vida durante toda la ejecución del proceso precisamente para que nunca se
//! destruya mientras el motor corre.

use std::time::Duration;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

/// Límite de drenaje por defecto tras recibir la señal de apagado.
///
/// Veinte segundos (decisión del 26 de agosto de 2026, subido de diez en el mismo movimiento que
/// el plazo de 8000 ms del proveedor real: 8 s x 2 intentos = 16 s deben caber bajo el drenaje).
/// Sigue lejos de los treinta del plazo de gracia del PRD: el punto de control del WAL más el
/// resto de la salida tienen que caber en lo que quede tras el drenaje. La etapa A-6 alineará el
/// `stop_timeout` del contenedor con este valor.
pub const LIMITE_DE_DRENAJE_POR_DEFECTO: Duration = Duration::from_secs(20);

/// Señal de apagado que el motor observa entre cada evento.
///
/// Envuelve el receptor de un `tokio::sync::watch` y el límite de drenaje con el que el motor debe
/// dejar de aceptar más trabajo tras la señal. No guarda su propio emisor (ver la nota del módulo).
#[derive(Debug)]
pub struct SenalDeApagado {
    receptor: watch::Receiver<bool>,
    limite_de_drenaje: Duration,
}

impl SenalDeApagado {
    /// Señal que nunca se dispara: para los seis sitios de prueba existentes que no ejercitan el
    /// apagado ordenado y que deben seguir comportándose exactamente como antes de esta tarea.
    ///
    /// El emisor se crea aquí, dentro de la función, y se descarta al volver: el receptor queda
    /// vivo, pero como nadie más sostiene el emisor, cualquier `changed()` posterior devolvería
    /// `Err` de inmediato en vez de quedarse esperando para siempre — que es exactamente lo que
    /// "nunca" debe significar para un receptor que ya vale `false` desde el arranque.
    pub fn nunca() -> Self {
        let (_emisor, receptor) = watch::channel(false);
        Self {
            receptor,
            limite_de_drenaje: LIMITE_DE_DRENAJE_POR_DEFECTO,
        }
    }

    /// ¿Ha llegado la señal de apagado?
    ///
    /// Sondeo síncrono sobre el último valor observado, sin esperar a un cambio: es lo que el
    /// motor usa dentro de `select!` como una de sus dos ramas.
    pub async fn recibida(&mut self) {
        // Un receptor cuyo emisor ya no existe (el caso de `nunca()`) devuelve `Err` de
        // inmediato; en ese caso este futuro no termina nunca, que es la semántica deseada.
        loop {
            if *self.receptor.borrow() {
                return;
            }
            if self.receptor.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// Límite de drenaje que el motor debe respetar tras recibir la señal.
    pub fn limite_de_drenaje(&self) -> Duration {
        self.limite_de_drenaje
    }
}

/// Marcador devuelto por [`Apagado::instalar`].
///
/// No necesita guardar el emisor del canal de `watch`: la tarea de fondo que arranca `instalar` lo
/// posee y se queda aparcada para siempre (`std::future::pending`), así que el emisor vive tanto
/// como el propio proceso sin que nada externo tenga que retenerlo. Este tipo existe para que la
/// raíz de composición tenga un valor que nombrar en la firma, documentando la intención en el
/// punto de la llamada.
pub struct Apagado;

impl Apagado {
    /// Registra los manejadores de señal y arranca la tarea que los observa.
    ///
    /// Falible: registrar un manejador de señal puede fallar, y este módulo no llama nunca a
    /// `expect()` para tratarlo — el error se devuelve para que `main` decida cómo reportarlo.
    pub fn instalar(limite_de_drenaje: Duration) -> std::io::Result<(Self, SenalDeApagado)> {
        let mut senal_terminar = signal(SignalKind::terminate())?;
        let mut senal_interrumpir = signal(SignalKind::interrupt())?;

        let (emisor, receptor) = watch::channel(false);

        tokio::task::spawn(async move {
            tokio::select! {
                _ = senal_terminar.recv() => {}
                _ = senal_interrumpir.recv() => {}
            }
            let _ = emisor.send(true);
            // El emisor se mantiene vivo dentro de esta tarea, que se queda aparcada para
            // siempre: así ningún receptor ve `Err` tras el cambio, y el valor `true` ya
            // observado por `borrow()` basta para que `recibida()` devuelva de inmediato.
            std::future::pending::<()>().await;
        });

        Ok((
            Self,
            SenalDeApagado {
                receptor,
                limite_de_drenaje,
            },
        ))
    }
}
