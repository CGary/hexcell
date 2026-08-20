//! Orquestación del respaldo de una célula: cinco bases, tres locales y dos por IPC.
//!
//! Las cuatro bases originales son `sessions.db`, `knowledge_live.db`, el almacén de identidad del
//! adaptador y el `sqlstore` del sidecar (`adr-0010`, punto 7). El hallazgo 12 (2026-08-20) añadió
//! una quinta: `identidad.db` del sidecar (lista STOP, mapeo de conversación, cortacircuitos;
//! `adr-0022`). Esta etapa copia las tres locales directamente; el `sqlstore` e `identidad.db` los
//! ejecuta el propio proceso del sidecar bajo `docs/contrato-ipc-respaldo-del-sqlstore.md`,
//! ordenados desde aquí por `ordenar_respaldo_sqlstore` y `ordenar_respaldo_identidad` (etapa A-3).
//!
//! `respaldar_celula` comprueba los tres destinos **antes** de tomar la primera copia, para que un
//! destino ya ocupado o inalcanzable falle sin dejar ninguna copia a medias, y delega la copia en
//! sí en `hexcell_storage::GestorDePools::respaldar_en` y en
//! `hexcell_storage::AlmacenDeIdentidad::respaldar_en`, que son quienes ejecutan `VACUUM INTO`
//! sobre las conexiones que el proceso ya tiene abiertas.
//!
//! Las tres bases locales y la orden al sqlstore comparten un `identificador_de_ronda`: quien
//! orquesta las cuatro llamadas pasa el mismo identificador a `respaldar_celula_con_ronda` y a
//! `ordenar_respaldo_sqlstore`, de modo que ambos lados registran `ronda=<id>` y la ronda completa
//! es correlacionable en los logs. `respaldar_celula` sigue sin cambios de firma, como atajo que
//! genera su propio identificador cuando el llamante no necesita esa correlación.
//!
//! # Disparador del operador en CLI (HEX-029)
//!
//! `respaldar_celula_con_ronda` y `ordenar_respaldo_sqlstore` son invocados por el subcomando CLI
//! `hexcell respaldar` (`crates/hexcell/src/respaldar.rs`), entregado en HEX-029 para permitir el
//! ensayo de restauración de la tarea 18 de la etapa A-3. `respaldar_celula` se mantiene como atajo
//! de 3 parámetros para llamantes que no necesitan correlacionar la ronda. La planificación
//! periódica, la frecuencia de producción y el empaquetado remoto corresponden a decisiones de
//! negocio o a la etapa A-6 (`docs/bitacora-de-descartes.md` D-20).
//!
//! La nota de alcance del punto 6 de `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md`
//! ("ninguna operación de respaldo tiene disparador de producción") queda parcialmente desactualizada
//! por este disparador de operador; el detalle y la decisión humana pendiente sobre un ADR sucesor
//! viven en `docs/STATUS.md`.

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

/// Genera un identificador de ronda de respaldo con resolución de nanosegundos, para que
/// `respaldar_celula` pueda correlacionar sus propias líneas de registro sin exigir que el
/// llamante conozca el concepto de ronda.
fn generar_identificador_de_ronda() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duracion| duracion.as_nanos())
        .unwrap_or(0);
    format!("ronda-{nanos}")
}

/// Respalda, en este orden fijo, las tres bases alcanzables desde esta etapa sobre `directorio`,
/// generando internamente el `identificador_de_ronda` de sus líneas de registro. Atajo de
/// `respaldar_celula_con_ronda` para llamantes que no necesitan correlacionar la ronda con el
/// respaldo del sqlstore.
pub fn respaldar_celula(
    pools: &GestorDePools,
    almacen: &AlmacenDeIdentidad,
    directorio: &Path,
) -> Result<ResumenDeRespaldoDeCelula, ErrorDeAlmacen> {
    respaldar_celula_con_ronda(
        pools,
        almacen,
        directorio,
        &generar_identificador_de_ronda(),
    )
}

