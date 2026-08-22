//! Núcleo de dominio de HexCell.
//!
//! Este crate contiene los tipos que el producto entiende y **ninguna dependencia de
//! infraestructura**: no hay almacenamiento, ni transporte, ni motor de ejecución asíncrona, ni
//! cliente HTTP. Su tabla de dependencias está vacía a propósito y es un criterio de aceptación,
//! porque una frontera que se sostiene por disciplina se cruza el primer día que corre prisa.
//!
//! En la etapa A-1 alberga la declaración del puerto de canal `ChannelAdapter` (FR-12), que es la
//! **frontera de coexistencia** entre el núcleo y el transporte de WhatsApp: dos adaptadores
//! vivos a la vez en células distintas del mismo servidor, sin que el núcleo sepa cuál está
//! debajo. El porqué de esa frontera está en `docs/adr/adr-0010-puerto-de-canal.md`; el porqué de
//! la división en crates, en `docs/adr/adr-0002-estructura-workspace.md`.

pub mod admision;
pub mod canal;
pub mod identidad;
pub mod inferencia;
