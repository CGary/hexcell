//! Adaptador `ChannelAdapter` simulado: implementación en memoria con semántica de Cloud API.
//!
//! Convención de entrega de eventos (`docs/adr/adr-0016-convencion-de-entrega-de-eventos.md`): el
//! trait `ChannelAdapter` de `hexcell-core` declara solo `send` y `estado_ventana` — el mecanismo
//! de entrega de `EventoEntrante` no es uno de los siete elementos de FR-12 y se decide en esta
//! misma etapa. Este adaptador crea y posee un canal `tokio::sync::mpsc` **acotado** — acotado
//! para que una ráfaga aplique contrapresión en vez de crecer sin límite contra el presupuesto de
//! memoria de NFR-01 — y entrega su extremo receptor al `Motor` en el momento de construirse. La
//! etapa A-3 (whatsmeow) ya cerrada adopta la misma convención: cada adaptador entrega sus eventos
//! por un canal propio, no por un método nuevo del trait.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use hexcell_core::canal::{
    ChannelAdapter, DURACION_VENTANA_SERVICIO, EstadoVentanaServicio, EventoEntrante,
    MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::IdConversacion;
use tokio::sync::mpsc;

use crate::reloj::Reloj;

/// Avería de transporte del adaptador simulado.
///
/// No es `std::convert::Infallible` a propósito: un tipo de error deshabitado dejaría el brazo
/// `Err` del `Motor` inalcanzable en la práctica, y el propósito de este adaptador es precisamente
/// permitir que un test fuerce esa avería y compruebe que el motor la trata sin `unwrap()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDelAdaptadorSimulado {
    /// Avería de transporte forzada a voluntad por el test mediante `forzar_averia()`.
    AveriaDeTransporteSimulada,
}

impl fmt::Display for ErrorDelAdaptadorSimulado {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AveriaDeTransporteSimulada => {
                write!(
                    f,
                    "avería de transporte simulada, forzada a propósito por el test"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDelAdaptadorSimulado {}

/// Estado interno mutable del adaptador, agrupado para que un único `Mutex` lo proteja entero.
struct EstadoInterno {
    /// Ancla de la ventana de servicio de cada conversación: el instante del último evento
    /// entrante inyectado para ella.
    anclas_de_ventana: HashMap<IdConversacion, std::time::SystemTime>,
    /// Resultados forzados específicos de una conversación, consumidos uno por llamada.
    forzados_por_conversacion: HashMap<IdConversacion, VecDeque<ResultadoEnvio>>,
    /// Resultados forzados sin conversación asociada, consumidos uno por llamada a `send`.
    forzados_siguientes: VecDeque<ResultadoEnvio>,
    /// Si está activo, la próxima llamada a `send` devuelve `Err` y lo desactiva.
    forzar_averia: bool,
    /// Copia de cada envío realizado, para que el test la inspeccione con `envios_capturados()`.
    envios_capturados: Vec<(IdConversacion, MensajeSaliente, ResultadoEnvio)>,
}

/// Adaptador `ChannelAdapter` en memoria que imita la semántica restrictiva de la Cloud API.
///
/// Imita la Cloud API, **no** whatsmeow: la ventana de servicio de 24 horas, `PlantillaRequerida`
/// fuera de ella y los cuatro rechazos de FR-12 son el caso restrictivo que el puerto admite, no
/// una obligación del canal propio (`hexcell_core::canal`, distinción TIPO/POLÍTICA).
pub struct AdaptadorSimulado {
    reloj: Arc<dyn Reloj + Send + Sync>,
    remitente_eventos: mpsc::Sender<EventoEntrante>,
    estado: Mutex<EstadoInterno>,
}

impl AdaptadorSimulado {
    /// Crea el adaptador simulado y el receptor que el `Motor` debe consumir.
    ///
    /// `capacidad` acota el canal `mpsc`: por debajo de ella `inyectar` se completa de inmediato,
    /// por encima aplica contrapresión, igual que hará cualquier adaptador real.
    pub fn nuevo(
        reloj: Arc<dyn Reloj + Send + Sync>,
        capacidad: usize,
    ) -> (Self, mpsc::Receiver<EventoEntrante>) {
        let (remitente_eventos, receptor_eventos) = mpsc::channel(capacidad);
        let adaptador = Self {
            reloj,
            remitente_eventos,
            estado: Mutex::new(EstadoInterno {
                anclas_de_ventana: HashMap::new(),
                forzados_por_conversacion: HashMap::new(),
                forzados_siguientes: VecDeque::new(),
                forzar_averia: false,
                envios_capturados: Vec::new(),
            }),
        };
        (adaptador, receptor_eventos)
    }

    /// Inyecta un evento entrante de forma determinista: lo entrega al `Motor` por el canal y
    /// ancla (o refresca) la ventana de servicio de su conversación en `reloj.ahora()`.
    ///
    /// Devuelve un error si el canal ya se cerró (el `Motor` dejó de escuchar), que no es un caso
    /// que el simulado deba enmascarar.
    pub async fn inyectar(
        &self,
        evento: EventoEntrante,
    ) -> Result<(), mpsc::error::SendError<EventoEntrante>> {
        {
            let mut estado = self.estado.lock().expect(
                "el mutex interno de AdaptadorSimulado no debería estar envenenado en un test",
            );
            estado
                .anclas_de_ventana
                .insert(evento.conversacion.clone(), self.reloj.ahora());
        }
        self.remitente_eventos.send(evento).await
    }

    /// Encola un resultado forzado para una conversación concreta; se consume en la próxima
    /// llamada a `send` sobre esa misma conversación, y solo en esa llamada.
    pub fn forzar(&self, conversacion: &IdConversacion, resultado: ResultadoEnvio) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado
            .forzados_por_conversacion
            .entry(conversacion.clone())
            .or_default()
            .push_back(resultado);
    }

    /// Encola un resultado forzado para la próxima llamada a `send`, sea cual sea la conversación.
    pub fn forzar_siguiente(&self, resultado: ResultadoEnvio) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.forzados_siguientes.push_back(resultado);
    }

