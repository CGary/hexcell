//! Lectura de una copia `VACUUM INTO` de `sessions.db` para el reporte de consumo de
//! unidades por conversación (`reporte tokens`, tarea 23 de la etapa A-6, HEX-084).
//!
//! # Por qué una copia y no la base caliente
//!
//! adr-0024 fija el dato de origen: el reporte nunca abre la `sessions.db` en caliente,
//! solo copias `VACUUM INTO` ya producidas por la ruta de respaldo de la etapa A-2
//! (`hexcell_storage::respaldo::respaldar_base`). Este módulo abre la copia **siempre** con
//! `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`, la misma combinación de banderas que
//! `crates/hexcell-storage/src/pools.rs` y `respaldo.rs` usan para las conexiones de solo
//! lectura.
//!
//! # Una sola fórmula, la de la migración 0004
//!
//! La agregación copia literalmente la fórmula de la vista `consumo_por_conversacion`
//! (migración 0004): `monto_reservado` menos el monto de la conciliación, sumado solo
//! sobre reservas `'conciliada'`; las `'liberada'` nunca cuentan. La misma sentencia sirve
//! para el caso sin periodo y con periodo por `resuelta_ms` (`--desde` inclusivo, `--hasta`
//! exclusivo): los límites se ligan como `Option<i64>` y un `NULL` es un no-op. No existe
//! una segunda copia de la fórmula que pueda desviarse de la vista con el tiempo. La unidad
//! se llama **unidades** en todo este módulo, nunca «tokens».

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

/// Fallo al generar el reporte: la copia no se pudo abrir, o se abrió pero no tiene el
/// esquema que la consulta necesita (tablas o columnas ausentes, por ejemplo en una base
/// anterior a la migración 0004). Todo error de `rusqlite` de la apertura, la preparación
/// o la lectura de filas se envuelve aquí.
///
/// El llamante lo mapea a `CodigoDeSalida::Fallo`, nunca a `UsoIncorrecto`: un archivo
/// ilegible o sin esquema no es un error de invocación, es un fallo de ejecución (AC-6).
#[derive(Debug)]
pub struct ErrorDeReporte {
    mensaje: String,
}

impl ErrorDeReporte {
    fn nuevo(contexto: &str, causa: rusqlite::Error) -> Self {
        Self {
            mensaje: format!("{contexto}: {causa}"),
        }
    }
}

impl std::fmt::Display for ErrorDeReporte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.mensaje)
    }
}

impl std::error::Error for ErrorDeReporte {}

/// Consulta única para el caso sin periodo y con periodo.
///
/// Es la fórmula literal de la vista `consumo_por_conversacion` de la migración 0004 con
/// dos añadidos: la ventana por `resuelta_ms` ligada a `?1`/`?2` (un `NULL` no filtra) y
/// el `ORDER BY id_conversacion` que la salida del reporte exige. El `CASE` de la vista se
/// copia tal cual: solo las reservas `'conciliada'` restan su conciliación del monto
/// reservado; las `'liberada'` contribuyen cero.
const CONSULTA_DE_CONSUMO_POR_CONVERSACION: &str = "SELECT r.id_conversacion, \
     SUM(CASE WHEN r.estado = 'conciliada' THEN r.monto_reservado - COALESCE(m.monto, 0) ELSE 0 END) \
     AS unidades_consumidas \
     FROM reservas AS r \
     LEFT JOIN movimientos AS m ON m.id_reserva = r.id AND m.clase = 'conciliacion' \
     WHERE r.id_conversacion IS NOT NULL \
       AND (?1 IS NULL OR r.resuelta_ms >= ?1) \
       AND (?2 IS NULL OR r.resuelta_ms < ?2) \
     GROUP BY r.id_conversacion \
     ORDER BY r.id_conversacion";

/// Genera el reporte de consumo de unidades por conversación desde una copia `VACUUM INTO`.
///
/// Abre `copia` en solo lectura, ejecuta la consulta única de [`CONSULTA_DE_CONSUMO_POR_CONVERSACION`]
/// y devuelve una fila por conversación con consumo distinto de cero o igual a cero —la
/// misma semántica de la vista, que agrupa también a las conversaciones cuyas reservas
/// fueron solo liberadas— ordenadas por identificador. `desde_ms` y `hasta_ms` acotan la
/// ventana por `resuelta_ms` (inclusiva y exclusiva respectivamente); `None` deja el
/// extremo abierto.
pub fn generar_reporte(
    copia: &Path,
    desde_ms: Option<i64>,
    hasta_ms: Option<i64>,
) -> Result<Vec<(String, i64)>, ErrorDeReporte> {
    let conexion = Connection::open_with_flags(
        copia,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|causa| ErrorDeReporte::nuevo("abrir la copia VACUUM INTO en solo lectura", causa))?;

    let mut sentencia = conexion
        .prepare(CONSULTA_DE_CONSUMO_POR_CONVERSACION)
        .map_err(|causa| {
            ErrorDeReporte::nuevo("preparar la consulta de consumo por conversación", causa)
        })?;

    let filas = sentencia
        .query_map(params![desde_ms, hasta_ms], |fila| {
            Ok((fila.get::<_, String>(0)?, fila.get::<_, i64>(1)?))
        })
        .map_err(|causa| {
            ErrorDeReporte::nuevo("ejecutar la consulta de consumo por conversación", causa)
        })?;

    let mut resultado = Vec::new();
    for fila in filas {
        resultado.push(fila.map_err(|causa| {
            ErrorDeReporte::nuevo("leer una fila de consumo por conversación", causa)
        })?);
    }
    Ok(resultado)
}
