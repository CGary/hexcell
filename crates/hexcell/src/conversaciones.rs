//! Estado de conversaciones en memoria: historial e hilo de respuestas diferidas por conversación.
//!
//! **Estructura TRANSITORIA**, igual que [`crate::deduplicacion`]: HEX-006 la sustituye por
//! `sessions.db`. No hay aquí ningún esquema, ninguna migración ni ninguna sentencia SQL — esta
//! tarea no inventa la persistencia real que HEX-006 diseñará, solo el estado en memoria que hace
//! observable, hoy, que un hilo de conversación sobrevive a un re-emparejamiento del dispositivo
//! (AC-5) y que la política ante `FueraDeVentana` (`crate::motor`) tiene dónde encolar sus
//! respuestas diferidas (AC-3, AC-4).
//!
//! Este módulo nunca construye, interpreta, parte o invierte un [`IdConversacion`]: lo recibe ya
//! resuelto por el puerto de canal y lo usa exclusivamente como clave opaca de un mapa. Eso es
//! AC-6 al nivel estructural de este archivo.

use std::collections::{HashMap, VecDeque};

use hexcell_core::canal::MensajeSaliente;
use hexcell_core::identidad::IdConversacion;

/// Tope duro de respuestas diferidas retenidas por conversación.
///
/// Una cola de respuestas sin entregar que creciera sin límite mientras un contacto no vuelve a
/// escribir es exactamente la fuga lenta de memoria que el presupuesto de NFR-01 (≤ 80 MB por
/// célula) no puede absorber. Al alcanzar el tope, la entrada más antigua se descarta para dejar
/// sitio a la nueva: la conversación pierde la respuesta diferida más vieja, no la más reciente,
/// que es la que tiene más probabilidad de seguir siendo relevante cuando el cliente escriba.
const LIMITE_DE_RESPUESTAS_DIFERIDAS_POR_CONVERSACION: usize = 16;

/// Un elemento del historial observable de una conversación: lo que entró y lo que salió, en
/// el orden en que el motor los procesó.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventoDeHistorial {
    /// Contenido textual ya normalizado de un evento entrante procesado para esta conversación.
    Entrante(String),
    /// Un mensaje saliente que el motor envió (o intentó enviar) para esta conversación.
    Saliente(MensajeSaliente),
}

/// Estado en memoria de un único hilo de conversación.
#[derive(Default)]
struct HiloDeConversacion {
    /// Historial completo, en orden de procesamiento, de lo entrante y lo saliente.
    historial: Vec<EventoDeHistorial>,
    /// Respuestas diferidas por la política ante `FueraDeVentana`, pendientes de reintento
    /// cuando el cliente vuelva a escribir. Cola FIFO: la más antigua se reintenta primero.
    diferidas: VecDeque<MensajeSaliente>,
}

/// Mapa de `IdConversacion` a su estado en memoria: el historial de una célula completa.
#[derive(Default)]
pub struct EstadoDeConversaciones {
    hilos: HashMap<IdConversacion, HiloDeConversacion>,
}

impl EstadoDeConversaciones {
    /// Crea un estado de conversaciones vacío.
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Registra que llegó un evento entrante con este contenido para esta conversación.
    pub fn registrar_entrante(&mut self, conversacion: &IdConversacion, contenido: String) {
        self.hilos
            .entry(conversacion.clone())
            .or_default()
            .historial
            .push(EventoDeHistorial::Entrante(contenido));
    }

    /// Registra que se envió (o intentó enviar) este mensaje saliente para esta conversación.
    pub fn registrar_saliente(&mut self, conversacion: &IdConversacion, mensaje: MensajeSaliente) {
        self.hilos
            .entry(conversacion.clone())
            .or_default()
            .historial
            .push(EventoDeHistorial::Saliente(mensaje));
    }

    /// Historial completo de una conversación, en el orden en que se registró.
    ///
    /// Devuelve una lista vacía para una conversación que todavía no tiene ningún registro, en
    /// vez de `Option`, porque «sin historial todavía» y «con historial vacío» son la misma cosa
    /// observable para quien llama.
    pub fn historial(&self, conversacion: &IdConversacion) -> &[EventoDeHistorial] {
        self.hilos
            .get(conversacion)
            .map(|hilo| hilo.historial.as_slice())
            .unwrap_or(&[])
    }

    /// Encola una respuesta diferida para esta conversación, aplicando la regla de descarte del
    /// más antiguo si ya se alcanzó [`LIMITE_DE_RESPUESTAS_DIFERIDAS_POR_CONVERSACION`].
    pub fn encolar_diferida(&mut self, conversacion: &IdConversacion, mensaje: MensajeSaliente) {
        let hilo = self.hilos.entry(conversacion.clone()).or_default();
        if hilo.diferidas.len() >= LIMITE_DE_RESPUESTAS_DIFERIDAS_POR_CONVERSACION {
            hilo.diferidas.pop_front();
        }
        hilo.diferidas.push_back(mensaje);
    }

    /// Retira y devuelve, en orden FIFO, todas las respuestas diferidas de esta conversación.
    ///
    /// Tras llamarla la cola queda vacía: quien llama es responsable de reintentarlas todas antes
    /// de considerar la conversación al día, que es exactamente lo que hace `crate::motor` al
    /// recibir un evento nuevo para la misma conversación.
    pub fn drenar_diferidas(&mut self, conversacion: &IdConversacion) -> Vec<MensajeSaliente> {
        match self.hilos.get_mut(conversacion) {
            Some(hilo) => hilo.diferidas.drain(..).collect(),
            None => Vec::new(),
        }
    }
}
