//! Objetos de valor del protocolo IPC versión 5 (documento 1.4): un struct por tipo de mensaje.
//!
//! Cada struct lleva `#[serde(deny_unknown_fields)]` porque la regla 3 del protocolo
//! (sección 1 de `docs/protocolo-ipc-nucleo-sidecar.md`) hace **obligatorio** rechazar campos
//! desconocidos, no opcional. La regla 4 hace que todos los campos estén siempre presentes, con
//! la ausencia codificada como `""` o `0`, nunca omitiendo el campo: por eso no se usa `Option`
//! en ningún campo.
//!
//! Los tipos de este módulo modelan el cable **tal como es**: profundidad 1, solo cadenas y
//! enteros con signo de 64 bits, sin booleanos, sin `null`, sin coma flotante.

use serde::{Deserialize, Serialize};

/// Versión de cable del protocolo. En esta implementación, `5` (documento 1.4).
pub const VERSION_PROTOCOLO: i64 = 5;

/// Límite de línea del protocolo: 131 072 bytes (128 KiB), contando el salto de línea final.
/// Una línea más larga es un error de protocolo y cierra la conexión.
pub const LIMITE_DE_LINEA: usize = 131_072;

// ---------------------------------------------------------------------------
// Mensajes bidireccionales
// ---------------------------------------------------------------------------

/// Saludo de versión (sección 3): primer mensaje de toda conexión, en las dos direcciones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Saludo {
    /// Versión de cable del protocolo.
    pub version: i64,
    /// Tipo de mensaje: siempre `"saludo"`.
    pub tipo: String,
    /// Emisor: `"nucleo"` o `"sidecar"`.
    pub emisor: String,
    /// Identificador opaco de la célula, para correlacionar registros.
    pub id_celula: String,
}

// ---------------------------------------------------------------------------
// Mensajes del sidecar al núcleo
// ---------------------------------------------------------------------------

/// Evento entrante del sidecar (sección 6): un mensaje recibido del canal, ya normalizado.
///
/// Los siete campos son exactamente los de la especificación. Ningún identificador de transporte
/// cruza esta frontera: `id_conversacion` e `id_remitente` son opacos, acuñados por el almacén
/// de identidad del sidecar (HEX-014), y el núcleo los trata como tales.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventoEntranteIpc {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"evento_entrante"`.
    pub tipo: String,
    /// Identificador durable del evento (FR-12). Es lo que el acuse referencia.
    pub id_deduplicacion: String,
    /// Identificador **interno** del hilo, opaco para el núcleo.
    pub id_conversacion: String,
    /// Identificador **interno** de quien escribió, opaco para el núcleo.
    pub id_remitente: String,
    /// Texto del mensaje, ya normalizado.
    pub contenido: String,
    /// Momento del evento según el transporte, en milisegundos desde la época Unix.
    pub marca_temporal_ms: i64,
}

/// Estado de sesión del sidecar (sección 6): estado de la sesión de WhatsApp y su causa.
///
/// Los campos `causa`, `codigo` y `expira_en_ms` son taxonomía de whatsmeow y se quedan
/// **dentro de este crate**: no se elevan a `hexcell_core::canal::EstadoSesion`, que solo
/// necesita las cuatro variantes sin detalle de transporte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstadoSesionIpc {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"estado_sesion"`.
    pub tipo: String,
    /// Estado: `"activa"`, `"reconectando"`, `"desvinculada"` o `"pausada"`.
    pub estado: String,
    /// Variante cruda de la taxonomía de desconexión; `""` si no aplica.
    pub causa: String,
    /// Código de la rama de desconexión cuando lo hay; `0` si no aplica.
    pub codigo: i64,
    /// Expiración declarada de un baneo temporal, en milisegundos desde la época Unix; `0` si no
    /// aplica. Es un instante **absoluto**, nunca una duración relativa.
    pub expira_en_ms: i64,
}

