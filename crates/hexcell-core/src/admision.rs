//! Módulo de control de admisión mediante Algoritmo de Tasa de Celdas Genérico (GCRA).
//!
//! Implementa una tasa sostenida y tolerancia a ráfagas configurables utilizando un único
//! tiempo de llegada teórico (TAT, *Theoretical Arrival Time*) por instancia / clave de límite,
//! actualizado de forma atómica y sin bloqueos (*lock-free*).
//!
//! # Invariantes y Arquitectura
//! - **Cero dependencias de infraestructura/transporte**: Opera únicamente sobre una clave
//!   abstracta de admisión (`&str` / `String`) y tipos de `std`.
//! - **Acceso atómico sin cerrojos**: El estado del TAT es un [`std::sync::atomic::AtomicU64`]
//!   actualizado mediante bucle CAS (*compare-and-swap*).
//! - **Fuente de tiempo inyectable**: Permite desacoplar el tiempo de pared y simular el avance
//!   temporal de forma determinista mediante el trait [`Reloj`].

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Fuente de tiempo inyectable para el cálculo del GCRA.
pub trait Reloj: Send + Sync {
    /// Devuelve los nanosegundos transcurridos desde un punto de referencia monotónico.
    fn ahora_nanos(&self) -> u64;
}

/// Reloj predeterminado basado en [`Instant`] del sistema.
#[derive(Clone, Debug)]
pub struct RelojDelSistema {
    inicio: Instant,
}

impl RelojDelSistema {
    /// Crea una nueva instancia de [`RelojDelSistema`] fijando el instante de inicio.
    pub fn nuevo() -> Self {
        Self {
            inicio: Instant::now(),
        }
    }
}

impl Default for RelojDelSistema {
    fn default() -> Self {
        Self::nuevo()
    }
}

impl Reloj for RelojDelSistema {
    fn ahora_nanos(&self) -> u64 {
        Instant::now().duration_since(self.inicio).as_nanos() as u64
    }
}

/// Reloj determinista para pruebas unitarias.
#[derive(Clone, Debug)]
pub struct RelojDePrueba {
    nanos: Arc<AtomicU64>,
}

impl RelojDePrueba {
    /// Crea un nuevo [`RelojDePrueba`] inicializado en el tiempo cero o el valor dado.
    pub fn nuevo(nanos_iniciales: u64) -> Self {
        Self {
            nanos: Arc::new(AtomicU64::new(nanos_iniciales)),
        }
    }

    /// Avanza el reloj de prueba en los nanosegundos indicados.
    pub fn avanzar_nanos(&self, delta_nanos: u64) {
        self.nanos.fetch_add(delta_nanos, Ordering::Relaxed);
    }

    /// Fija el reloj de prueba en un instante absoluto en nanosegundos.
    pub fn fijar_nanos(&self, nanos: u64) {
        self.nanos.store(nanos, Ordering::Relaxed);
    }
}

impl Reloj for RelojDePrueba {
    fn ahora_nanos(&self) -> u64 {
        self.nanos.load(Ordering::Relaxed)
    }
}

/// Error al validar la configuración de GCRA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeConfiguracionGcra {
    /// La tasa sostenida debe ser finita y estrictamente mayor que cero.
    TasaInvalida,
}

impl fmt::Display for ErrorDeConfiguracionGcra {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TasaInvalida => write!(
                f,
                "La tasa sostenida debe ser finita y estrictamente mayor a cero"
            ),
        }
    }
}

impl std::error::Error for ErrorDeConfiguracionGcra {}

/// Configuración de límites para el algoritmo GCRA.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfiguracionGcra {
    tasa_sostenida_por_segundo: f64,
    tolerancia_rafaga: u32,
    intervalo_emision_nanos: u64,
    ventana_tolerancia_nanos: u64,
}

