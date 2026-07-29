//! Reloj inyectable para el adaptador simulado.
//!
//! # Por qué `tokio::time::pause()` no sirve aquí
//!
//! El puerto `ChannelAdapter` de `hexcell-core` construye su ventana de servicio sobre
//! `std::time::SystemTime`: `EventoEntrante::marca_temporal` y
//! `EstadoVentanaServicio::Abierta::expira_en` son ambos `SystemTime`, no `tokio::time::Instant`.
//! `tokio::time::pause()` virtualiza únicamente el reloj de `tokio::time` — `Instant::now()` y
//! `sleep` dentro del runtime pausado — y nunca toca `SystemTime::now()`, que sigue leyendo el
//! reloj de pared del sistema operativo pase lo que pase con el runtime. Un test que quisiera
//! expirar la ventana de 24 horas con `tokio::time::pause()` tendría que esperar 24 horas reales
//! de reloj de pared, porque `SystemTime` no es lo que esa función controla.
//!
//! Por eso el adaptador simulado no llama nunca directamente al reloj de pared: recibe un
//! [`Reloj`] inyectado, y el único punto del crate donde puede aparecer una lectura del reloj real
//! es [`RelojDelSistema`], en este mismo archivo.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Fuente de tiempo del adaptador simulado, sustituible en los tests.
///
/// Se declara compatible con objetos de trait (`dyn Reloj`) a propósito: el adaptador guarda un
/// `Arc<dyn Reloj + Send + Sync>` para no arrastrar un parámetro genérico de reloj a cada firma
/// que lo consume.
pub trait Reloj {
    /// Devuelve el instante actual según esta fuente de tiempo.
    fn ahora(&self) -> SystemTime;
}

/// Reloj real de pared. Único lugar del crate donde se permite `SystemTime::now()`.
///
/// Se usa cuando el adaptador simulado se levanta fuera de un test (por ejemplo, en el binario
/// `hexcell` mientras no existe todavía un canal real que sustituirlo); en los tests de la
/// ventana de 24 horas se sustituye siempre por [`RelojDePrueba`].
#[derive(Clone, Copy, Debug, Default)]
pub struct RelojDelSistema;

impl Reloj for RelojDelSistema {
    fn ahora(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Reloj controlable desde el test: no avanza solo, avanza cuando el test se lo pide.
///
/// Guarda el instante actual detrás de un `Mutex` con mutabilidad interior para que
/// `avanzar`/`fijar` funcionen a través de una referencia compartida (`&self`), que es la forma
/// en que el adaptador simulado lo mantiene junto al resto de su estado.
#[derive(Clone)]
pub struct RelojDePrueba {
    instante: Arc<Mutex<SystemTime>>,
}

impl RelojDePrueba {
    /// Crea el reloj de prueba fijado en el instante dado.
    pub fn nuevo(instante_inicial: SystemTime) -> Self {
        Self {
            instante: Arc::new(Mutex::new(instante_inicial)),
        }
    }

    /// Avanza el reloj de prueba la duración dada, sin tocar el reloj de pared.
    pub fn avanzar(&self, duracion: Duration) {
        let mut instante = self.instante.lock().expect(
            "el mutex interno de RelojDePrueba no debería estar envenenado en un proceso de test",
        );
        *instante += duracion;
    }

    /// Fija el reloj de prueba en un instante concreto.
    pub fn fijar(&self, instante_nuevo: SystemTime) {
        let mut instante = self.instante.lock().expect(
            "el mutex interno de RelojDePrueba no debería estar envenenado en un proceso de test",
        );
        *instante = instante_nuevo;
    }
}

impl Reloj for RelojDePrueba {
    fn ahora(&self) -> SystemTime {
        *self.instante.lock().expect(
            "el mutex interno de RelojDePrueba no debería estar envenenado en un proceso de test",
        )
    }
}
