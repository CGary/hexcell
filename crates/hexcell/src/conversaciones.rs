//! Estado de conversaciones: historial persistido y cola de respuestas diferidas en memoria.
//!
//! **El historial ya no vive en memoria.** Desde HEX-006 lo guarda `sessions.db`, que es su única
//! fuente de verdad, y este tipo delega en `hexcell_storage::RepositorioDeSesiones` sin conservar
//! ninguna copia propia delante. Ese es el motivo de que `registrar_entrante`, `registrar_saliente`
//! y `historial` devuelvan ahora `Result`: escribir en disco puede fallar y esta capa no puede
//! fingir que no.
//!
//! # Por qué las respuestas diferidas SÍ siguen en memoria
//!
//! Es la excepción documentada, no un olvido. La cola de diferidas solo se drena cuando el cliente
//! vuelve a escribir a esa misma conversación (`crate::motor`), y un evento entrante posterior a
//! un reinicio produce de todos modos una respuesta **fresca** del procesador. Persistirla
//! compraría, en el mejor caso, reenviar tras el reinicio una respuesta calculada antes de él —más
//! vieja que la que el propio evento nuevo genera— y costaría una tabla, su poda y un modo de
//! fallo nuevo. Se acepta la pérdida a cambio: es una cola acotada de respuestas que el canal ya
//! rechazó una vez.
//!
//! Este módulo nunca construye, interpreta, parte o invierte un [`IdConversacion`]: lo recibe ya
//! resuelto por el puerto de canal y lo usa exclusivamente como clave opaca.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

use hexcell_core::canal::MensajeSaliente;
use hexcell_core::identidad::{IdConversacion, IdRemitente};
use hexcell_storage::{ErrorDeAlmacen, RepositorioDeSesiones};

pub use hexcell_storage::EventoDeHistorial;

/// Tope duro de respuestas diferidas retenidas por conversación.
///
/// Una cola de respuestas sin entregar que creciera sin límite mientras un contacto no vuelve a
/// escribir es exactamente la fuga lenta de memoria que el presupuesto de NFR-01 (≤ 80 MB por
/// célula) no puede absorber. Al alcanzar el tope, la entrada más antigua se descarta para dejar
/// sitio a la nueva: la conversación pierde la respuesta diferida más vieja, no la más reciente,
/// que es la que tiene más probabilidad de seguir siendo relevante cuando el cliente escriba.
const LIMITE_DE_RESPUESTAS_DIFERIDAS_POR_CONVERSACION: usize = 16;

/// Estado de las conversaciones de una célula: historial en `sessions.db` y diferidas en memoria.
pub struct EstadoDeConversaciones {
    repositorio: Arc<RepositorioDeSesiones>,
    /// Respuestas diferidas por la política ante `FueraDeVentana`, pendientes de reintento cuando
    /// el cliente vuelva a escribir. Cola FIFO por conversación: la más antigua se reintenta
    /// primero. Ver la nota del módulo sobre por qué esta parte no se persiste.
    diferidas: HashMap<IdConversacion, VecDeque<MensajeSaliente>>,
}

impl EstadoDeConversaciones {
    /// Crea el estado de conversaciones sobre el repositorio de sesiones ya abierto.
    pub fn nuevo(repositorio: Arc<RepositorioDeSesiones>) -> Self {
        Self {
            repositorio,
            diferidas: HashMap::new(),
        }
    }

    /// Registra que llegó un evento entrante con este contenido para esta conversación.
    ///
    /// El remitente se recibe además de la conversación porque `sessions.db` los distingue: una
    /// conversación de grupo tiene varios contactos, y colapsarlos perdería el dato.
    pub fn registrar_entrante(
        &self,
        conversacion: &IdConversacion,
        remitente: &IdRemitente,
        contenido: &str,
        marca_temporal: SystemTime,
    ) -> Result<(), ErrorDeAlmacen> {
        self.repositorio
            .anotar_entrante(conversacion, remitente, contenido, marca_temporal)
    }

    /// Registra que se envió (o intentó enviar) este mensaje saliente para esta conversación.
    pub fn registrar_saliente(
        &self,
        conversacion: &IdConversacion,
        mensaje: &MensajeSaliente,
        marca_temporal: SystemTime,
    ) -> Result<(), ErrorDeAlmacen> {
        self.repositorio
            .anotar_saliente(conversacion, mensaje, marca_temporal)
    }

    /// Historial completo de una conversación, en el orden en que se registró.
    ///
    /// Devuelve una lista vacía para una conversación que todavía no tiene ningún registro, en
    /// vez de `Option`, porque «sin historial todavía» y «con historial vacío» son la misma cosa
    /// observable para quien llama.
    pub fn historial(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<Vec<EventoDeHistorial>, ErrorDeAlmacen> {
        self.repositorio.historial(conversacion)
    }

    /// Encola una respuesta diferida para esta conversación, aplicando la regla de descarte del
    /// más antiguo si ya se alcanzó [`LIMITE_DE_RESPUESTAS_DIFERIDAS_POR_CONVERSACION`].
    pub fn encolar_diferida(&mut self, conversacion: &IdConversacion, mensaje: MensajeSaliente) {
        let cola = self.diferidas.entry(conversacion.clone()).or_default();
        if cola.len() >= LIMITE_DE_RESPUESTAS_DIFERIDAS_POR_CONVERSACION {
            cola.pop_front();
        }
        cola.push_back(mensaje);
    }

    /// Retira y devuelve, en orden FIFO, todas las respuestas diferidas de esta conversación.
    ///
    /// Tras llamarla la cola queda vacía: quien llama es responsable de reintentarlas todas antes
    /// de considerar la conversación al día, que es exactamente lo que hace `crate::motor` al
    /// recibir un evento nuevo para la misma conversación.
    pub fn drenar_diferidas(&mut self, conversacion: &IdConversacion) -> Vec<MensajeSaliente> {
        match self.diferidas.get_mut(conversacion) {
            Some(cola) => cola.drain(..).collect(),
            None => Vec::new(),
        }
    }
}