impl ConfiguracionGcra {
    /// Crea una nueva configuración validando que la tasa sostenida sea válida.
    pub fn nueva(
        tasa_sostenida_por_segundo: f64,
        tolerancia_rafaga: u32,
    ) -> Result<Self, ErrorDeConfiguracionGcra> {
        if !tasa_sostenida_por_segundo.is_finite() || tasa_sostenida_por_segundo <= 0.0 {
            return Err(ErrorDeConfiguracionGcra::TasaInvalida);
        }

        let intervalo_emision_nanos = (1_000_000_000.0 / tasa_sostenida_por_segundo).round() as u64;
        let ventana_tolerancia_nanos = (tolerancia_rafaga as u64) * intervalo_emision_nanos;

        Ok(Self {
            tasa_sostenida_por_segundo,
            tolerancia_rafaga,
            intervalo_emision_nanos,
            ventana_tolerancia_nanos,
        })
    }

    /// Obtiene la tasa sostenida en peticiones por segundo.
    pub fn tasa_sostenida_por_segundo(&self) -> f64 {
        self.tasa_sostenida_por_segundo
    }

    /// Obtiene la tolerancia a ráfagas en número de peticiones extra.
    pub fn tolerancia_rafaga(&self) -> u32 {
        self.tolerancia_rafaga
    }

    /// Intervalo de emisión $T = 1 / \text{tasa}$ expresado en nanosegundos.
    pub fn intervalo_emision_nanos(&self) -> u64 {
        self.intervalo_emision_nanos
    }

    /// Ventana de tolerancia a ráfagas $\tau = \text{tolerancia} \times T$ en nanosegundos.
    pub fn ventana_tolerancia_nanos(&self) -> u64 {
        self.ventana_tolerancia_nanos
    }
}

/// Valores predeterminados provisionales para una conversación individual uno a uno.
///
/// Nota: Estos valores son provisionales para pruebas y desarrollo por omisión; la
/// parametrización definitiva por variables de entorno y su ADR corresponden a la tarea 3 de la etapa A-4.
impl Default for ConfiguracionGcra {
    fn default() -> Self {
        // Tasa sostenida por omisión: 0.5 peticiones/seg (1 cada 2 seg), tolerancia a ráfaga de 3 extra.
        Self::nueva(0.5, 3).expect("La configuración por omisión debe ser válida")
    }
}

/// Motivo por el cual una petición de admisión fue descartada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MotivoDescarte {
    /// La tasa sostenida o presupuesto de ráfaga para la clave ha sido superado.
    TasaSostenidaExcedida,
}

impl fmt::Display for MotivoDescarte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TasaSostenidaExcedida => {
                write!(f, "Tasa sostenida o límite de ráfaga superado")
            }
        }
    }
}

/// Resultado de evaluar una petición de admisión.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResultadoDeAdmision {
    /// Petición admitida dentro del presupuesto de tasa/ráfaga.
    Admitido,
    /// Petición descartada con la clave correspondiente y el motivo.
    Descartado {
        clave: String,
        motivo: MotivoDescarte,
    },
}

/// Instancia de control de admisión GCRA para una única clave límite.
#[derive(Debug)]
pub struct Gcra<R: Reloj = RelojDelSistema> {
    clave: String,
    configuracion: ConfiguracionGcra,
    tat: AtomicU64,
    reloj: R,
}

impl Gcra<RelojDelSistema> {
    /// Crea un nuevo limitador GCRA para la clave y configuración dadas usando el reloj del sistema.
    pub fn nueva(clave: impl Into<String>, configuracion: ConfiguracionGcra) -> Self {
        Self::con_reloj(clave, configuracion, RelojDelSistema::nuevo())
    }
}

impl<R: Reloj> Gcra<R> {
    /// Crea un nuevo limitador GCRA inyectando un reloj personalizado.
    pub fn con_reloj(clave: impl Into<String>, configuracion: ConfiguracionGcra, reloj: R) -> Self {
        Self {
            clave: clave.into(),
            configuracion,
            tat: AtomicU64::new(0),
            reloj,
        }
    }

    /// Retorna la clave límite de esta instancia.
    pub fn clave(&self) -> &str {
        &self.clave
    }

    /// Retorna la configuración asociada a esta instancia.
    pub fn configuracion(&self) -> &ConfiguracionGcra {
        &self.configuracion
    }

