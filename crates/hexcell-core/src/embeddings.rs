//! Puerto de incrustaciones vectoriales `ProveedorDeEmbeddings`: frontera del dominio de conocimiento.
//!
//! Declara la operación de generación de vectores de incrustación (*embeddings*) sobre fragmentos
//! de texto ordenados, consumida por el proceso de ingesta del catálogo de conocimiento (etapa A-5).
//! Todo el módulo se apoya exclusivamente en la biblioteca estándar (`adr-0002`), preservando la
//! tabla de dependencias vacía de `hexcell-core`.
//!
//! # Por qué el método se declara `-> impl Future` y no `async fn`
//!
//! Por la misma razón documentada en `crate::inferencia` y `crate::canal`: sobre rustc 1.92.0,
//! `async fn` dentro de un trait dispara el aviso `async_fn_in_trait`, que
//! `cargo clippy --workspace -- -D warnings` convierte en error de compilación. Retornar
//! `impl Future<Output = ...> + Send` evita el aviso sin silenciarlo y fija la cota `Send`
//! requerida para la ejecución asíncrona. Como consecuencia directa, el trait no es compatible
//! con objetos de trait (`dyn`), por lo que se consume de forma genérica o mediante enumeraciones
//! de selección estática, nunca como puntero dinámico.
//!
//! # Correspondencia posicional y gestión de resultados parciales
//!
//! `RespuestaDeEmbeddings` garantiza estructuralmente que la longitud de su vector `vectores`
//! coincide con la cantidad de textos solicitados en `PeticionDeEmbeddings`. Cada posición `i`
//! corresponde al texto `i` de la petición. Un elemento `None` representa un fragmento no resuelto
//! en el intento actual, permitiendo modelar respuestas parciales sin desalinear los índices.
//!
//! # Disposición binaria de los vectores
//!
//! [`VectorDeEmbedding`] serializa sus componentes de punto flotante en formato IEEE-754 `binary32`
//! en orden *little-endian* sin cabecera ni relleno, cumpliendo el contrato normativo documentado en
//! `crates/hexcell-storage/migraciones/conocimiento/0002-esquema-de-conocimiento.sql`.

use std::fmt;

use crate::presupuesto::UnidadesDePresupuesto;

/// Vector de incrustación (*embedding*): secuencia ordenada de valores numéricos de punto flotante.
///
/// Encapsula un vector `Vec<f32>` garantizando la conversión determinista hacia y desde su
/// representación binaria en almacenamiento.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorDeEmbedding(Vec<f32>);

impl VectorDeEmbedding {
    /// Construye un nuevo vector de incrustación a partir de sus componentes de punto flotante.
    pub fn nuevo(valores: Vec<f32>) -> Self {
        Self(valores)
    }

    /// Devuelve una referencia a la secuencia de valores numéricos del vector.
    pub fn valores(&self) -> &[f32] {
        &self.0
    }

    /// Devuelve la dimensión del vector (cantidad de componentes de punto flotante).
    pub fn dimension(&self) -> usize {
        self.0.len()
    }

    /// Serializa el vector como una secuencia continua de bytes en formato IEEE-754 *little-endian*.
    ///
    /// No incluye cabecera, prefijo de longitud ni relleno. La longitud en bytes resultante es
    /// exactamente `4 * dimension()`.
    pub fn a_bytes_le(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.0.len() * 4);
        for valor in &self.0 {
            bytes.extend_from_slice(&valor.to_le_bytes());
        }
        bytes
    }

    /// Reconstruye un vector a partir de una secuencia de bytes en formato IEEE-754 *little-endian*.
    ///
    /// Devuelve `None` si la longitud del bloque de bytes no es múltiplo exacto de 4.
    pub fn desde_bytes_le(bytes: &[u8]) -> Option<Self> {
        if !bytes.len().is_multiple_of(4) {
            return None;
        }
        let cantidad = bytes.len() / 4;
        let mut valores = Vec::with_capacity(cantidad);
        for fragmento in bytes.chunks_exact(4) {
            let mut arreglo = [0u8; 4];
            arreglo.copy_from_slice(fragmento);
            valores.push(f32::from_le_bytes(arreglo));
        }
        Some(Self(valores))
    }
}

/// Petición de incrustaciones: lote ordenado de fragmentos de texto a procesar en una llamada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeticionDeEmbeddings {
    /// Textos ordenados para los cuales se solicita la generación de vectores.
    pub textos: Vec<String>,
}

/// Respuesta de incrustaciones: vectores generados correspondientes a la petición.
#[derive(Clone, Debug, PartialEq)]
pub struct RespuestaDeEmbeddings {
    /// Vectores resultantes ordenados en correspondencia biunívoca con los textos de entrada.
    ///
    /// Cada posición `i` contiene `Some(vector)` si el fragmento fue procesado con éxito, o
    /// `None` si quedó pendiente o no fue devuelto por el proveedor en este intento.
    pub vectores: Vec<Option<VectorDeEmbedding>>,
    /// Cantidad real de unidades de presupuesto consumidas durante la operación.
    pub unidades_consumidas: UnidadesDePresupuesto,
}