/// Código de emparejamiento del sidecar (sección 6): código QR o código de vinculación.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodigoEmparejamiento {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"codigo_emparejamiento"`.
    pub tipo: String,
    /// Método: `"qr"` o `"codigo_de_vinculacion"`.
    pub metodo: String,
    /// Dato opaco: la cadena a codificar como QR, o el código de ocho caracteres.
    pub valor: String,
    /// Milisegundos desde la época Unix en que este código deja de ser válido. `0` si la
    /// expiración es desconocida.
    pub expira_en_ms: i64,
}

/// Acuse de emparejamiento del sidecar (sección 6): resultado terminal del emparejamiento.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcuseEmparejamiento {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"acuse_emparejamiento"`.
    pub tipo: String,
    /// Resultado: `"completado"`, `"expirado"` o `"fallido"`.
    pub resultado: String,
    /// Descripción legible si `resultado` es `"fallido"`; `""` en caso contrario. **Nunca lleva
    /// la cadena QR, el código de vinculación ni ningún otro dato de credencial.**
    pub motivo: String,
}

/// Acuse del respaldo del `sqlstore` (sección 7): desenlace de la copia.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcuseRespaldoSqlstore {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"acuse_respaldo_sqlstore"`.
    pub tipo: String,
    /// El mismo identificador de ronda recibido en la orden.
    pub identificador_de_ronda: String,
    /// Resultado: `"completado"` o `"fallido"`.
    pub resultado: String,
    /// Ruta de la copia; `""` si `resultado` es `"fallido"`.
    pub ruta_de_la_copia: String,
    /// Tamaño de la copia en bytes; `0` si `resultado` es `"fallido"`.
    pub bytes: i64,
    /// Descripción legible del fallo; `""` si `resultado` es `"completado"`. **Nunca lleva
    /// ninguna credencial del protocolo ni ningún contenido de mensaje.**
    pub motivo: String,
}

/// Acuse del respaldo del almacén de identidad del sidecar (`identidad.db`): desenlace de la copia.
///
/// Mismos cinco campos que [`AcuseRespaldoSqlstore`], pero es un TIPO distinto a propósito
/// (`adr-0022`): el adaptador correlaciona este acuse en un mapa de pendientes separado, para que
/// dos acuses de la misma ronda —uno del sqlstore, otro de identidad— nunca colisionen por clave.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcuseRespaldoIdentidad {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"acuse_respaldo_identidad"`.
    pub tipo: String,
    /// El mismo identificador de ronda recibido en la orden.
    pub identificador_de_ronda: String,
    /// Resultado: `"completado"` o `"fallido"`.
    pub resultado: String,
    /// Ruta de la copia; `""` si `resultado` es `"fallido"`.
    pub ruta_de_la_copia: String,
    /// Tamaño de la copia en bytes; `0` si `resultado` es `"fallido"`.
    pub bytes: i64,
    /// Descripción legible del fallo; `""` si `resultado` es `"completado"`. **Nunca lleva
    /// ninguna credencial del protocolo ni ningún contenido de mensaje.**
    pub motivo: String,
}

// ---------------------------------------------------------------------------
// Mensajes del núcleo al sidecar
// ---------------------------------------------------------------------------

/// Confirmación de entrega (sección 4): acuse durable de un `evento_entrante`.
///
/// Lleva el **identificador de deduplicación** del evento, nunca un número de secuencia por
/// conexión (sección 4 del protocolo, prohibición explícita).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmacion {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"confirmacion"`.
    pub tipo: String,
    /// El mismo `id_deduplicacion` que llegó en el `evento_entrante`.
    pub id_deduplicacion: String,
}

/// Mensaje saliente (sección 4): un mensaje que el núcleo envía para ser entregado.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MensajeSalienteIpc {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"mensaje_saliente"`.
    pub tipo: String,
    /// Identificador opaco de este mensaje para acuses.
    pub id_mensaje: String,
    /// Identificador de la conversación de destino.
    pub id_conversacion: String,
    /// Contenido textual.
    pub contenido: String,
    /// Marca temporal de origen en milisegundos absolutos, tomada del estado del hilo.
    pub marca_temporal_origen_ms: i64,
}

