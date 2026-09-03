//! Módulo de recuperación de contexto RAG para el motor de conocimiento.
//!
//! Declaración de los tipos del dominio que representan las peticiones de recuperación
//! y sus resultados estructurados, soportados exclusivamente por la biblioteca estándar (`adr-0002`)
//! para preservar la tabla de dependencias vacía de `hexcell-core`.
//!
//! # Separación estricta entre recuperación y ensamblado de prompt (AC-6)
//!
//! Los tipos declarados en este módulo representan los fragmentos seleccionados como valores
//! estructurados e independientes (identificador, texto, similitud). No ofrecen ningún método
//! ni formateo para concatenarlos en una cadena de prompt final: esa responsabilidad pertenece
//! al adaptador de inferencia en una etapa posterior. Mantener esta frontera explícita es
//! load-bearing para la observabilidad, la comprobabilidad y la prevención de inyecciones de prompt.

/// Configuración de la consulta de recuperación de contexto RAG.
///
/// Define el límite máximo de fragmentos a seleccionar y el umbral mínimo de similitud
/// coseno aceptable para incluir un fragmento en el contexto devuelto.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfiguracionDeRecuperacion {
    /// Número máximo de fragmentos a devolver en la respuesta.
    pub maximo_de_fragmentos: usize,
    /// Umbral mínimo de similitud coseno (entre 0.0 y 1.0) para aceptar un fragmento.
    pub umbral_de_similitud: f32,
}

/// Fragmento recuperado del catálogo de conocimiento con su puntuación de relevancia.
#[derive(Clone, Debug, PartialEq)]
pub struct FragmentoRecuperado {
    /// Identificador único e intrínseco del fragmento en la base de datos de origen.
    pub id_fragmento: i64,
    /// Texto contenido en el fragmento.
    pub texto: String,
    /// Similitud coseno calculada entre el vector del fragmento y el vector de consulta.
    pub similitud: f32,
}

/// Contexto recuperado: colección ordenada de fragmentos relevantes para la consulta.
///
/// Encapsula el vector de resultados seleccionados por el motor de recuperación.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextoRecuperado {
    fragmentos: Vec<FragmentoRecuperado>,
}

impl ContextoRecuperado {
    /// Construye una nueva instancia de contexto recuperado envolviendo la lista de fragmentos.
    pub fn nuevo(fragmentos: Vec<FragmentoRecuperado>) -> Self {
        Self { fragmentos }
    }

    /// Devuelve una referencia a los fragmentos contenidos en este contexto.
    pub fn fragmentos(&self) -> &[FragmentoRecuperado] {
        &self.fragmentos
    }

    /// Indica si el contexto recuperado carece de fragmentos (está vacío).
    pub fn esta_vacio(&self) -> bool {
        self.fragmentos.is_empty()
    }
}

/// Ordena un vector de fragmentos recuperados por similitud descendente con desempate determinista.
///
/// # Criterio de ordenación (AC-2)
/// 1. Similitud coseno en orden descendente, evaluada mediante `f32::total_cmp`. Se prefiere
///    `total_cmp` sobre la comparación parcial ordinaria porque define un orden total sobre todos los valores `f32`
///    (incluyendo valores no finitos) garantizando que el ordenador nunca entre en pánico.
/// 2. Ante empate exacto de similitud, desempata por `id_fragmento` en orden ascendente.
///
/// # Por qué desempata por `id_fragmento` ascendente
/// El identificador de fila es la única clave intrínseca, estable y total que ofrece la época.
/// Apoyar el desempate en la clave primaria garantiza que la ordenación sea una función pura e
/// independiente del orden de las filas devuelto por SQLite o de la inestabilidad del algoritmo
/// de ordenación, eliminando comportamientos no deterministas en las pruebas (HEX-058, HEX-059).
pub fn ordenar_por_relevancia(fragmentos: &mut [FragmentoRecuperado]) {
    fragmentos.sort_by(|a, b| {
        b.similitud
            .total_cmp(&a.similitud)
            .then_with(|| a.id_fragmento.cmp(&b.id_fragmento))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordenar_por_relevancia_es_determinista_y_desempata_por_id_ascendente() {
        // Preparar fragmentos con similitudes idénticas pero IDs en orden no ascendente.
        let mut fragmentos = vec![
            FragmentoRecuperado {
                id_fragmento: 50,
                texto: "texto B".to_string(),
                similitud: 0.85,
            },
            FragmentoRecuperado {
                id_fragmento: 10,
                texto: "texto A".to_string(),
                similitud: 0.85,
            },
            FragmentoRecuperado {
                id_fragmento: 2,
                texto: "texto C".to_string(),
                similitud: 0.95,
            },
        ];

        ordenar_por_relevancia(&mut fragmentos);

        // El de mayor similitud (0.95, id 2) debe ir primero.
        // Entre los de similitud 0.85, el de menor id (10) debe preceder al de id (50).
        assert_eq!(fragmentos[0].id_fragmento, 2);
        assert_eq!(fragmentos[1].id_fragmento, 10);
        assert_eq!(fragmentos[2].id_fragmento, 50);

        // Una segunda ejecución con los mismos datos debe producir exactamente el mismo resultado.
        let mut fragmentos_copia = fragmentos.clone();
        ordenar_por_relevancia(&mut fragmentos_copia);
        assert_eq!(fragmentos, fragmentos_copia);
    }

    #[test]
    fn ordenar_por_relevancia_soporta_valores_no_finitos_sin_panico() {
        let mut fragmentos = vec![
            FragmentoRecuperado {
                id_fragmento: 1,
                texto: "normal".to_string(),
                similitud: 0.5,
            },
            FragmentoRecuperado {
                id_fragmento: 2,
                texto: "nan".to_string(),
                similitud: f32::NAN,
            },
        ];

        // total_cmp no entra en pánico al comparar NaN con valores finitos.
        ordenar_por_relevancia(&mut fragmentos);
        assert_eq!(fragmentos.len(), 2);
    }
}
