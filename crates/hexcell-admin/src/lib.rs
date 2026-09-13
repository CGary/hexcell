//! Cara de biblioteca del binario `hexcell-admin`, la CLI central de administración.
//!
//! Este crate es, ante todo, un binario (`src/main.rs`): el proceso que el operador invoca para
//! gobernar las células del servidor. Tiene además un objetivo de biblioteca — este archivo — cuya
//! razón de ser es dejar que sus módulos, como `docker`, `estado_de_celula`, `codigo_de_salida` y
//! `salida`, se ejerciten desde `crates/hexcell-admin/tests/` con la API pública normal, sin que
//! ese código de test tenga que vivir como módulo `#[cfg(test)]` dentro de los mismos archivos que
//! lo implementan.
//!
//! `main.rs` todavía no llama a [`docker::ClienteDocker`] ni construye un
//! [`codigo_de_salida::CodigoDeSalida`] o un [`salida::Salida`]: el analizador de argumentos, los
//! seis subcomandos, el modo de simulación y el cableado de `main.rs` son la tarea hermana
//! HEX-074-c, fuera del alcance de esta tarea, y conectarlos aquí sería ampliar el alcance sin
//! ningún comportamiento nuevo que ejercitar de extremo a extremo.

pub mod codigo_de_salida;
pub mod docker;
pub mod estado_de_celula;
pub mod salida;