/// Acuse de envío del sidecar (sección 4): el resultado de procesar un `mensaje_saliente`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcuseEnvioIpc {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"acuse_envio"`.
    pub tipo: String,
    /// El mismo identificador enviado en el `mensaje_saliente`.
    pub id_mensaje: String,
    /// Estado: `"enviado"`, `"entregado"`, `"leido"` o `"fallido"`.
    pub estado: String,
    /// Identificador que el canal acuñó para este mensaje, si lo hay.
    pub id_correlacion: String,
    /// Motivo legible si falló; de lo contrario `""`.
    pub motivo: String,
    /// Momento del acuse según el transporte en milisegundos desde la época Unix.
    pub marca_temporal_ms: i64,
}

/// Orden de emparejar (sección 6): orden de iniciar un emparejamiento.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdenEmparejar {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"orden_emparejar"`.
    pub tipo: String,
    /// Método: `"qr"` o `"codigo_de_vinculacion"`.
    pub metodo: String,
}

/// Orden de respaldo del `sqlstore` (sección 7): orden de copia del `sqlstore`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdenRespaldoSqlstore {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"orden_respaldo_sqlstore"`.
    pub tipo: String,
    /// Cadena fija `"respaldar_sqlstore"`.
    pub orden: String,
    /// Directorio de destino ya resuelto por quien dispara la orden.
    pub destino: String,
    /// Agrupa esta orden con las de las otras tres bases de la misma ronda.
    pub identificador_de_ronda: String,
}

/// Orden de respaldo del almacén de identidad del sidecar (`identidad.db`): orden de copia.
///
/// Mismos tres campos que [`OrdenRespaldoSqlstore`]; es un tipo distinto (`adr-0022`) para que su
/// acuse (`acuse_respaldo_identidad`) no colisione con el del sqlstore en la misma ronda.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdenRespaldoIdentidad {
    /// Versión de cable.
    pub version: i64,
    /// Tipo de mensaje: siempre `"orden_respaldo_identidad"`.
    pub tipo: String,
    /// Cadena fija `"respaldar_identidad"`.
    pub orden: String,
    /// Directorio de destino ya resuelto por quien dispara la orden.
    pub destino: String,
    /// Agrupa esta orden con las de las otras bases de la misma ronda.
    pub identificador_de_ronda: String,
}

// ---------------------------------------------------------------------------
// Enumerado cerrado de despacho: línea entrante → variante tipada
// ---------------------------------------------------------------------------

/// Mensaje entrante del sidecar, despachado por el campo `tipo`.
///
/// Las variantes cubren los tipos que el sidecar puede emitir hacia el núcleo. Los tipos que el
/// núcleo envía (confirmacion, orden_emparejar, orden_respaldo_sqlstore, orden_respaldo_identidad,
/// mensaje_saliente) no aparecen aquí porque no son mensajes que el núcleo reciba.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MensajeEntrante {
    /// Saludo de versión del sidecar.
    Saludo(Saludo),
    /// Evento entrante del canal.
    EventoEntrante(EventoEntranteIpc),
    /// Estado de sesión de WhatsApp.
    EstadoSesion(EstadoSesionIpc),
    /// Código de emparejamiento (QR o vinculación).
    CodigoEmparejamiento(CodigoEmparejamiento),
    /// Acuse terminal del emparejamiento.
    AcuseEmparejamiento(AcuseEmparejamiento),
    /// Acuse del respaldo del `sqlstore`.
    AcuseRespaldoSqlstore(AcuseRespaldoSqlstore),
    /// Acuse del respaldo del almacén de identidad del sidecar (`identidad.db`).
    AcuseRespaldoIdentidad(AcuseRespaldoIdentidad),
    /// Acuse de envío de un mensaje saliente.
    AcuseEnvio(AcuseEnvioIpc),
}

