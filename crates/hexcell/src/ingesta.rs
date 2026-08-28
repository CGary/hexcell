//! Ingesta y orquestación del catálogo de conocimiento.
//!
//! Este módulo implementa el servicio de aplicación asíncrono para coordinar la fragmentación,
//! obtención de vectores de incrustación e inserción por lotes en la base de datos en sombra.
//! La orquestación corre en el hilo asíncrono de la célula (hexcell) y delega la persistencia
//! síncrona a la capa de almacenamiento (hexcell-storage) para respetar los límites de rusqlite.
//!
//! Diseñado el 28 de agosto de 2026 para cumplir con las directrices de contabilidad en dos fases.

use std::fmt;
use std::path::Path;

use hexcell_core::embeddings::PeticionDeEmbeddings;
use hexcell_core::fragmentacion::{ConfiguracionDeFragmentacion, fragmentar};
use hexcell_storage::{ConstructorDeConocimientoEnSombra, DocumentoDeIngesta};

use crate::embeddings::{ProveedorDeEmbeddingsDeCelula, ServicioDeEmbeddings};

/// Desviación o resultado final del proceso de ingesta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesenlaceDeIngesta {
    /// Todos los fragmentos del documento fueron indexados con sus vectores.
    Completa,
    /// Algunos fragmentos del documento fueron indexados y otros fallaron sin abortar la ejecución.
    Parcial,
    /// La ejecución fue cancelada en un límite de lote por la señal de apagado.
    DetenidaPorApagado,
    /// No se logró obtener ningún vector válido durante todo el proceso de ingesta.
    SinIncrustaciones,
}

/// Resumen de los contadores y resultado final de la ingesta para diagnóstico.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumenDeIngesta {
    pub fragmentos_solicitados: usize,
    pub fragmentos_escritos: usize,
    pub lotes_emitidos: usize,
    pub dimension_observada: Option<usize>,
    pub desenlace: DesenlaceDeIngesta,
}

/// Fallo estructural en la ejecución de la ingesta.
#[derive(Debug)]
pub enum ErrorDeIngesta {
    /// Error surgido en el algoritmo de fragmentación de caracteres Unicode.
    Fragmentacion(hexcell_core::fragmentacion::ErrorDeFragmentacion),
    /// Error surgido al operar la base de datos en sombra.
    Almacen(hexcell_storage::ErrorDeAlmacen),
    /// Error estructural del proveedor de embeddings o fallo de presupuesto.
    Embeddings(String),
}

impl fmt::Display for ErrorDeIngesta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fragmentacion(e) => write!(f, "fallo de fragmentación: {e}"),
            Self::Almacen(e) => write!(f, "fallo de persistencia en la base en sombra: {e}"),
            Self::Embeddings(msg) => write!(f, "fallo en el servicio de embeddings: {msg}"),
        }
    }
}

impl std::error::Error for ErrorDeIngesta {}

