//! Retroceso exponencial determinista con techo para la reconexión IPC.
//!
//! Los parámetros se inyectan por construcción, no se leen de variables de entorno. El protocolo
//! asigna las cinco variables `HEXCELL_RETROCESO_*` a la política propia del **sidecar** para su
//! reconexión de WhatsApp (sección 5 de `docs/protocolo-ipc-nucleo-sidecar.md`); al núcleo solo
//! le concede las palabras «retroceso exponencial y techo» sin valores, y su calibración es una
//! decisión abierta registrada en `docs/STATUS.md`. Por eso los valores por omisión de este
//! módulo están documentados como **provisionales** y nunca como calibrados.
//!
//! La inyección es también lo que permite que los tests de reconexión (AC-4) se ejecuten sin
//! dormir sobre el reloj de pared.

use std::time::Duration;

/// Retroceso exponencial determinista con techo.
///
/// Cada llamada a [`Retroceso::siguiente`] devuelve la espera actual y multiplica la base por
/// el factor, hasta alcanzar el techo. [`Retroceso::reiniciar`] vuelve al valor inicial.
#[derive(Clone, Debug)]
pub struct Retroceso {
    /// Espera inicial tras la primera desconexión.
    inicial: Duration,
    /// Factor multiplicador de cada intento sucesivo.
    factor: u32,
    /// Espera máxima: ninguna espera supera este valor.
    techo: Duration,
    /// Espera actual; crece con cada llamada a `siguiente`.
    actual: Duration,
}

impl Retroceso {
    /// Construye un retroceso con los parámetros dados.
    ///
    /// Los valores son **provisionales y pendientes de calibración** bajo tráfico real
    /// (`docs/STATUS.md`). No se presentan como calibrados.
    pub fn nuevo(inicial: Duration, factor: u32, techo: Duration) -> Self {
        Self {
            inicial,
            factor,
            techo,
            actual: inicial,
        }
    }

    /// Retroceso con valores provisionales por omisión.
    ///
    /// Estos valores son un punto de partida razonable, **no una medición bajo tráfico real**.
    /// Su calibración es una decisión abierta registrada en `docs/STATUS.md`.
    pub fn por_omision() -> Self {
        Self::nuevo(Duration::from_millis(500), 2, Duration::from_secs(30))
    }

    /// Devuelve la espera actual y avanza al siguiente nivel.
    pub fn siguiente(&mut self) -> Duration {
        let espera = self.actual;
        let siguiente = self.actual.saturating_mul(self.factor);
        self.actual = if siguiente > self.techo {
            self.techo
        } else {
            siguiente
        };
        espera
    }

    /// Reinicia el retroceso al valor inicial.
    pub fn reiniciar(&mut self) {
        self.actual = self.inicial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_retroceso_crece_exponencialmente_hasta_el_techo() {
        let mut retroceso =
            Retroceso::nuevo(Duration::from_millis(100), 2, Duration::from_millis(500));

        assert_eq!(retroceso.siguiente(), Duration::from_millis(100));
        assert_eq!(retroceso.siguiente(), Duration::from_millis(200));
        assert_eq!(retroceso.siguiente(), Duration::from_millis(400));
        // El siguiente sería 800, pero el techo lo limita a 500.
        assert_eq!(retroceso.siguiente(), Duration::from_millis(500));
        assert_eq!(retroceso.siguiente(), Duration::from_millis(500));
    }

    #[test]
    fn reiniciar_vuelve_al_valor_inicial() {
        let mut retroceso =
            Retroceso::nuevo(Duration::from_millis(100), 2, Duration::from_secs(10));

        let _ = retroceso.siguiente();
        let _ = retroceso.siguiente();
        retroceso.reiniciar();
        assert_eq!(retroceso.siguiente(), Duration::from_millis(100));
    }
}