    /// Evalúa la admisión de una petición de manera atómica y libre de bloqueos (*lock-free*).
    pub fn admitir(&self) -> ResultadoDeAdmision {
        let ahora = self.reloj.ahora_nanos();
        let i = self.configuracion.intervalo_emision_nanos();
        let tau = self.configuracion.ventana_tolerancia_nanos();

        let mut tat_actual = self.tat.load(Ordering::Relaxed);

        loop {
            let tat_base = if tat_actual < ahora {
                ahora
            } else {
                tat_actual
            };
            let nuevo_tat = tat_base.saturating_add(i);

            if nuevo_tat > ahora.saturating_add(tau).saturating_add(i) {
                return ResultadoDeAdmision::Descartado {
                    clave: self.clave.clone(),
                    motivo: MotivoDescarte::TasaSostenidaExcedida,
                };
            }

            match self.tat.compare_exchange_weak(
                tat_actual,
                nuevo_tat,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return ResultadoDeAdmision::Admitido,
                Err(observado) => tat_actual = observado,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac_1_control_de_tasa_sostenida_sin_rafaga() {
        // Tasa de 1 request por segundo (intervalo = 1_000_000_000 nanos), ráfaga 0.
        let config = ConfiguracionGcra::nueva(1.0, 0).expect("configuración válida");
        let reloj = RelojDePrueba::nuevo(1_000_000);
        let gcra = Gcra::con_reloj("contacto_1", config, reloj.clone());

        // Primera llamada: admitida (TAT pasa a 1_000_000 + 1_000_000_000)
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);

        // Segunda llamada inmediata en el mismo instante: descartada por exceder tasa sostenida
        let res_descarte = gcra.admitir();
        assert_eq!(
            res_descarte,
            ResultadoDeAdmision::Descartado {
                clave: "contacto_1".to_string(),
                motivo: MotivoDescarte::TasaSostenidaExcedida
            }
        );

        // Avanzar el reloj menos del intervalo (500 ms): sigue descartada
        reloj.avanzar_nanos(500_000_000);
        assert_eq!(
            gcra.admitir(),
            ResultadoDeAdmision::Descartado {
                clave: "contacto_1".to_string(),
                motivo: MotivoDescarte::TasaSostenidaExcedida
            }
        );

        // Avanzar el resto hasta cumplir el intervalo completo (otros 500 ms): admitida
        reloj.avanzar_nanos(500_000_000);
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);
    }

    #[test]
    fn ac_2_tolerancia_a_rafaga_exacta() {
        // Tasa de 1 request/seg, ráfaga N = 2 extra (permite N+1 = 3 peticiones seguidas).
        let config = ConfiguracionGcra::nueva(1.0, 2).expect("configuración válida");
        let reloj = RelojDePrueba::nuevo(0);
        let gcra = Gcra::con_reloj("contacto_2", config, reloj);

        // Las primeras N+1 = 3 llamadas deben ser admitidas
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);

        // La cuarta llamada excede la tolerancia a ráfagas y debe descartarse
        assert_eq!(
            gcra.admitir(),
            ResultadoDeAdmision::Descartado {
                clave: "contacto_2".to_string(),
                motivo: MotivoDescarte::TasaSostenidaExcedida
            }
        );
    }

    #[test]
    fn ac_3_perfil_conversacional_realista_cero_falsos_positivos() {
        // Configuración por omisión: 0.5 req/seg (1 msg cada 2 seg), ráfaga de 3 extra.
        let config = ConfiguracionGcra::default();
        let reloj = RelojDePrueba::nuevo(0);
        let gcra = Gcra::con_reloj("conversacion_123", config, reloj.clone());

        // Simulación de interacción conversacional legítima:
        // 1. Mensaje inicial + repetición rápida (ráfaga legítima de 2 mensajes)
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);
        reloj.avanzar_nanos(100_000_000); // 100 ms después
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);

        // 2. Pausa de lectura de la respuesta (5 segundos)
        reloj.avanzar_nanos(5_000_000_000);
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);

        // 3. Pausa conversacional (10 segundos)
        reloj.avanzar_nanos(10_000_000_000);
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);

        // 4. Otro mensaje tras 3 segundos
        reloj.avanzar_nanos(3_000_000_000);
        assert_eq!(gcra.admitir(), ResultadoDeAdmision::Admitido);
    }
}