/// Respalda, en este orden fijo, las tres bases alcanzables desde esta etapa sobre `directorio`,
/// emitiendo las líneas de registro de la operación bajo el `identificador_de_ronda` dado -- el
/// mismo que, cuando un llamante orquesta las cuatro bases, se pasa también a
/// `ordenar_respaldo_sqlstore` para que la ronda completa sea correlacionable en los logs. Nunca
/// ve ni transporta el texto de un mensaje: solo cuentas, tamaños en bytes y rutas.
pub fn respaldar_celula_con_ronda(
    pools: &GestorDePools,
    almacen: &AlmacenDeIdentidad,
    directorio: &Path,
    identificador_de_ronda: &str,
) -> Result<ResumenDeRespaldoDeCelula, ErrorDeAlmacen> {
    registro::emitir(
        EntradaDeRegistro::nueva(NivelDeRegistro::Info, "respaldo_iniciado")
            .con_detalle(format!("ronda={identificador_de_ronda}")),
    );

    match ejecutar_respaldo(pools, almacen, directorio) {
        Ok(copias) => {
            let bytes_totales: u64 = copias.iter().map(|copia| copia.bytes).sum();
            registro::emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Info, "respaldo_completado").con_detalle(
                    format!(
                        "ronda={identificador_de_ronda} copias={} bytes_totales={bytes_totales}",
                        copias.len()
                    ),
                ),
            );
            Ok(ResumenDeRespaldoDeCelula { copias })
        }
        Err(error) => {
            registro::emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_fallido")
                    .con_detalle(format!("ronda={identificador_de_ronda} {error}")),
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

/// Resultado del respaldo del `sqlstore` ejecutado por el sidecar sobre IPC.
#[derive(Debug)]
pub enum ResultadoRespaldoSqlstore {
    /// Copia verificada generada con éxito por el sidecar.
    Completado(CopiaVerificada),
    /// Fallo informado por el sidecar con su motivo descriptivo.
    Fallido {
        /// Descripción del motivo del fallo.
        motivo: String,
    },
}

/// Solicita al sidecar el respaldo del `sqlstore` a través de su conexión IPC.
///
/// La copia física la ejecuta el propio proceso del sidecar vía `VACUUM INTO` sobre su conexión
/// dedicada de solo lectura, y este método espera el acuse correspondiente correlacionado por
/// `identificador_de_ronda`.
pub async fn ordenar_respaldo_sqlstore(
    adaptador: &hexcell_canal_whatsmeow::AdaptadorWhatsmeow,
    destino: &Path,
    identificador_de_ronda: &str,
    plazo: std::time::Duration,
) -> Result<ResultadoRespaldoSqlstore, hexcell_canal_whatsmeow::ErrorCanalWhatsmeow> {
    let destino_str = destino.to_string_lossy();
    match adaptador
        .ordenar_respaldo_sqlstore(&destino_str, identificador_de_ronda, plazo)
        .await
    {
        Ok(acuse) => {
            if acuse.resultado == "completado" {
                registro::emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Info, "respaldo_sqlstore_completado")
                        .con_detalle(format!(
                            "ronda={identificador_de_ronda} bytes={}",
                            acuse.bytes
                        )),
                );
                Ok(ResultadoRespaldoSqlstore::Completado(CopiaVerificada {
                    nombre_logico: "sqlstore.db",
                    ruta: std::path::PathBuf::from(acuse.ruta_de_la_copia),
                    bytes: acuse.bytes as u64,
                }))
            } else if acuse.resultado == "fallido" {
                registro::emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_sqlstore_fallido")
                        .con_detalle(format!(
                            "ronda={identificador_de_ronda} motivo={}",
                            acuse.motivo
                        )),
                );
                Ok(ResultadoRespaldoSqlstore::Fallido {
                    motivo: acuse.motivo,
                })
            } else {
                let motivo = format!("resultado desconocido en acuse: {}", acuse.resultado);
                registro::emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_sqlstore_fallido")
                        .con_detalle(format!("ronda={identificador_de_ronda} motivo={motivo}")),
                );
                Ok(ResultadoRespaldoSqlstore::Fallido { motivo })
            }
        }
        Err(error) => {
            registro::emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_sqlstore_fallido")
                    .con_detalle(format!("ronda={identificador_de_ronda} error={error}")),
            );
            Err(error)
        }
    }
}

/// Resultado del respaldo de `identidad.db` ejecutado por el sidecar sobre IPC.
#[derive(Debug)]
pub enum ResultadoRespaldoIdentidad {
    /// Copia verificada generada con éxito por el sidecar.
    Completado(CopiaVerificada),
    /// Fallo informado por el sidecar con su motivo descriptivo.
    Fallido {
        /// Descripción del motivo del fallo.
        motivo: String,
    },
}

/// Solicita al sidecar el respaldo del almacén de identidad (`identidad.db`) sobre IPC.
///
/// Espejo de [`ordenar_respaldo_sqlstore`]: la copia física la ejecuta el propio proceso del
/// sidecar vía `VACUUM INTO` sobre su conexión dedicada de solo lectura (`adr-0022`); el núcleo
/// nunca abre `identidad.db`. Espera el acuse correlacionado por `identificador_de_ronda`.
pub async fn ordenar_respaldo_identidad(
    adaptador: &hexcell_canal_whatsmeow::AdaptadorWhatsmeow,
    destino: &Path,
    identificador_de_ronda: &str,
    plazo: std::time::Duration,
) -> Result<ResultadoRespaldoIdentidad, hexcell_canal_whatsmeow::ErrorCanalWhatsmeow> {
    let destino_str = destino.to_string_lossy();
    let acuse = match adaptador
        .ordenar_respaldo_identidad(&destino_str, identificador_de_ronda, plazo)
        .await
    {
        Ok(acuse) => acuse,
        Err(error) => {
            registro::emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_identidad_fallido")
                    .con_detalle(format!("ronda={identificador_de_ronda} error={error}")),
            );
            return Err(error);
        }
    };

    if acuse.resultado == "completado" {
        registro::emitir(
            EntradaDeRegistro::nueva(NivelDeRegistro::Info, "respaldo_identidad_completado")
                .con_detalle(format!(
                    "ronda={identificador_de_ronda} bytes={}",
                    acuse.bytes
                )),
        );
        return Ok(ResultadoRespaldoIdentidad::Completado(CopiaVerificada {
            nombre_logico: "identidad.db",
            ruta: std::path::PathBuf::from(acuse.ruta_de_la_copia),
            bytes: acuse.bytes as u64,
        }));
    }

    let motivo = if acuse.resultado == "fallido" {
        acuse.motivo
    } else {
        format!("resultado desconocido en acuse: {}", acuse.resultado)
    };
    registro::emitir(
        EntradaDeRegistro::nueva(NivelDeRegistro::Error, "respaldo_identidad_fallido")
            .con_detalle(format!("ronda={identificador_de_ronda} motivo={motivo}")),
    );
    Ok(ResultadoRespaldoIdentidad::Fallido { motivo })
}