/// Ejecuta la ingesta asíncrona de un documento en la base de datos de conocimiento en sombra.
///
/// Realiza el troceado del contenido, la obtención de vectores respetando el tamaño de lote
/// del adaptador y la persistencia atómica por lotes.
pub async fn ejecutar_ingesta<F>(
    documento: DocumentoDeIngesta,
    config_fragmentacion: ConfiguracionDeFragmentacion,
    servicio_embeddings: &ServicioDeEmbeddings<ProveedorDeEmbeddingsDeCelula>,
    ruta_datos: &Path,
    debe_apagar: F,
) -> Result<ResumenDeIngesta, ErrorDeIngesta>
where
    F: Fn() -> bool,
{
    // Se fragmenta el documento en caracteres Unicode para asegurar cortes limpios sin romper emojis.
    let fragmentos = fragmentar(&documento.contenido, &config_fragmentacion)
        .map_err(ErrorDeIngesta::Fragmentacion)?;

    let fragmentos_solicitados = fragmentos.len();

    // Se inicializa el constructor en sombra. Esto destruye físicamente cualquier archivo previo
    // para evitar arrastrar residuos consistentes de ejecuciones interrumpidas.
    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &documento)
        .map_err(ErrorDeIngesta::Almacen)?;

    // Se extrae el tamaño de lote configurado para el adaptador activo a través del despachador.
    let tamano_lote = servicio_embeddings.tamano_de_lote();

    // Se fuerza que el tamaño de lote sea al menos 1 para evitar un pánico por división
    // entre cero al recorrer el vector en particiones de ese tamaño, sin depender de
    // validaciones externas.
    let tamano_lote = tamano_lote.max(1);

    let mut fragmentos_escritos = 0;
    let mut lotes_emitidos = 0;
    let mut dimension_observada = None;
    let mut detenido_por_apagado = false;

    // Se recorre el vector de fragmentos en particiones consecutivas de tamaño `tamano_lote`,
    // avanzando el índice de inicio manualmente para no atarse a un único método concreto
    // de partición de la biblioteca estándar.
    let mut inicio_de_particion = 0usize;
    while inicio_de_particion < fragmentos.len() {
        // La comprobación de apagado se realiza exclusivamente en la frontera del lote.
        // Esto impide dejar reservas activas colgadas a mitad de un lote en el repositorio de sesiones.
        if debe_apagar() {
            detenido_por_apagado = true;
            break;
        }

        let fin_de_particion = (inicio_de_particion + tamano_lote).min(fragmentos.len());
        let porcion = &fragmentos[inicio_de_particion..fin_de_particion];

        let ordinal_inicial = inicio_de_particion;
        let peticion = PeticionDeEmbeddings {
            textos: porcion.to_vec(),
        };

        lotes_emitidos += 1;
        let marca_temporal = std::time::SystemTime::now();

        // Se invoca el servicio de embeddings que encapsula la reserva previa y la conciliación.
        match servicio_embeddings
            .incrustar_lote(peticion, marca_temporal)
            .await
        {
            Ok(respuesta) => {
                let mut lote_a_escribir = Vec::with_capacity(respuesta.vectores.len());
                for (desplazamiento, vector_opcional) in respuesta.vectores.into_iter().enumerate()
                {
                    if let Some(vector) = vector_opcional {
                        let ordinal = ordinal_inicial + desplazamiento;
                        let texto = porcion[desplazamiento].clone();
                        lote_a_escribir.push((ordinal, texto, vector.valores().to_vec()));

                        if dimension_observada.is_none() {
                            dimension_observada = Some(vector.dimension());
                        }
                    }
                }

                if !lote_a_escribir.is_empty() {
                    fragmentos_escritos += lote_a_escribir.len();
                    constructor
                        .escribir_lote_de_fragmentos(&lote_a_escribir)
                        .map_err(ErrorDeIngesta::Almacen)?;
                }
            }
            Err(e) => {
                // Cualquier error estructural (incluyendo saldo insuficiente) se propaga
                // inmediatamente para abortar la ingesta incompleta.
                return Err(ErrorDeIngesta::Embeddings(e.to_string()));
            }
        }

        inicio_de_particion = fin_de_particion;
    }

    // Se consolida el resultado cerrando el constructor.
    constructor.finalizar().map_err(ErrorDeIngesta::Almacen)?;

    let desenlace = if detenido_por_apagado {
        DesenlaceDeIngesta::DetenidaPorApagado
    } else if fragmentos_escritos == 0 {
        DesenlaceDeIngesta::SinIncrustaciones
    } else if fragmentos_escritos == fragmentos_solicitados {
        DesenlaceDeIngesta::Completa
    } else {
        DesenlaceDeIngesta::Parcial
    };

    Ok(ResumenDeIngesta {
        fragmentos_solicitados,
        fragmentos_escritos,
        lotes_emitidos,
        dimension_observada,
        desenlace,
    })
}