/// Analiza una línea JSON ya validada en tamaño y la despacha al tipo concreto por el campo
/// `tipo`.
///
/// Devuelve un error de protocolo si:
/// - La línea no es un objeto JSON válido.
/// - El campo `tipo` no es una cadena o no está presente.
/// - El valor de `tipo` no es uno de los cinco tipos que el sidecar puede emitir al núcleo.
/// - Algún campo es desconocido (regla 3), está ausente, o tiene un tipo incorrecto.
///
/// **Nunca se registra la línea recibida** en el mensaje de error: podría contener texto de
/// mensaje (`adr-0019`). Solo se nombra el tipo de error y, como mucho, el campo ofensor.
pub fn analizar_mensaje_entrante(linea: &str) -> Result<MensajeEntrante, String> {
    // Primer pase: extraer solo el campo `tipo` para despachar sin deserializar todo.
    // Se usa serde_json::Value parcial solo para obtener el tipo.
    let valor: serde_json::Value =
        serde_json::from_str(linea).map_err(|e| format!("JSON inválido: {e}"))?;

    let tipo = valor
        .get("tipo")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "campo 'tipo' ausente o no es una cadena".to_string())?;

    match tipo {
        "saludo" => {
            let msg: Saludo =
                serde_json::from_str(linea).map_err(|e| format!("saludo inválido: {e}"))?;
            Ok(MensajeEntrante::Saludo(msg))
        }
        "evento_entrante" => {
            let msg: EventoEntranteIpc = serde_json::from_str(linea)
                .map_err(|e| format!("evento_entrante inválido: {e}"))?;
            Ok(MensajeEntrante::EventoEntrante(msg))
        }
        "estado_sesion" => {
            let msg: EstadoSesionIpc =
                serde_json::from_str(linea).map_err(|e| format!("estado_sesion inválido: {e}"))?;
            Ok(MensajeEntrante::EstadoSesion(msg))
        }
        "codigo_emparejamiento" => {
            let msg: CodigoEmparejamiento = serde_json::from_str(linea)
                .map_err(|e| format!("codigo_emparejamiento inválido: {e}"))?;
            Ok(MensajeEntrante::CodigoEmparejamiento(msg))
        }
        "acuse_emparejamiento" => {
            let msg: AcuseEmparejamiento = serde_json::from_str(linea)
                .map_err(|e| format!("acuse_emparejamiento inválido: {e}"))?;
            Ok(MensajeEntrante::AcuseEmparejamiento(msg))
        }
        "acuse_respaldo_sqlstore" => {
            let msg: AcuseRespaldoSqlstore = serde_json::from_str(linea)
                .map_err(|e| format!("acuse_respaldo_sqlstore inválido: {e}"))?;
            Ok(MensajeEntrante::AcuseRespaldoSqlstore(msg))
        }
        "acuse_respaldo_identidad" => {
            let msg: AcuseRespaldoIdentidad = serde_json::from_str(linea)
                .map_err(|e| format!("acuse_respaldo_identidad inválido: {e}"))?;
            Ok(MensajeEntrante::AcuseRespaldoIdentidad(msg))
        }
        "acuse_envio" => {
            let msg: AcuseEnvioIpc =
                serde_json::from_str(linea).map_err(|e| format!("acuse_envio inválido: {e}"))?;
            Ok(MensajeEntrante::AcuseEnvio(msg))
        }
        // Los tipos que el núcleo ENVÍA no se esperan como entrantes.
        "confirmacion"
        | "orden_emparejar"
        | "orden_respaldo_sqlstore"
        | "orden_respaldo_identidad"
        | "mensaje_saliente" => Err(format!(
            "tipo '{tipo}' no es un mensaje entrante válido del sidecar"
        )),
        _ => Err(format!("tipo desconocido: '{tipo}'")),
    }
}
