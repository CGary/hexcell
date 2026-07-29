//! Motor de mensajería: consume eventos, despacha al procesador y envía por el puerto de canal.
//!
//! El motor no conoce ningún transporte concreto: es genérico sobre cualquier implementación de
//! `ChannelAdapter` (`hexcell_core::canal`) y sobre cualquier `ProcesadorDeMensajes`
//! (`crate::procesador`). Recibe ambos por inyección en su constructor, nunca fija el tipo de un
//! adaptador concreto.
//!
//! # Convención de entrega de eventos
//!
//! El puerto `ChannelAdapter` declara solo `send` y `estado_ventana`; el mecanismo de entrega de
//! `EventoEntrante` no es uno de los siete elementos de FR-12 y se decide en esta etapa
//! (`docs/adr/adr-0016-convencion-de-entrega-de-eventos.md`). La convención, documentada aquí sin
//! nombrar ningún transporte concreto, es: todo adaptador entrega sus eventos por un canal
//! `tokio::sync::mpsc` acotado que él mismo crea y posee, y cuyo extremo receptor pasa a este
//! motor en el momento de construirse.

use hexcell_core::canal::{ChannelAdapter, EventoEntrante, MensajeSaliente, ResultadoEnvio};
use tokio::sync::mpsc;

use crate::procesador::ProcesadorDeMensajes;

/// Motor de mensajería de una célula: bucle asíncrono sobre un adaptador y un procesador.
pub struct Motor<A, P>
where
    A: ChannelAdapter,
    P: ProcesadorDeMensajes,
{
    adaptador: A,
    procesador: P,
    receptor_eventos: mpsc::Receiver<EventoEntrante>,
}

impl<A, P> Motor<A, P>
where
    A: ChannelAdapter,
    P: ProcesadorDeMensajes,
{
    /// Construye el motor a partir del adaptador, el procesador y el receptor de eventos que el
    /// propio adaptador entregó al crearse, siguiendo la convención de entrega descrita arriba.
    pub fn nuevo(
        adaptador: A,
        procesador: P,
        receptor_eventos: mpsc::Receiver<EventoEntrante>,
    ) -> Self {
        Self {
            adaptador,
            procesador,
            receptor_eventos,
        }
    }

    /// Ejecuta el bucle de consumo hasta que el canal de eventos se cierra.
    ///
    /// Por cada evento: lo despacha al procesador y, si este decide responder, envía la
    /// respuesta con `send(conversacion, mensaje)`. El `Result<ResultadoEnvio, A::Error>` se trata
    /// con un `match` que nombra las cinco variantes de `ResultadoEnvio` y el caso `Err`, sin
    /// brazo comodín: una variante nueva rompe la compilación de este archivo en vez de colarse
    /// silenciosa por un `_ =>`.
    pub async fn ejecutar(&mut self) {
        while let Some(evento) = self.receptor_eventos.recv().await {
            let Some(mensaje) = self.procesador.procesar(&evento) else {
                continue;
            };

            self.enviar_y_registrar(&evento, mensaje).await;
        }
    }

    async fn enviar_y_registrar(&self, evento: &EventoEntrante, mensaje: MensajeSaliente) {
        let resultado = self.adaptador.send(&evento.conversacion, mensaje).await;

        match resultado {
            Ok(ResultadoEnvio::Aceptado) => {
                println!("motor: envío aceptado por el canal configurado");
            }
            Ok(ResultadoEnvio::FueraDeVentana) => {
                eprintln!(
                    "motor: envío rechazado, ventana de servicio cerrada para la conversación"
                );
            }
            Ok(ResultadoEnvio::PlantillaRequerida) => {
                eprintln!("motor: envío rechazado, el canal exige una plantilla aprobada");
            }
            Ok(ResultadoEnvio::LimiteDeTasa) => {
                eprintln!("motor: envío rechazado, el canal está limitando la tasa de envío");
            }
            Ok(ResultadoEnvio::DestinatarioInvalido) => {
                eprintln!("motor: envío rechazado, el destinatario no es válido");
            }
            Err(averia) => {
                eprintln!("motor: avería de transporte al enviar: {averia}");
            }
        }
    }
}
