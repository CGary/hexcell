//! Limitador de concurrencia de tareas por contenedor.
//!
//! Garantiza un límite estricto sobre el número de tareas de procesamiento de eventos en vuelo
//! concurrentemente por contenedor, acotando la degradación por cambio de contexto de CPU. La
//! adquisición nunca se bloquea de forma indefinida (`intentar_adquirir` utiliza `try_acquire_owned`),
//! y la saturación produce un descarte explícito y registrado de forma coherente con la política
//! de admisión.

use std::fmt;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Límite de concurrencia por defecto por contenedor.
pub const LIMITE_DE_CONCURRENCIA_POR_DEFECTO: usize = 8;

/// Motivo de descarte por límite de concurrencia alcanzado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MotivoDescarteConcurrencia {
    /// Se alcanzó el límite estricto de concurrencia en vuelo para el contenedor.
    Saturacion,
}

impl fmt::Display for MotivoDescarteConcurrencia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Saturacion => write!(
                f,
                "límite estricto de concurrencia de tareas por contenedor alcanzado"
            ),
        }
    }
}

impl std::error::Error for MotivoDescarteConcurrencia {}

/// Limitador de concurrencia basado en un semáforo de Tokio acotado.
#[derive(Clone, Debug)]
pub struct LimitadorDeConcurrencia {
    semaforo: Arc<Semaphore>,
}

impl LimitadorDeConcurrencia {
    /// Crea un nuevo limitador con la cantidad de permisos indicada.
    pub fn nuevo(limite: usize) -> Self {
        Self {
            semaforo: Arc::new(Semaphore::new(limite)),
        }
    }

    /// Intenta adquirir un permiso de concurrencia sin bloquear ni esperar asíncronamente.
    ///
    /// Devuelve `Some(OwnedSemaphorePermit)` si hay permisos disponibles, o `None` si el
    /// limitador está saturado.
    pub fn intentar_adquirir(&self) -> Option<OwnedSemaphorePermit> {
        self.semaforo.clone().try_acquire_owned().ok()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn limita_concurrencia_y_permite_liberar() {
        let limitador = LimitadorDeConcurrencia::nuevo(2);

        let p1 = limitador.intentar_adquirir();
        assert!(p1.is_some());

        let p2 = limitador.intentar_adquirir();
        assert!(p2.is_some());

        // Saturado: el 3er intento devuelve None inmediatamente
        let p3 = limitador.intentar_adquirir();
        assert!(p3.is_none());

        // Liberar un permiso
        drop(p1);

        // Ahora sí se puede adquirir nuevamente
        let p4 = limitador.intentar_adquirir();
        assert!(p4.is_some());
    }

    #[test]
    fn descarte_por_saturacion_formatea_mensaje_en_espanol() {
        let motivo = MotivoDescarteConcurrencia::Saturacion;
        assert_eq!(
            motivo.to_string(),
            "límite estricto de concurrencia de tareas por contenedor alcanzado"
        );
    }
}
