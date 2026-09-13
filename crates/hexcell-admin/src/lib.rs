//! Cara de biblioteca del binario `hexcell-admin`, la CLI central de administración.
//!
//! Este crate es, ante todo, un binario (`src/main.rs`): el proceso que el operador invoca para
//! gobernar las células del servidor. Tiene además un objetivo de biblioteca — este archivo — cuya
//! razón de ser es dejar que sus módulos, como `docker` y `estado_de_celula`, se ejerciten desde
//! `crates/hexcell-admin/tests/` con la API pública normal, sin que ese código de test tenga que
//! vivir como módulo `#[cfg(test)]` dentro de los mismos archivos que lo implementan.
//!
//! `main.rs` todavía no llama a [`docker::ClienteDocker`]: el esqueleto de la CLI que lo haría es
//! la tarea 10 de la etapa A-6, fuera del alcance de esta tarea, y conectarlo aquí sería ampliar
//! el alcance sin ningún comportamiento que ejercitar de extremo a extremo.

pub mod docker;
pub mod estado_de_celula;
