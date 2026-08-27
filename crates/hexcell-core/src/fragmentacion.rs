//! Módulo de fragmentación de contenido para el motor de conocimiento.
//!
//! Implementa una estrategia de troceado con solapamiento basada en ventanas
//! de caracteres Unicode, siguiendo el mismo principio que `estimar_coste` en
//! `presupuesto.rs`: medir en caracteres, no en bytes ni en tokens, para evitar
//! dependencias externas y garantizar la integridad de los puntos de código
//! Unicode (acentos, eñe, emojis).
//!
//! El tamaño del fragmento y el solapamiento son parámetros de la función,
//! tal como requiere el plan de la etapa A-5 ("parametrizada").
//!
//! La función no intenta divisiones semánticas ni por líneas; el límite
//! entre fragmentos es puramente basado en un recuento de caracteres. Esto
//! significa que un límite puede caer dentro de una línea de texto o un
//! elemento de lista, y ese comportamiento es documentado y probado explícitamente.
//! No se considera un error, sino una característica conocida de la estrategia
//! de ventana de caracteres.

use std::fmt;

/// Configuración para la fragmentación de texto.
///
/// Ambos campos se miden en caracteres Unicode (`chars().count()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfiguracionDeFragmentacion {
    /// Tamaño de cada fragmento en caracteres.
    pub tamano_de_fragmento: usize,
    /// Número de caracteres que se solapan entre fragmentos consecutivos.
    pub solapamiento: usize,
}

/// Errores que pueden ocurrir durante la fragmentación.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorDeFragmentacion {
    /// El solapamiento debe ser estrictamente menor que el tamaño del fragmento.
    SolapamientoNoMenorQueTamano {
        /// Tamaño del fragmento configurado.
        tamano_de_fragmento: usize,
        /// Valor de solapamiento configurado.
        solapamiento: usize,
    },
}

impl fmt::Display for ErrorDeFragmentacion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SolapamientoNoMenorQueTamano {
                tamano_de_fragmento,
                solapamiento,
            } => write!(
                f,
                "El solapamiento ({solapamiento}) debe ser estrictamente menor que el tamaño del fragmento ({tamano_de_fragmento})"
            ),
        }
    }
}

impl std::error::Error for ErrorDeFragmentacion {}

/// Fragmenta un texto en solapamientos de tamaño fijo medidos en caracteres Unicode.
///
/// # Algoritmo
/// 1. Valida que `solapamiento < tamano_de_fragmento`. Si no, devuelve `Err`.
/// 2. Convierte el texto en un vector de caracteres (`Vec<char>`) para operar
///    por puntos de código Unicode, evitando cortes en medio de un carácter
///    multi-byte (como acentos, eñe o emojis).
/// 3. Si el vector está vacío (texto de entrada vacío), devuelve un vector
///    vacío de fragmentos.
/// 4. Itera sobre el vector de caracteres con un paso de
///    `tamano_de_fragmento - solapamiento`:
///    - Toma un segmento desde `inicio` hasta `min(inicio + tamano_de_fragmento, len)`.
///    - Convierte ese segmento de caracteres de vuelta a `String`.
///    - Avanza `inicio` en `tamano_de_fragmento - solapamiento`.
///    - Detén el bucle cuando `inicio + tamano_de_fragmento` alcance o supere
///      la longitud total de caracteres.
/// 5. El último fragmento puede ser más corto que `tamano_de_fragmento` (resto
///    irregular), pero aún así solapará con el fragmento precedente por la cantidad
///    configurada siempre que haya suficientes caracteres anteriores.
///
/// # Por qué esta implementación
/// - **Caracteres, no bytes**: Al usar `chars().collect()` y rebanadas de `Vec<char>`
///   garantizamos que ningún punto de código Unicode se particiona, cumpliendo
///   con el requisito AC-6.
/// - **Parametrizado**: El tamaño y solapamiento vienen de la configuración, no
///   son constantes hardcodeadas, siguiendo el principio de la etapa A-5.
/// - **Sin dependencias**: Solo usa la biblioteca estándar, manteniendo la tabla
///   de dependencias de `hexcell-core` vacía (adr-0002).
/// - **Índice como ordinal futuro**: El vector devuelto mantiene el orden de
///   inserción, y su índice puede usarse como `fragmentos.ordinal` en la
///   tabla `fragmentos` sin riesgo de vacíos (cada fragmento empujado tiene
///   `fin > inicio` por construcción).
pub fn fragmentar(
    texto: &str,
    configuracion: &ConfiguracionDeFragmentacion,
) -> Result<Vec<String>, ErrorDeFragmentacion> {
    // Validar primero la configuración para evitar bucles infinitos o
    // asignaciones desproporcionadas.
    if configuracion.solapamiento >= configuracion.tamano_de_fragmento {
        return Err(ErrorDeFragmentacion::SolapamientoNoMenorQueTamano {
            tamano_de_fragmento: configuracion.tamano_de_fragmento,
            solapamiento: configuracion.solapamiento,
        });
    }

    // Convertir a vector de caracteres para operar por puntos de código Unicode.
    let caracteres: Vec<char> = texto.chars().collect();

    // Caso especial: entrada vacía produce cero fragmentos.
    if caracteres.is_empty() {
        return Ok(Vec::new());
    }

    let mut fragmentos = Vec::new();
    let mut inicio: usize = 0;
    let len = caracteres.len();

    loop {
        // Calcular el fin del fragmento actual, asegurando no pasarnos del límite.
        let fin = (inicio + configuracion.tamano_de_fragmento).min(len);
        // Construir el fragmento como String a partir del rango de caracteres.
        let fragmento: String = caracteres[inicio..fin].iter().collect();
        fragmentos.push(fragmento);

        // Si hemos alcanzado el final, salir del bucle.
        if fin == len {
            break;
        }

        // Avanzar el inicio para el siguiente fragmento, manteniendo el solapamiento.
        inicio += configuracion.tamano_de_fragmento - configuracion.solapamiento;
    }

    Ok(fragmentos)
}
