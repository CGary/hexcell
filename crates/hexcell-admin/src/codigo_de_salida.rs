//! Contrato tipado de códigos de salida del proceso `hexcell-admin`.
//!
//! Esta tarea es la segunda de tres hijas de la tarea 10 de la etapa A-6 (esqueleto de la CLI
//! `hexcell-admin`). Fija el vocabulario cerrado de desenlaces del proceso y su conversión hacia
//! [`std::process::ExitCode`]. El analizador de argumentos, los seis subcomandos y el modo de
//! simulación son trabajo de la tarea hermana HEX-074-c, que consume este contrato: ninguno de
//! ellos se construye aquí.
//!
//! El repositorio hoy solo usa `ExitCode::SUCCESS` y `ExitCode::FAILURE`. Este módulo introduce
//! deliberadamente un contrato más rico: un código de uso incorrecto, distinguible de un fallo de
//! ejecución real, y un código reservado para un subcomando declarado pero todavía no
//! implementado, de modo que el operador pueda distinguir por el número de salida qué clase de
//! problema tuvo, sin depurador ni bitácora.

/// Los cuatro desenlaces posibles de una invocación de `hexcell-admin`.
///
/// Enumerado cerrado a propósito (sin `#[non_exhaustive]`), siguiendo el precedente de
/// `EstadoDeCelula` en `estado_de_celula.rs`: las pruebas externas de este mismo crate
/// (`tests/codigo_de_salida.rs`) necesitan poder emparejar sobre él sin un brazo por defecto, de
/// modo que añadir o quitar una variante rompa esa compilación en vez de caer en silencio en una
/// reacción genérica.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodigoDeSalida {
    /// El comando terminó con éxito.
    Exito,
    /// Fallo genérico: el comando no pudo completar su tarea.
    Fallo,
    /// El operador invocó el comando de forma incorrecta (subcomando desconocido, argumento
    /// faltante o mal formado, etc.). Distinto de [`Self::Fallo`] para que el operador pueda
    /// distinguir un error de invocación de un fallo de ejecución real.
    UsoIncorrecto,
    /// El subcomando existe en la superficie declarada pero todavía no tiene comportamiento real
    /// detrás. Reservado para que la tarea hermana HEX-074-c pueda anunciar una superficie de CLI
    /// completa sin implementar cada comando de una sola vez.
    NoImplementadoTodavia,
}

impl CodigoDeSalida {
    /// El valor numérico `u8` asociado a esta variante.
    ///
    /// Coincidencia exhaustiva con cuatro brazos y ningún brazo por defecto: añadir una quinta
    /// variante al enumerado sin extender esta función deja de compilar, en vez de asignarle en
    /// silencio un número no revisado.
    pub fn codigo(self) -> u8 {
        match self {
            CodigoDeSalida::Exito => 0,
            CodigoDeSalida::Fallo => 1,
            CodigoDeSalida::UsoIncorrecto => 2,
            CodigoDeSalida::NoImplementadoTodavia => 3,
        }
    }
}

/// Conversión hacia el código de salida real del proceso, siempre a través de
/// `ExitCode::from(u8)`.
///
/// Enrutar la conversión por el valor numérico de [`CodigoDeSalida::codigo`] en vez de mapear
/// cada variante a mano contra `ExitCode::SUCCESS` / `ExitCode::FAILURE` es la garantía de que el
/// contrato numérico documentado y el código de salida real del proceso no puedan divergir: solo
/// hay un lugar donde vive el número de cada variante.
impl From<CodigoDeSalida> for std::process::ExitCode {
    fn from(codigo: CodigoDeSalida) -> Self {
        std::process::ExitCode::from(codigo.codigo())
    }
}
