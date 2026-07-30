//! Registro de deduplicación: idempotencia de entrega de eventos entrantes.
//!
//! **Ya no vive en memoria.** Desde HEX-006, `sessions.db` es la única fuente de verdad del
//! conjunto de identificadores ya procesados: este tipo es una fachada delgada sobre
//! `hexcell_storage::RepositorioDeSesiones` que recuerda la ventana de retención configurada y no
//! guarda ningún mapa propio. No queda ninguna caché delante de la base a propósito: dos fuentes
//! de verdad para el mismo conjunto es exactamente cómo un reinicio acaba en desacuerdo consigo
//! mismo sin que nadie lo note.
//!
//! # La ventana de retención
//!
//! [`VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO`] es el valor por defecto, no un valor
//! definitivo: **la cifra definitiva de esta ventana es una decisión de producto todavía
//! abierta**, registrada como entrada `Pendiente` en `docs/STATUS.md` con fecha 2026-07-30. Una
//! hora cubre, con margen amplio, los dos patrones de reentrega normales de un canal de
//! mensajería: el reintento inmediato de una entrega no confirmada, y la reentrega de lo que
//! quedó pendiente cuando el transporte se reconectó — ambos casos suelen resolverse en minutos,
//! no en horas. La ventana es, además, un **parámetro del constructor** de
//! [`RegistroDeDeduplicacion`] y no una constante fija dentro de él, precisamente porque el valor
//! definitivo sigue abierto: `crates/hexcell/src/configuracion.rs` la hace configurable por
//! variable de entorno siguiendo el precedente de `HEXCELL_CAPACIDAD_COLA`, y este módulo no debe
//! restatear el número en dos sitios.
//!
//! # Por qué el registro no tiene reloj propio
//!
//! El registro nunca lee la hora del sistema: poda contra el máximo `marca_temporal` visto hasta
//! ahora en el propio flujo de eventos, que le llega como parámetro en cada llamada a `procesar`.
//! Ese máximo —el horizonte— pasó a vivir en la tabla `estado_del_motor` de `sessions.db` y avanza
//! de forma monótona también entre reinicios, así que la semántica que fijó HEX-005 no cambia:
//! sigue midiéndose en tiempo del **canal** y no en tiempo de pared, con la misma consecuencia
//! aceptada a sabiendas —un adaptador que entregase marcas temporales muy desordenadas podaría
//! antes de lo previsto—. Este crate tampoco importa el trait de tiempo inyectable del crate de
//! test-double (`hexcell-canal-simulado`) para nada relacionado con el tiempo de producción.
//!
//! # AC-9: un duplicado que llega fuera de la ventana
//!
//! El comportamiento no se inventa en esta tarea: la tabla de riesgos de
//! `docs/plan/fase-a-2-nucleo-persistencia.md` ya lo fija — se procesa como evento **nuevo**,
//! duplicando el trabajo conversacional, como limitación residual aceptada y documentada. Este
//! módulo no rechaza ese caso, no hace `panic!` y no inventa un tercer camino: simplemente, si la
//! entrada ya fue podada por antigua, el identificador vuelve a parecer nuevo.
//!
//! El tope duro de entradas retenidas vive ahora junto al SQL que lo aplica, en
//! `hexcell_storage::LIMITE_DE_ENTRADAS_RETENIDAS`, con su valor sin cambios.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hexcell_core::identidad::IdDeduplicacion;
use hexcell_storage::{ErrorDeAlmacen, RepositorioDeSesiones};

pub use hexcell_storage::VeredictoDeDeduplicacion;

/// Ventana de retención por defecto del registro de deduplicación: una hora.
///
/// Justificación funcional, sin nombrar ningún proveedor concreto: la reentrega normal de un
/// canal de mensajería es o bien un reintento inmediato de una entrega no confirmada, o bien la
/// repetición de lo que quedó pendiente cuando el transporte se reconectó, y ambos casos aterrizan
/// en minutos. Una hora cubre con margen amplio un reinicio o un ciclo completo de reintentos sin
/// dejar crecer la tabla sin necesidad. **La cifra definitiva sigue siendo una decisión de
/// producto abierta** (`docs/STATUS.md`, entrada `Pendiente` del 2026-07-30): este valor es el
/// que se usa mientras esa decisión no se tome, no un número ya cerrado.
pub const VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO: Duration = Duration::from_secs(60 * 60);

/// Fachada del registro de deduplicación respaldado por `sessions.db`.
pub struct RegistroDeDeduplicacion {
    repositorio: Arc<RepositorioDeSesiones>,
    /// Ventana de retención con la que se construyó este registro.
    ventana: Duration,
}

impl RegistroDeDeduplicacion {
    /// Construye el registro sobre el repositorio de sesiones y con la ventana de retención dada.
    ///
    /// La ventana es un parámetro y no una constante interna: el valor por defecto vive en
    /// [`VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO`] y quien construye el registro (hoy,
    /// `crates/hexcell/src/main.rs` a partir de `Configuracion::ventana_deduplicacion`) decide si
    /// usa ese valor o uno configurado explícitamente.
    pub fn nuevo(repositorio: Arc<RepositorioDeSesiones>, ventana: Duration) -> Self {
        Self {
            repositorio,
            ventana,
        }
    }

    /// Procesa un identificador de deduplicación llegado con la marca temporal dada.
    ///
    /// Delega en una única transacción de `sessions.db` que avanza el horizonte monótono, poda por
    /// antigüedad y por el tope duro, e inserta el identificador si no estaba. Devuelve `Err`
    /// cuando la persistencia falla; qué hacer con ese error es política del motor y está
    /// documentada allí, no aquí: esta fachada no decide por el negocio del cliente.
    pub fn procesar(
        &mut self,
        id: IdDeduplicacion,
        marca_temporal: SystemTime,
    ) -> Result<VeredictoDeDeduplicacion, ErrorDeAlmacen> {
        self.repositorio
            .procesar_deduplicacion(&id, marca_temporal, self.ventana)
    }
}
