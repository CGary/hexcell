//! Orquestación del respaldo de una célula: las tres bases alcanzables desde esta etapa.
//!
//! Las cuatro bases del respaldo de una célula son `sessions.db`, `knowledge_live.db`, el almacén
//! de identidad del adaptador y el `sqlstore` del sidecar (`adr-0010`, punto 7). Esta etapa solo
//! puede copiar las tres primeras por sí misma: el `sqlstore` lo ejecuta el propio proceso del
//! sidecar bajo el contrato versionado de `docs/contrato-ipc-respaldo-del-sqlstore.md`, y su
//! ejecución real es explícitamente de la etapa A-3.
//!
//! `respaldar_celula` comprueba los tres destinos **antes** de tomar la primera copia, para que un
//! destino ya ocupado o inalcanzable falle sin dejar ninguna copia a medias, y delega la copia en
//! sí en `hexcell_storage::GestorDePools::respaldar_en` y en
//! `hexcell_storage::AlmacenDeIdentidad::respaldar_en`, que son quienes ejecutan `VACUUM INTO`
//! sobre las conexiones que el proceso ya tiene abiertas.
//!
//! # Sin disparador de producción, y eso es una decisión
//!
//! Ni la especificación de esta tarea ni la tarea 13 del plan de la etapa A-2 piden un
//! planificador, una ruta HTTP ni un subcomando de CLI: el apagado ordenado es de HEX-007 y las
//! metas explícitamente descartadas de esta tarea prohíben reabrirlo, y el empaquetado y la
//! planificación son de la etapa A-6. Así que los únicos llamantes de `respaldar_celula` en este
//! árbol son los tests de integración; un futuro planificador, o un operador humano, invocan esta
//! misma función siguiendo el procedimiento que describe
//! `docs/runbook-restauracion-de-celula.md`. La ausencia de disparador queda anotada también en
//! `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md` y en `docs/STATUS.md`, para que se lea
//! como una decisión y no como un hueco.

use std::path::Path;

use hexcell_storage::{
    AlmacenDeIdentidad, CopiaVerificada, ErrorDeAlmacen, GestorDePools,
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
    NOMBRE_DE_ARCHIVO_DE_SESIONES, verificar_destino_disponible,
};

use crate::registro::{self, EntradaDeRegistro, NivelDeRegistro};

/// Resultado agregado del respaldo de las tres bases alcanzables desde esta etapa.
#[derive(Debug)]
pub struct ResumenDeRespaldoDeCelula {
    /// Copias verificadas, en el orden fijo en que se tomaron: `sessions.db`,
    /// `knowledge_live.db` y `adapter_identity.db`.
    pub copias: Vec<CopiaVerificada>,
}

/// Respalda, en este orden fijo, las tres bases alcanzables desde esta etapa sobre `directorio`,
/// emitiendo las líneas de registro de la operación. Nunca ve ni transporta el texto de un
/// mensaje: solo cuentas, tamaños en bytes y rutas.
pub fn respaldar_celula(
    pools: &GestorDePools,
    almacen: &AlmacenDeIdentidad,
    directorio: &Path,
) -> Result<ResumenDeRespaldoDeCelula, ErrorDeAlmacen> {
    registro::emitir(EntradaDeRegistro::nueva(
        NivelDeRegistro::Info,
        "respaldo_iniciado",
    ));

    match ejecutar_respaldo(pools, almacen, directorio) {
        Ok(copias) => {
            let bytes_totales: u64 = copias.iter().map(|copia| copia.bytes).sum();
            registro::emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Info, "respaldo_completado").con_detalle(
                    format!("copias={} bytes_totales={bytes_totales}", copias.len()),
                ),
            );
            Ok(ResumenDeRespaldoDeCelula { copias })
        }
        Err(error) => {
            registro::emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_fallido")
                    .con_detalle(error.to_string()),
            );
            Err(error)
        }
    }
}

/// Comprueba los tres destinos y ejecuta las dos copias que entrega el respaldo.
fn ejecutar_respaldo(
    pools: &GestorDePools,
    almacen: &AlmacenDeIdentidad,
    directorio: &Path,
) -> Result<Vec<CopiaVerificada>, ErrorDeAlmacen> {
    verificar_destino_disponible(&directorio.join(NOMBRE_DE_ARCHIVO_DE_SESIONES))?;
    verificar_destino_disponible(&directorio.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO))?;
    verificar_destino_disponible(&directorio.join(NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR))?;

    let mut copias = pools.respaldar_en(directorio)?.copias;
    copias.push(almacen.respaldar_en(directorio)?);
    Ok(copias)
}
