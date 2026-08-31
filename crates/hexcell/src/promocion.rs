//! Orquestación asíncrona de la conmutación de épocas de conocimiento.
//!
//! Este módulo provee el servicio de aplicación asíncrono que invoca la secuencia
//! síncrona de almacenamiento en `hexcell_storage::promocion`. La ejecución corre
//! en línea en la tarea asíncrona actual sin intermediación de `spawn_blocking`,
//! siguiendo el mismo precedente establecido en la ingesta de conocimiento (HEX-052).
//! La exclusión mutua frente a ejecuciones simultáneas reside en la compuerta atómica
//! del gestor de persistencia.

use std::path::Path;

use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::pools::GestorDePools;
use hexcell_storage::promocion::{DesenlaceDePromocion, promover_epoca};

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
