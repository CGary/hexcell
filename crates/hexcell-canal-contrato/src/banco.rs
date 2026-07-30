//! Arnés de pruebas (`BancoDeCanal`): el segundo trait que hace posible una batería reutilizable.
//!
//! `ChannelAdapter` (`hexcell_core::canal`) declara únicamente `send` y `estado_ventana`, que son
//! los dos elementos de FR-12 que un consumidor de producción necesita. Pero un test de contrato
//! necesita además **control**: inyectar un evento entrante para abrir la ventana de servicio de
//! una conversación, y forzar que la próxima llamada a `send` devuelva un `ResultadoEnvio`
//! concreto. Ninguna de las dos cosas es un elemento de FR-12, así que añadirlas al puerto
//! contaminaría el tipo del dominio con ganchos de test que ningún adaptador de producción
//! necesita ofrecer a su consumidor real.
//!
//! `BancoDeCanal` traslada ese control a un segundo trait, implementado no por el adaptador sino
//! por un tipo auxiliar de cada crate consumidor (aquí, `crates/hexcell-canal-simulado/tests/`; en
//! la etapa B-1, un tipo equivalente que sepa hablar con la Cloud API de pruebas o con un sandbox
//! HTTP). La batería de [`crate::bateria`] es genérica sobre `B: BancoDeCanal`, nunca sobre un
//! adaptador concreto.

use std::time::Duration;

use hexcell_core::canal::ChannelAdapter;
use hexcell_core::identidad::IdConversacion;

/// Arnés mínimo que cualquier consumidor de la batería debe implementar.
///
/// `type Adaptador` fija, para cada implementación del arnés, cuál es el `ChannelAdapter`
/// concreto bajo prueba; la batería nunca lo nombra ella misma.
pub trait BancoDeCanal {
    /// El adaptador concreto bajo prueba en este banco.
    type Adaptador: ChannelAdapter;

    /// Vista prestada del adaptador bajo prueba, para llamar a `send`/`estado_ventana`.
    fn adaptador(&self) -> &Self::Adaptador;

    /// Inyecta un evento entrante para la conversación dada, ancla su ventana de servicio en el
    /// instante actual del banco y lo entrega por el mecanismo de entrega propio del adaptador.
    ///
    /// La batería no necesita observar el evento entregado: lo que le importa es su efecto
    /// posterior sobre `send`/`estado_ventana` para esa misma conversación.
    fn inyectar_evento(&self, conversacion: &IdConversacion) -> impl Future<Output = ()> + Send;

    /// Fuerza que la próxima llamada a `send` sobre esa conversación devuelva `resultado`,
    /// consumiéndose una sola vez, sin afectar a ninguna otra llamada.
    fn forzar_resultado(
        &self,
        conversacion: &IdConversacion,
        resultado: hexcell_core::canal::ResultadoEnvio,
    );
}

/// Sub-trait opcional para los bancos que sí pueden controlar su fuente de tiempo.
///
/// Deliberadamente separado de [`BancoDeCanal`]: un futuro banco contra un adaptador real que no
/// pueda avanzar su reloj —por ejemplo, uno que hable con un sandbox HTTP ajeno— sigue
/// implementando el trait base íntegro y ejecuta todos los escenarios no temporales de la
/// batería; solo se queda fuera de los dos escenarios que dependen de expirar la ventana de
/// servicio de verdad.
pub trait BancoConRelojControlable: BancoDeCanal {
    /// Avanza la fuente de tiempo del banco la duración dada, sin tocar el reloj de pared.
    fn avanzar(&self, duracion: Duration);
}
