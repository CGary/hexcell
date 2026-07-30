//! Puerto de inferencia LLM `ProveedorDeInferencia`: la frontera entre el motor y el proveedor.
//!
//! Igual que `crate::canal` es la frontera de coexistencia entre el núcleo y el transporte de
//! WhatsApp, este módulo es la frontera entre el motor de mensajería y quien de verdad genera una
//! respuesta. La inferencia es 100 % externa, sobre proveedores comerciales de terceros
//! (`adr-0012`), y el núcleo no sabe nada de ninguno de ellos: solo declara la operación que
//! cualquiera debe cumplir. Sumar un proveedor real en la etapa A-4 es escribir un adaptador de
//! este trait, no reabrir el motor ni esta declaración.
//!
//! # Qué NO lleva esta versión, y por qué
//!
//! `PeticionDeInferencia` y `RespuestaDeInferencia` llevan solo lo mínimo que el motor consume hoy:
//! la conversación y el contenido de entrada, y el texto de respuesta. Ningún recuento de tokens,
//! coste, nombre de modelo, temperatura ni variante de streaming: escribir esa firma por
//! adelantado, como mitigación de compatibilidad, es exactamente D-09 en
//! `docs/bitacora-de-descartes.md` — una firma que compila no garantiza que la etapa A-4, que trae
//! la contabilidad financiera de dos fases, pueda envolver el proveedor sin tocar lo que el motor
//! consume; para eso basta con que `generar` conserve su firma, y añadir un campo a
//! `RespuestaDeInferencia` cuando exista una respuesta real que modelar no la cambia.
//!
//! # Por qué el método se declara `-> impl Future` y no `async fn`
//!
//! La misma razón ya escrita en `crate::canal` para `ChannelAdapter`: sobre rustc 1.92.0, `async
//! fn` dentro de un trait dispara el aviso `async_fn_in_trait`, activo por omisión, que `cargo
//! clippy --workspace -- -D warnings` convierte en error. Escribir el retorno como `impl
//! Future<Output = ...> + Send` evita el aviso sin silenciarlo, y permite declarar hoy la cota
//! `Send` que el consumidor asíncrono necesita para lanzar la tarea. La consecuencia es la misma
//! que para `ChannelAdapter`: el trait no es compatible con objetos de trait, así que el motor lo
//! consume genérico, nunca como `Box<dyn ProveedorDeInferencia>`.

use crate::identidad::IdConversacion;

/// Petición de inferencia: lo mínimo que un proveedor necesita para generar una respuesta.
///
/// El contenido ya llega normalizado por el adaptador de canal, igual que
/// `EventoEntrante::contenido`; este tipo no repite esa normalización, solo la transporta hasta el
/// proveedor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeticionDeInferencia {
    /// Conversación a la que pertenece la petición, para que un proveedor con memoria de hilo
    /// pueda usarla; el simulado de esta tarea la ignora.
    pub conversacion: IdConversacion,
    /// Contenido normalizado del evento entrante que motiva la petición.
    pub contenido: String,
}

/// Respuesta de inferencia: el texto que el motor envía como réplica.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RespuestaDeInferencia {
    /// Texto de la respuesta generada.
    pub contenido: String,
}

/// Puerto de inferencia: todo proveedor LLM se implementa detrás de este trait.
///
/// El tipo asociado [`ProveedorDeInferencia::Error`] transporta averías de transporte hacia el
/// proveedor —igual que [`crate::canal::ChannelAdapter::Error`]—, nunca una decisión de producto:
/// qué contesta la célula cuando la inferencia falla es una decisión pendiente ligada al modo
/// degradado de la etapa A-4 (FR-10), no algo que este puerto resuelva.
pub trait ProveedorDeInferencia {
    /// Avería del proveedor: la llamada de red falló, la respuesta no se pudo interpretar, etc.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Genera una respuesta a partir de una petición ya normalizada.
    fn generar(
        &self,
        peticion: PeticionDeInferencia,
    ) -> impl Future<Output = Result<RespuestaDeInferencia, Self::Error>> + Send;
}
