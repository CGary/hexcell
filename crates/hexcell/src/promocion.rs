//! Orquestación asíncrona de la conmutación y drenaje de épocas de conocimiento.
//!
//! Este módulo provee los servicios de aplicación asíncronos que invocan las secuencias
//! síncronas de almacenamiento en `hexcell_storage::promocion` y `hexcell_storage::drenaje`.
//! La ejecución corre en línea en la tarea asíncrona actual sin intermediación de
//! `spawn_blocking`, siguiendo el precedente de la ingesta de conocimiento (HEX-052).
//! La exclusión mutua frente a ejecuciones simultáneas reside en la compuerta atómica
//! del gestor de persistencia.

use std::path::Path;
use std::time::Duration;

use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::drenaje::{
    DesenlaceDeDrenaje, LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO, drenar_epoca_superseida,
};
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::pools::GestorDePools;
use hexcell_storage::promocion::{DesenlaceDePromocion, EpocaSuperseida, promover_epoca};

/// Nombre de la variable de entorno que configura el límite de drenaje de época en milisegundos.
pub const HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS: &str = "HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS";

/// Obtiene el límite temporal configurado para el drenaje de época o recurre al valor por omisión.
pub fn limite_de_drenaje_de_epoca_desde_entorno() -> Duration {
    match std::env::var(HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS) {
        Ok(valor_texto) => match valor_texto.parse::<u64>() {
            Ok(ms) => Duration::from_millis(ms),
            Err(_) => LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO,
        },
        Err(_) => LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO,
    }
}

/// Orquesta de forma asíncrona la promoción de la base de datos de conocimiento en sombra.
///
/// Invoca la secuencia síncrona de validación, sellado, consolidación, renombrado
/// y reemplazo atómico del pool en el hilo de ejecución actual.
pub async fn promover_epoca_de_conocimiento(
    gestor: &GestorDePools,
    ruta_datos: &Path,
    configuracion_de_fragmentacion: &ConfiguracionDeFragmentacion,
    ahora_ms: i64,
) -> Result<DesenlaceDePromocion, ErrorDeAlmacen> {
    promover_epoca(gestor, ruta_datos, configuracion_de_fragmentacion, ahora_ms)
}

/// Orquesta de forma asíncrona el drenaje ordenado de una época superseída.
///
/// Invoca la secuencia síncrona en línea en la tarea actual sin `spawn_blocking`,
/// aplicando el límite temporal configurado desde el entorno o el valor por omisión.
pub async fn drenar_epoca_superseida_de_conocimiento(
    epoca: EpocaSuperseida,
) -> Result<DesenlaceDeDrenaje, ErrorDeAlmacen> {
    let limite = limite_de_drenaje_de_epoca_desde_entorno();
    drenar_epoca_superseida(epoca, limite)
}
