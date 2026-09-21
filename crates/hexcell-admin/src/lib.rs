//! Cara de biblioteca del binario `hexcell-admin`, la CLI central de administración.
//!
//! Este crate es, ante todo, un binario (`src/main.rs`): el proceso que el operador invoca para
//! gobernar las células del servidor. Tiene además un objetivo de biblioteca — este archivo — cuya
//! razón de ser es dejar que sus módulos, como `docker`, `estado_de_celula`, `codigo_de_salida`,
//! `salida`, `argumentos` y `comandos`, se ejerciten desde `crates/hexcell-admin/tests/` con la
//! API pública normal, sin que ese código de test tenga que vivir como módulo `#[cfg(test)]`
//! dentro de los mismos archivos que lo implementan.
//!
//! `main.rs` recoge los argumentos del proceso, los pasa al analizador de `argumentos`, construye
//! el `Salida` de producción y los entrega a `comandos::ejecutar`, que devuelve el
//! `CodigoDeSalida` que el proceso devuelve al sistema operativo a través de
//! `std::process::ExitCode`.

pub mod argumentos;
pub mod ciclo_de_vida;
pub mod codigo_de_salida;
pub mod comandos;
pub mod docker;
pub mod esquema_configuracion;
pub mod estado_de_celula;
pub mod renderizado_configuracion;
pub mod salida;
