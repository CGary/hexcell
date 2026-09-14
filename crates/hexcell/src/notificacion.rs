//! Sumidero simulado y selector estático de sumideros de notificación operativa.
//!
//! Agrupa dos componentes del binario, siguiendo el precedente de `crate::embeddings`:
//!
//! 1. [`SumideroSimulado`]: doble de prueba que captura notificaciones en proceso, sin red.
//! 2. [`SumideroDeCelula`]: selector estático por enumeración para despachar entre el sumidero
//!    simulado y el sumidero real de Telegram, sin recurrir a `Box<dyn>` (el puerto no es
//!    compatible con objetos de trait; ver `hexcell_core::notificacion`).

use std::fmt;
use std::sync::{Arc, Mutex};

use hexcell_core::notificacion::{Notificacion, SumideroDeNotificaciones};

/// Avería del sumidero simulado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDeSumideroSimulado {
    /// Avería forzada a propósito por un test mediante `SumideroSimulado::que_falla`.
    AveriaSimulada,
}

impl fmt::Display for ErrorDeSumideroSimulado {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AveriaSimulada => write!(
                f,
                "avería de notificación simulada, forzada a propósito por el test"
            ),
        }
    }
}

impl std::error::Error for ErrorDeSumideroSimulado {}

/// Sumidero de notificaciones en proceso, sin ninguna llamada de red, para tests.
#[derive(Clone, Debug, Default)]
pub struct SumideroSimulado {
    notificaciones: Arc<Mutex<Vec<Notificacion>>>,
    forzar_averia: bool,
}

impl SumideroSimulado {
    /// Construye un sumidero simulado vacío que nunca falla.
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Construye un sumidero simulado configurado para fallar incondicionalmente.
    pub fn que_falla() -> Self {
        Self {
            notificaciones: Arc::new(Mutex::new(Vec::new())),
            forzar_averia: true,
        }
    }

    /// Devuelve una copia de las notificaciones capturadas hasta ahora, en orden de llegada.
    pub fn notificaciones(&self) -> Vec<Notificacion> {
        self.notificaciones
            .lock()
            .expect("cerrojo del sumidero simulado envenenado")
            .clone()
    }

    /// Cantidad de notificaciones capturadas hasta ahora.
    pub fn cantidad(&self) -> usize {
        self.notificaciones
            .lock()
            .expect("cerrojo del sumidero simulado envenenado")
            .len()
    }
}

impl SumideroDeNotificaciones for SumideroSimulado {
    type Error = ErrorDeSumideroSimulado;

    async fn notificar(&self, notificacion: Notificacion) -> Result<(), Self::Error> {
        if self.forzar_averia {
            return Err(ErrorDeSumideroSimulado::AveriaSimulada);
        }
        self.notificaciones
            .lock()
            .expect("cerrojo del sumidero simulado envenenado")
            .push(notificacion);
        Ok(())
    }
}

/// Error unificado devuelto por el selector de sumidero de notificación de la célula.
#[derive(Debug)]
pub enum ErrorDeNotificacion {
    /// Error devuelto por el sumidero simulado.
    Simulado(ErrorDeSumideroSimulado),
    /// Error devuelto por el sumidero Telegram.
    Telegram(crate::notificador_telegram::ErrorDeTelegram),
}

impl fmt::Display for ErrorDeNotificacion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simulado(e) => write!(f, "{e}"),
            Self::Telegram(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErrorDeNotificacion {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Simulado(e) => Some(e),
            Self::Telegram(e) => Some(e),
        }
    }
}

/// Selector estático del sumidero de notificación (simulado o real Telegram).
///
/// Permite despachar llamadas polimórficas sin recurrir a objetos de trait dinámicos (`dyn`),
/// porque `SumideroDeNotificaciones` no es compatible con `dyn` (retorna `impl Future`).
#[derive(Clone)]
pub enum SumideroDeCelula {
    /// Variante simulada, sin llamadas de red.
    Simulado(SumideroSimulado),
    /// Variante real sobre la API de Telegram.
    Telegram(Box<crate::notificador_telegram::NotificadorTelegram>),
}

impl SumideroDeCelula {
    /// Elige la variante Telegram si se proveyó configuración, o la simulada en caso contrario.
    pub fn desde_configuracion(
        configuracion: Option<crate::notificador_telegram::ConfiguracionDeTelegram>,
    ) -> Self {
        match configuracion {
            Some(cfg) => Self::Telegram(Box::new(
                crate::notificador_telegram::NotificadorTelegram::nuevo(cfg),
            )),
            None => Self::Simulado(SumideroSimulado::nuevo()),
        }
    }
}

impl SumideroDeNotificaciones for SumideroDeCelula {
    type Error = ErrorDeNotificacion;

    async fn notificar(&self, notificacion: Notificacion) -> Result<(), Self::Error> {
        match self {
            Self::Simulado(sumidero) => sumidero
                .notificar(notificacion)
                .await
                .map_err(ErrorDeNotificacion::Simulado),
            Self::Telegram(sumidero) => sumidero
                .notificar(notificacion)
                .await
                .map_err(ErrorDeNotificacion::Telegram),
        }
    }
}