/// Error al integrar una respuesta parcial dentro de un acumulador de lote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDeIntegracion {
    /// La cantidad de vectores en la respuesta no coincide con la cantidad de índices pendientes.
    LongitudIncompatible {
        /// Cantidad de índices enviados.
        esperado: usize,
        /// Cantidad de vectores devueltos en la respuesta.
        recibido: usize,
    },
    /// Un índice indicado excede los límites de fragmentos del lote.
    IndiceFueraDeRango(usize),
    /// Se intentó integrar un resultado sobre una posición que ya había sido resuelta previamente.
    IndiceYaResuelto(usize),
}

impl fmt::Display for ErrorDeIntegracion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LongitudIncompatible { esperado, recibido } => {
                write!(
                    f,
                    "longitud incompatible al integrar lote: se esperaban {esperado} elementos pero se recibieron {recibido}"
                )
            }
            Self::IndiceFueraDeRango(idx) => {
                write!(f, "índice de fragmento {idx} fuera de rango en el lote")
            }
            Self::IndiceYaResuelto(idx) => {
                write!(
                    f,
                    "el fragmento en la posición {idx} ya contaba con un vector resuelto"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDeIntegracion {}

/// Acumulador ordenado para la gestión de reanudación y completado de lotes de incrustaciones.
///
/// Mantiene la lista completa de textos originales y un vector de resultados parciales.
/// Permite extraer exclusivamente los fragmentos pendientes con sus índices de origen,
/// garantizando que los fragmentos ya resueltos no vuelvan a solicitarse ni a presupuestarse.
#[derive(Clone, Debug, PartialEq)]
pub struct LoteDeEmbeddings {
    textos: Vec<String>,
    acumulador: Vec<Option<VectorDeEmbedding>>,
}

impl LoteDeEmbeddings {
    /// Inicializa un nuevo lote de incrustaciones con la lista ordenada de textos.
    pub fn nuevo(textos: Vec<String>) -> Self {
        let cantidad = textos.len();
        Self {
            textos,
            acumulador: vec![None; cantidad],
        }
    }

    /// Referencia a la lista completa de textos del lote original.
    pub fn textos(&self) -> &[String] {
        &self.textos
    }

    /// Cantidad de fragmentos que aún no tienen vector asignado.
    pub fn pendientes(&self) -> usize {
        self.acumulador.iter().filter(|v| v.is_none()).count()
    }

    /// Indica si todos los fragmentos del lote han sido resueltos satisfactoriamente.
    pub fn esta_completo(&self) -> bool {
        self.acumulador.iter().all(|v| v.is_some())
    }

    /// Genera la petición de fragmentos pendientes junto con sus índices originales.
    ///
    /// Si todos los fragmentos ya están resueltos, devuelve `None`.
    pub fn peticion_pendiente(&self) -> Option<(PeticionDeEmbeddings, Vec<usize>)> {
        let mut textos_pendientes = Vec::new();
        let mut indices = Vec::new();

        for (idx, (texto, slot)) in self.textos.iter().zip(self.acumulador.iter()).enumerate() {
            if slot.is_none() {
                textos_pendientes.push(texto.clone());
                indices.push(idx);
            }
        }

        if indices.is_empty() {
            None
        } else {
            Some((
                PeticionDeEmbeddings {
                    textos: textos_pendientes,
                },
                indices,
            ))
        }
    }

    /// Integra una respuesta parcial en el acumulador asignando los vectores a sus posiciones.
    ///
    /// Rechaza la integración si la longitud de `respuesta.vectores` difiere de `indices.len()`,
    /// si algún índice es inválido o si apunta a una posición previamente completada.
    pub fn integrar(
        &mut self,
        indices: &[usize],
        respuesta: RespuestaDeEmbeddings,
    ) -> Result<(), ErrorDeIntegracion> {
        if respuesta.vectores.len() != indices.len() {
            return Err(ErrorDeIntegracion::LongitudIncompatible {
                esperado: indices.len(),
                recibido: respuesta.vectores.len(),
            });
        }

        for (&idx, opt_vector) in indices.iter().zip(respuesta.vectores) {
            if idx >= self.acumulador.len() {
                return Err(ErrorDeIntegracion::IndiceFueraDeRango(idx));
            }
            if let Some(vector) = opt_vector {
                if self.acumulador[idx].is_some() {
                    return Err(ErrorDeIntegracion::IndiceYaResuelto(idx));
                }
                self.acumulador[idx] = Some(vector);
            }
        }

        Ok(())
    }

    /// Consume el acumulador y devuelve los vectores si todos los elementos están resueltos.
    ///
    /// Si aún restan fragmentos pendientes, devuelve `None`.
    pub fn completo(self) -> Option<Vec<VectorDeEmbedding>> {
        let mut resultado = Vec::with_capacity(self.acumulador.len());
        for opt in self.acumulador {
            match opt {
                Some(v) => resultado.push(v),
                None => return None,
            }
        }
        Some(resultado)
    }
}

/// Puerto de incrustaciones vectoriales: todo proveedor externo se implementa tras este trait.
pub trait ProveedorDeEmbeddings {
    /// Tipo de error devuelto ante anomalías de transporte, formato o red.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Genera vectores de incrustación para un lote ordenado de textos.
    fn incrustar_lote(
        &self,
        peticion: PeticionDeEmbeddings,
    ) -> impl Future<Output = Result<RespuestaDeEmbeddings, Self::Error>> + Send;
}
