//! Conversión entre `SystemTime` y el entero que SQLite guarda.
//!
//! # Por qué milisegundos y no segundos
//!
//! La poda del registro de deduplicación se mide contra el **máximo instante recibido**, no
//! contra un reloj de pared, y ese horizonte tiene que poder ordenar dos eventos llegados dentro
//! del mismo segundo: con segundos, dos entregas consecutivas de una ráfaga colapsarían al mismo
//! valor y el corte de la poda dejaría de distinguirlas. Con milisegundos, el orden se conserva y
//! el valor sigue cabiendo holgadamente en el entero de 64 bits que SQLite compara e indexa más
//! barato que cualquier representación textual.
//!
//! # Por qué ninguna de las dos funciones puede fallar
//!
//! Ambas son totales: saturan en los extremos en vez de devolver `Result` o entrar en pánico. Una
//! marca temporal anterior al epoch, o absurdamente grande, la produciría un transporte que
//! entrega basura, y en ese caso rechazar el evento entero sería peor para el negocio del cliente
//! que ordenarlo mal. La saturación queda documentada aquí, no implícita en el código.

use std::time::{Duration, SystemTime};

/// Convierte un instante absoluto en milisegundos desde el epoch Unix.
///
/// Satura en `0` para cualquier instante anterior al epoch y en `i64::MAX` para cualquiera que no
/// quepa en el entero con signo de 64 bits que SQLite almacena.
pub fn a_milisegundos(instante: SystemTime) -> i64 {
    match instante.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(transcurrido) => i64::try_from(transcurrido.as_millis()).unwrap_or(i64::MAX),
        // Instante anterior al epoch: se satura en el propio epoch, que es el suelo del orden.
        Err(_) => 0,
    }
}

/// Reconstruye un instante absoluto a partir de milisegundos desde el epoch Unix.
///
/// Satura en el epoch para valores negativos, que no tienen representación en este esquema, y
/// también para el desbordamiento del tipo de instante de la plataforma: ese segundo caso no lo
/// alcanza ningún valor que este repositorio escriba —haría falta una marca de cientos de
/// millones de años— y se resuelve devolviendo el epoch en vez de inventar un instante futuro.
pub fn desde_milisegundos(milisegundos: i64) -> SystemTime {
    let magnitud = match u64::try_from(milisegundos) {
        Ok(valor) => valor,
        Err(_) => return SystemTime::UNIX_EPOCH,
    };

    match SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(magnitud)) {
        Some(instante) => instante,
        None => SystemTime::UNIX_EPOCH,
    }
}
