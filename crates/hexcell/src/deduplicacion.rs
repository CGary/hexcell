//! Registro de deduplicación: idempotencia de entrega de eventos entrantes.
//!
//! **Estructura TRANSITORIA.** Vive en memoria porque `sessions.db` no existe todavía —llega en
//! HEX-006, que es quien la respaldará con persistencia real—. No hay aquí ningún esquema, ninguna
//! migración y ninguna sentencia SQL: esta tarea no inventa la persistencia que HEX-006 diseñará.
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
//! El registro nunca lee un reloj: poda contra el máximo `marca_temporal` visto hasta ahora en el
//! propio flujo de eventos, que le llega como parámetro en cada llamada a `procesar`. Esto compra
//! dos cosas: el motor no gana ninguna dependencia de reloj, y —más importante— este crate nunca
//! importa el trait de reloj inyectable, ni `RelojDePrueba` ni `RelojDelSistema`, del crate de
//! test-double (`hexcell-canal-simulado`) para nada relacionado con el tiempo de producción. La
//! consecuencia
//! que se acepta a sabiendas: la retención se mide en tiempo del **canal**, no en tiempo de pared,
//! así que un adaptador que entregase marcas temporales muy desordenadas podaría antes de lo
//! previsto. Documentado aquí; HEX-006 lo revisita cuando el registro gane persistencia.
//!
//! # AC-9: un duplicado que llega fuera de la ventana
//!
//! El comportamiento no se inventa en esta tarea: la tabla de riesgos de
//! `docs/plan/fase-a-2-nucleo-persistencia.md` ya lo fija — se procesa como evento **nuevo**,
//! duplicando el trabajo conversacional, como limitación residual aceptada y documentada. Este
//! módulo no rechaza ese caso, no hace `panic!` y no inventa un tercer camino: simplemente, si la
//! entrada ya fue podada por antigua, el identificador vuelve a parecer nuevo.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use hexcell_core::identidad::IdDeduplicacion;

/// Ventana de retención por defecto del registro de deduplicación: una hora.
///
/// Justificación funcional, sin nombrar ningún proveedor concreto: la reentrega normal de un
/// canal de mensajería es o bien un reintento inmediato de una entrega no confirmada, o bien la
/// repetición de lo que quedó pendiente cuando el transporte se reconectó, y ambos casos aterrizan
/// en minutos. Una hora cubre con margen amplio un reinicio o un ciclo completo de reintentos sin
/// dejar crecer el mapa sin necesidad. **La cifra definitiva sigue siendo una decisión de producto
/// abierta** (`docs/STATUS.md`, entrada `Pendiente` del 2026-07-30): este valor es el
/// que se usa mientras esa decisión no se tome, no un número ya cerrado.
pub const VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO: Duration = Duration::from_secs(60 * 60);

/// Tope duro de identificadores retenidos, sea cual sea la ventana configurada.
///
/// Protege el presupuesto de memoria de NFR-01 frente a una ráfaga: sin este tope, una ráfaga de
/// entregas con identificadores distintos podría crecer el mapa sin límite dentro de la propia
/// ventana, antes de que la poda por antigüedad tuviera ocasión de actuar.
const LIMITE_DE_ENTRADAS_RETENIDAS: usize = 10_000;

/// Veredicto de `RegistroDeDeduplicacion::procesar` sobre un identificador de deduplicación.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VeredictoDeDeduplicacion {
    /// No se había visto antes dentro de la ventana de retención vigente: se debe procesar.
    Nuevo,
    /// Ya se vio antes, dentro de la ventana de retención vigente: se debe descartar.
    Duplicado,
}

/// Registro en memoria de identificadores de deduplicación ya procesados.
///
/// Estructura **transitoria**: HEX-006 la respalda con `sessions.db`. Mientras tanto vive
/// enteramente en memoria del proceso y se pierde en cada reinicio, lo cual es aceptable porque un
/// reinicio ya interrumpe la ventana de servicio del canal de todos modos.
pub struct RegistroDeDeduplicacion {
    /// Identificador visto → marca temporal a la que se vio por primera vez.
    vistos: HashMap<IdDeduplicacion, SystemTime>,
    /// Máximo `marca_temporal` observado hasta ahora, en tiempo del canal, no de pared.
    horizonte: SystemTime,
    /// Ventana de retención con la que se construyó este registro.
    ventana: Duration,
}

impl RegistroDeDeduplicacion {
    /// Construye el registro con la ventana de retención dada.
    ///
    /// La ventana es un parámetro y no una constante interna: el valor por defecto vive en
    /// [`VENTANA_DE_RETENCION_DEDUPLICACION_POR_DEFECTO`] y quien construye el registro (hoy,
    /// `crates/hexcell/src/main.rs` a partir de `Configuracion::ventana_deduplicacion`) decide si
    /// usa ese valor o uno configurado explícitamente.
    pub fn nuevo(ventana: Duration) -> Self {
        Self {
            vistos: HashMap::new(),
            horizonte: SystemTime::UNIX_EPOCH,
            ventana,
        }
    }

    /// Procesa un identificador de deduplicación llegado con la marca temporal dada.
    ///
    /// Actualiza primero el horizonte monótono, poda las entradas que hayan quedado fuera de la
    /// ventana de retención medida contra ese horizonte y contra el tope duro de entradas
    /// retenidas, y solo entonces decide si `id` ya estaba presente. Un identificador podado por
    /// antigüedad —AC-9— vuelve a parecer nuevo a propósito: es el comportamiento ya fijado por el
    /// plan, no una laguna de esta implementación.
    pub fn procesar(
        &mut self,
        id: IdDeduplicacion,
        marca_temporal: SystemTime,
    ) -> VeredictoDeDeduplicacion {
        if marca_temporal > self.horizonte {
            self.horizonte = marca_temporal;
        }
        self.podar();

        if self.vistos.contains_key(&id) {
            return VeredictoDeDeduplicacion::Duplicado;
        }

        self.vistos.insert(id, marca_temporal);
        VeredictoDeDeduplicacion::Nuevo
    }

    /// Descarta las entradas fuera de la ventana de retención vigente y, si aun así se supera el
    /// tope duro de entradas, descarta además las más antiguas hasta volver a estar dentro de él.
    fn podar(&mut self) {
        if let Some(corte) = self.horizonte.checked_sub(self.ventana) {
            self.vistos.retain(|_, marca| *marca >= corte);
        }

        if self.vistos.len() > LIMITE_DE_ENTRADAS_RETENIDAS {
            let mut restantes: Vec<(IdDeduplicacion, SystemTime)> = self
                .vistos
                .iter()
                .map(|(id, marca)| (id.clone(), *marca))
                .collect();
            restantes.sort_by_key(|(_, marca)| *marca);
            let exceso = restantes.len() - LIMITE_DE_ENTRADAS_RETENIDAS;
            for (id, _) in restantes.into_iter().take(exceso) {
                self.vistos.remove(&id);
            }
        }
    }
}