    /// Hace que la próxima llamada a `send` devuelva `Err`, una única vez.
    pub fn forzar_averia(&self) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.forzar_averia = true;
    }

    /// Instantánea de cada envío realizado hasta ahora, en el orden en que ocurrieron.
    pub fn envios_capturados(&self) -> Vec<(IdConversacion, MensajeSaliente, ResultadoEnvio)> {
        let estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.envios_capturados.clone()
    }
}

impl ChannelAdapter for AdaptadorSimulado {
    type Error = ErrorDelAdaptadorSimulado;

    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        let resultado = {
            let mut estado = self.estado.lock().expect(
                "el mutex interno de AdaptadorSimulado no debería estar envenenado en un test",
            );

            if estado.forzar_averia {
                estado.forzar_averia = false;
                return Err(ErrorDelAdaptadorSimulado::AveriaDeTransporteSimulada);
            }

            let forzado = estado
                .forzados_por_conversacion
                .get_mut(conversacion)
                .and_then(VecDeque::pop_front)
                .or_else(|| estado.forzados_siguientes.pop_front());

            let resultado = forzado.unwrap_or_else(|| {
                let ahora = self.reloj.ahora();
                match &mensaje {
                    MensajeSaliente::Plantilla { .. } => ResultadoEnvio::Aceptado,
                    MensajeSaliente::RespuestaLibre(_) => {
                        match estado.anclas_de_ventana.get(conversacion) {
                            Some(ancla) if ahora >= *ancla + DURACION_VENTANA_SERVICIO => {
                                ResultadoEnvio::FueraDeVentana
                            }
                            _ => ResultadoEnvio::Aceptado,
                        }
                    }
                }
            });

            estado
                .envios_capturados
                .push((conversacion.clone(), mensaje.clone(), resultado));

            resultado
        };

        Ok(resultado)
    }

    async fn estado_ventana(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<EstadoVentanaServicio, Self::Error> {
        let estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        let ahora = self.reloj.ahora();
        match estado.anclas_de_ventana.get(conversacion) {
            Some(ancla) if ahora < *ancla + DURACION_VENTANA_SERVICIO => {
                Ok(EstadoVentanaServicio::Abierta {
                    expira_en: *ancla + DURACION_VENTANA_SERVICIO,
                })
            }
            _ => Ok(EstadoVentanaServicio::Cerrada),
        }
    }
}
