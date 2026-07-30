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
//!
//! # Orden de las tres políticas nuevas por evento
//!
//! El orden es la propia política, no un detalle de implementación:
//!
//! 1. **Deduplicación primero.** Se consulta el registro con el identificador de deduplicación y
//!    la marca temporal del evento; un veredicto de duplicado hace `continue` sin despachar al
//!    procesador y sin enviar nada (AC-7).
//! 2. **Drenaje de diferidas, antes de la respuesta del propio evento.** Que llegue un evento
//!    nuevo para una conversación es, precisamente, que el cliente ha vuelto a escribir, y eso es
//!    lo que reabre la ventana de servicio en el adaptador simulado. Las respuestas que quedaron
//!    diferidas para esa conversación se reintentan **antes** de la respuesta del evento que
//!    acaba de llegar, para que el hilo se mantenga cronológico.
//! 3. **Registro, despacho y envío**, como hacía el motor antes de esta tarea, salvo que el brazo
//!    `FueraDeVentana` ya no se limita a registrar un mensaje: aplica la política (encolar la
//!    respuesta como diferida) en vez de tratar el rechazo como los demás.
//!
//! # Política ante `FueraDeVentana`: diferir, no escalar
//!
//! Se eligió **diferir** (encolar la respuesta hasta que el cliente vuelva a escribir) en vez de
//! **escalar a un humano**. La escalada se descartó por falta de dónde aterrizar, no por
//! preferencia: no existe todavía ningún registro estructurado (llega en HEX-008), ninguna vía de
//! notificación a un operador ni ningún plano de CLI de administración (llega en la etapa A-6); una
//! rama de escalada hoy sería, en la práctica, imprimir una línea a `stderr` y llamarlo política.
//! Diferir, en cambio, es implementable, observable y probable en memoria ahora mismo.
//!
//! La cola de diferidas es **acotada por conversación**
//! (`crate::conversaciones::EstadoDeConversaciones`) con una regla de descarte del más antiguo en
//! el tope: una cola sin límite de respuestas no entregables es exactamente la fuga lenta que el
//! presupuesto de ≤ 80 MB por célula de NFR-01 no puede absorber. No hay bucle de reintento, ni
//! temporizador de `backoff`, ni tarea de fondo: las diferidas se reintentan únicamente cuando
//! llega un evento **posterior** para esa misma conversación, y una respuesta rechazada de nuevo
//! al drenar vuelve a encolarse, sujeta al mismo tope. Un temporizador necesitaría un reloj dentro
//! del motor, exactamente el acoplamiento que este módulo evita a propósito.

use std::time::Duration;

use hexcell_core::canal::{ChannelAdapter, EventoEntrante, MensajeSaliente, ResultadoEnvio};
use hexcell_core::identidad::IdConversacion;
use tokio::sync::mpsc;

use crate::conversaciones::EstadoDeConversaciones;
use crate::deduplicacion::{RegistroDeDeduplicacion, VeredictoDeDeduplicacion};
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
    deduplicacion: RegistroDeDeduplicacion,
    conversaciones: EstadoDeConversaciones,
}

impl<A, P> Motor<A, P>
where
    A: ChannelAdapter,
    P: ProcesadorDeMensajes,
{
    /// Construye el motor a partir del adaptador, el procesador, el receptor de eventos que el
    /// propio adaptador entregó al crearse (siguiendo la convención de entrega descrita arriba) y
    /// la ventana de retención con la que arranca el registro de deduplicación que el motor posee
    /// (`Configuracion::ventana_deduplicacion` en producción).
    pub fn nuevo(
        adaptador: A,
        procesador: P,
        receptor_eventos: mpsc::Receiver<EventoEntrante>,
        ventana_deduplicacion: Duration,
    ) -> Self {
        Self {
            adaptador,
            procesador,
            receptor_eventos,
            deduplicacion: RegistroDeDeduplicacion::nuevo(ventana_deduplicacion),
            conversaciones: EstadoDeConversaciones::nuevo(),
        }
    }

    /// Historial en memoria de una conversación, para que los tests observen su continuidad.
    pub fn historial(
        &self,
        conversacion: &IdConversacion,
    ) -> &[crate::conversaciones::EventoDeHistorial] {
        self.conversaciones.historial(conversacion)
    }

    /// Ejecuta el bucle de consumo hasta que el canal de eventos se cierra.
    ///
    /// Por cada evento aplica, en orden, deduplicación, drenaje de diferidas y despacho al
    /// procesador; ver la documentación del módulo para el porqué de ese orden exacto.
    pub async fn ejecutar(&mut self) {
        while let Some(evento) = self.receptor_eventos.recv().await {
            let veredicto = self
                .deduplicacion
                .procesar(evento.deduplicacion.clone(), evento.marca_temporal);
            if veredicto == VeredictoDeDeduplicacion::Duplicado {
                println!(
                    "motor: evento entrante descartado por duplicado, ya procesado dentro de la \
                     ventana de retención"
                );
                continue;
            }

            self.drenar_diferidas(&evento.conversacion).await;

            self.conversaciones
                .registrar_entrante(&evento.conversacion, evento.contenido.clone());

            let Some(mensaje) = self.procesador.procesar(&evento) else {
                continue;
            };

            self.enviar_y_registrar(&evento.conversacion, mensaje).await;
        }
    }

    /// Reintenta, en orden de llegada, cada respuesta que quedó diferida para esta conversación.
    async fn drenar_diferidas(&mut self, conversacion: &IdConversacion) {
        for mensaje in self.conversaciones.drenar_diferidas(conversacion) {
            self.enviar_y_registrar(conversacion, mensaje).await;
        }
    }

    async fn enviar_y_registrar(
        &mut self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) {
        let resultado = self.adaptador.send(conversacion, mensaje.clone()).await;

        match resultado {
            Ok(ResultadoEnvio::Aceptado) => {
                println!("motor: envío aceptado por el canal configurado");
                self.conversaciones
                    .registrar_saliente(conversacion, mensaje);
            }
            Ok(ResultadoEnvio::FueraDeVentana) => {
                println!(
                    "motor: ventana de servicio cerrada, la respuesta se difiere hasta que el \
                     cliente vuelva a escribir"
                );
                self.conversaciones.encolar_diferida(conversacion, mensaje);
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
