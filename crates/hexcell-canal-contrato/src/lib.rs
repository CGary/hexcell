//! Batería reutilizable de tests de contrato para el puerto `ChannelAdapter`.
//!
//! Este crate existe para una sola cosa: alojar la especificación, en forma de tests, que
//! **cualquier** implementación de `ChannelAdapter` (`hexcell_core::canal`) debe cumplir. No
//! nombra ningún adaptador concreto en ninguno de sus archivos — ni el simulado de
//! `hexcell-canal-simulado`, ni el futuro adaptador de la Cloud API real — porque la etapa B-1
//! debe poder ejercitar esta misma batería contra ese adaptador real sin reescribir una sola
//! línea de aquí, que es exactamente el argumento que justifica su existencia como crate propio
//! en vez de como módulo de tests dentro del simulado.
//!
//! # La distinción TIPO/POLÍTICA
//!
//! El puerto se abstrae hacia el caso más restrictivo (la Meta Cloud API), pero **el TIPO admite
//! el resultado restrictivo; la POLÍTICA de cada adaptador decide si lo produce**. Que esta
//! batería ejercite una ventana de servicio de 24 horas contra un adaptador que la imite no dice
//! nada sobre lo que debe hacer el adaptador del canal propio: ese adaptador no tiene por qué
//! imponer ninguna ventana, porque su transporte no la impone. Esta batería no es una prueba de
//! que el canal propio deba parecerse a la Cloud API; es una prueba de que **cualquier**
//! adaptador que sí produzca esos resultados los produce de forma coherente con el tipo.
//!
//! # Por qué existe un segundo trait (`banco`)
//!
//! `ChannelAdapter` declara solo `send` y `estado_ventana` (FR-12); forzar un rechazo concreto,
//! inyectar un evento entrante o avanzar el reloj son operaciones de **control** que no
//! pertenecen al puerto de producción y que ensuciarían el tipo del dominio si se añadieran allí
//! solo para que los tests puedan pedirlas. El módulo [`banco`] declara ese segundo trait —el
//! arnés de pruebas— y el módulo [`bateria`] lo consume: cada escenario es una función genérica
//! sobre ese arnés, nunca sobre un tipo concreto.

pub mod banco;
pub mod bateria;

pub use banco::{BancoConRelojControlable, BancoDeCanal};
pub use bateria::verificar_contrato_del_puerto;
