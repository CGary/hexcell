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

use crate::configuracion::FuenteDeConfiguracion;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::drenaje::{
    DesenlaceDeDrenaje, LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO, drenar_epoca_superseida,
};
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::pools::GestorDePools;
use hexcell_storage::promocion::{DesenlaceDePromocion, EpocaSuperseida, promover_epoca};
use hexcell_storage::reversion::{DesenlaceDeReversion, revertir_a_epoca};

/// Nombre de la variable de entorno que configura el límite de drenaje de época en milisegundos.
pub const HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS: &str = "HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS";

/// Nombre de la variable de entorno que configura la cantidad de épocas previas a retener.
pub const HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS: &str = "HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS";

/// Obtiene el límite temporal configurado para el drenaje de época o recurre al valor por omisión.
///
/// La fuente llega por parámetro, nunca de un global: es lo que permite ejercer los casos válido,
/// no numérico y ausente sin escribir el entorno del proceso desde un hilo de pruebas.
pub fn limite_de_drenaje_de_epoca_desde_fuente(fuente: &dyn FuenteDeConfiguracion) -> Duration {
    match fuente.leer(HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS) {
        Some(valor_texto) => match valor_texto.parse::<u64>() {
            Ok(ms) => Duration::from_millis(ms),
            Err(_) => LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO,
        },
        None => LIMITE_DE_DRENAJE_DE_EPOCA_POR_DEFECTO,
    }
}

/// Obtiene la ventana de retención de épocas configurada en la fuente o recurre al valor por omisión.
pub fn ventana_de_retencion_de_epocas_desde_fuente(fuente: &dyn FuenteDeConfiguracion) -> usize {
    match fuente.leer(HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS) {
        Some(valor_texto) => match valor_texto.parse::<usize>() {
            Ok(ventana) => ventana,
            Err(_) => hexcell_storage::retencion::VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO,
        },
        None => hexcell_storage::retencion::VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO,
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

/// Orquesta de forma asíncrona la reversión de la base de datos de conocimiento a una época previa.
///
/// Invoca la secuencia síncrona de validación de integridad, reasignación atómica del enlace simbólico
/// y conmutación atómica del pool en el hilo de ejecución actual sin intermediación de `spawn_blocking`.
pub async fn revertir_epoca_de_conocimiento(
    gestor: &GestorDePools,
    ruta_datos: &Path,
    configuracion_de_fragmentacion: &ConfiguracionDeFragmentacion,
    numero_destino: i64,
) -> Result<DesenlaceDeReversion, ErrorDeAlmacen> {
    revertir_a_epoca(
        gestor,
        ruta_datos,
        configuracion_de_fragmentacion,
        numero_destino,
    )
}

/// Orquesta de forma asíncrona el drenaje ordenado de una época superseída y retira su registro en uso.
///
/// Invoca la secuencia síncrona en línea en la tarea actual sin `spawn_blocking`, aplicando el límite
/// temporal configurado en la fuente inyectada o el valor por omisión. Si el drenaje concluye con
/// éxito, retira la época del registro `epocas_en_uso` presentando la constancia no falsificable.
pub async fn drenar_epoca_superseida_de_conocimiento(
    gestor: &GestorDePools,
    epoca: EpocaSuperseida,
    fuente: &dyn FuenteDeConfiguracion,
) -> Result<DesenlaceDeDrenaje, ErrorDeAlmacen> {
    let limite = limite_de_drenaje_de_epoca_desde_fuente(fuente);
    let desenlace = drenar_epoca_superseida(epoca, limite)?;
    if let DesenlaceDeDrenaje::Drenada { ref constancia, .. } = desenlace {
        gestor.retirar_epoca_en_uso(constancia);
    }
    Ok(desenlace)
}

/// Orquesta de forma asíncrona la purga de épocas selladas retiradas fuera de la ventana de retención.
///
/// Invoca la secuencia síncrona en línea en la tarea actual sin intermediación de `spawn_blocking`,
/// consultando la ventana de retención configurada en la fuente o recurriendo al valor por omisión.
pub async fn purgar_epocas_de_conocimiento(
    gestor: &GestorDePools,
    ruta_datos: &Path,
    fuente: &dyn FuenteDeConfiguracion,
) -> Result<hexcell_storage::retencion::DesenlaceDePurga, ErrorDeAlmacen> {
    let ventana = ventana_de_retencion_de_epocas_desde_fuente(fuente);
    hexcell_storage::retencion::purgar_epocas_retiradas(gestor, ruta_datos, ventana)
}
