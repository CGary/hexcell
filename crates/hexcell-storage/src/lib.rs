//! Capa de persistencia de una célula: acceso a SQLite y gestión de pools.
//!
//! Esqueleto de la etapa A-1: compila vacío. El contenido real llega en la etapa A-2, que es la
//! que define la persistencia dual de FR-05 —`sessions.db` en lectura y escritura caliente,
//! `knowledge_live.db` en solo lectura— y en la A-5, que añade la conmutación atómica por épocas
//! de FR-07.
//!
//! Este crate existe separado del núcleo desde el primer día, y no como un módulo de
//! `hexcell-core`, precisamente para que la tabla de dependencias del núcleo pueda quedarse
//! vacía y verificable. El motivo completo está en `docs/adr/adr-0002-estructura-workspace.md`.
//!
//! Regla que hereda de `adr-0010` y que condiciona su esquema futuro: `sessions.db` **nunca**
//! almacena identificadores de transporte crudos. El mapeo de identidad y su almacén son
//! propiedad del adaptador de canal, no de esta capa.
