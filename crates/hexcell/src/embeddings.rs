//! Servicio y selector de proveedores de incrustaciones vectoriales (*embeddings*).
//!
//! Agrupa tres componentes del binario:
//!
//! 1. [`ProveedorDeEmbeddingsSimulado`]: implementación determinista sin red basada en la huella FNV-1a.
//! 2. [`ProveedorDeEmbeddingsDeCelula`]: selector estático por enumeración para despachar entre la
//!    implementación simulada y el adaptador OpenRouter real, permitiendo incorporar futuras
//!    variantes (HEX-051-b) como adición pura sin alterar el puerto ni reestructurar el enum.
//! 3. [`ServicioDeEmbeddings`]: envoltorio de contabilidad financiera en dos fases que ejecuta
//!    la reserva previa atómica por llamada (`reservar_presupuesto_de_ingesta`), la conciliación
//!    posterior contra el uso reportado (`conciliar_presupuesto`) y la liberación ante fallos
//!    (`liberar_presupuesto`).

use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use hexcell_core::embeddings::{
    PeticionDeEmbeddings, ProveedorDeEmbeddings, RespuestaDeEmbeddings, VectorDeEmbedding,
};
use hexcell_core::presupuesto::estimar_coste_de_lote;
use hexcell_storage::{
    ErrorDeAlmacen, RepositorioDeSesiones, ResultadoDeResolucion, VeredictoDeReserva,
};

use crate::registro::{EntradaDeRegistro, NivelDeRegistro, emitir};

/// Dimensión por defecto de los vectores generados por el proveedor simulado.
const DIMENSION_SIMULADA_POR_DEFECTO: usize = 4;

/// Avería del proveedor de incrustaciones simulado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDeEmbeddingsSimulado {
    /// Avería forzada a propósito por un test mediante `ProveedorDeEmbeddingsSimulado::que_falla`.
    AveriaSimulada,
}

impl fmt::Display for ErrorDeEmbeddingsSimulado {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AveriaSimulada => {
                write!(
                    f,
                    "avería de embeddings simulada, forzada a propósito por el test"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDeEmbeddingsSimulado {}

/// Proveedor de incrustaciones determinista sin acceso a red para pruebas y desarrollo.
#[derive(Clone, Debug)]
pub struct ProveedorDeEmbeddingsSimulado {
    dimension: usize,
    forzar_averia: bool,
    limite_elementos: Option<usize>,
    consumo_personalizado: Option<u64>,
}

impl Default for ProveedorDeEmbeddingsSimulado {
    fn default() -> Self {
        Self {
            dimension: DIMENSION_SIMULADA_POR_DEFECTO,
            forzar_averia: false,
            limite_elementos: None,
            consumo_personalizado: None,
        }
    }
}

impl ProveedorDeEmbeddingsSimulado {
    /// Construye un proveedor simulado con dimensión estándar de 4 componentes.
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Construye un proveedor simulado con una dimensión vectorial fija personalizada.
    pub fn con_dimension(dimension: usize) -> Self {
        Self {
            dimension,
            forzar_averia: false,
            limite_elementos: None,
            consumo_personalizado: None,
        }
    }

    /// Construye un proveedor simulado configurado para fallar incondicionalmente.
    pub fn que_falla() -> Self {
        Self {
            dimension: DIMENSION_SIMULADA_POR_DEFECTO,
            forzar_averia: true,
            limite_elementos: None,
            consumo_personalizado: None,
        }
    }

    /// Limita la cantidad de elementos devueltos en la respuesta para emular respuestas parciales.
    pub fn con_limite_elementos(mut self, limite: usize) -> Self {
        self.limite_elementos = Some(limite);
        self
    }

    /// Fija una cantidad personalizada de unidades consumidas a reportar en la respuesta.
    pub fn con_consumo_personalizado(mut self, unidades: u64) -> Self {
        self.consumo_personalizado = Some(unidades);
        self
    }
}

impl ProveedorDeEmbeddings for ProveedorDeEmbeddingsSimulado {
    type Error = ErrorDeEmbeddingsSimulado;

    async fn incrustar_lote(
        &self,
        peticion: PeticionDeEmbeddings,
    ) -> Result<RespuestaDeEmbeddings, Self::Error> {
        if self.forzar_averia {
            return Err(ErrorDeEmbeddingsSimulado::AveriaSimulada);
        }

        let cantidad = peticion.textos.len();
        let mut vectores = Vec::with_capacity(cantidad);
        let tope = self.limite_elementos.unwrap_or(cantidad).min(cantidad);

        for (i, texto) in peticion.textos.iter().enumerate() {
            if i < tope {
                let huella = crate::inferencia::huella_determinista(texto);
                let mut componentes = Vec::with_capacity(self.dimension);
                for d in 0..self.dimension {
                    let factor =
                        huella.wrapping_add((d as u64).wrapping_mul(0x517c_c1b7_2722_0a95));
                    componentes.push(((factor & 0xFFFF) as f32) / 65535.0);
                }
                vectores.push(Some(VectorDeEmbedding::nuevo(componentes)));
            } else {
                vectores.push(None);
            }
        }

        let unidades_consumidas = self
            .consumo_personalizado
            .unwrap_or_else(|| estimar_coste_de_lote(&peticion.textos));

        Ok(RespuestaDeEmbeddings {
            vectores,
            unidades_consumidas,
        })
    }
}

/// Error unificado devuelto por el selector de proveedor de embeddings de la célula.
#[derive(Debug)]
pub enum ErrorDeEmbeddingsDeCelula {
    /// Error devuelto por el proveedor simulado.
    Simulado(ErrorDeEmbeddingsSimulado),
    /// Error devuelto por el proveedor OpenRouter HTTPS.
    OpenRouter(crate::proveedor_embeddings::ErrorDeProveedorDeEmbeddings),
}

impl fmt::Display for ErrorDeEmbeddingsDeCelula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simulado(e) => write!(f, "{e}"),
            Self::OpenRouter(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErrorDeEmbeddingsDeCelula {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Simulado(e) => Some(e),
            Self::OpenRouter(e) => Some(e),
        }
    }
}

/// Selector estático del proveedor de embeddings (simulado o real OpenRouter).
///
/// Permite despachar llamadas polimórficas sin recurrir a objetos de trait dinámicos (`dyn`).
#[derive(Clone)]
pub enum ProveedorDeEmbeddingsDeCelula {
    /// Variante simulada determinista sin llamadas de red.
    Simulado(ProveedorDeEmbeddingsSimulado),
    /// Variante de red sobre la API compatible de OpenRouter.
    OpenRouter(Box<crate::proveedor_embeddings::ProveedorDeEmbeddingsOpenRouter>),
}

impl ProveedorDeEmbeddings for ProveedorDeEmbeddingsDeCelula {
    type Error = ErrorDeEmbeddingsDeCelula;

    async fn incrustar_lote(
        &self,
        peticion: PeticionDeEmbeddings,
    ) -> Result<RespuestaDeEmbeddings, Self::Error> {
        match self {
            Self::Simulado(proveedor) => proveedor
                .incrustar_lote(peticion)
                .await
                .map_err(ErrorDeEmbeddingsDeCelula::Simulado),
            Self::OpenRouter(proveedor) => proveedor
                .incrustar_lote(peticion)
                .await
                .map_err(ErrorDeEmbeddingsDeCelula::OpenRouter),
        }
    }
}

/// Avería producida durante la ejecución de una llamada de incrustación bajo contabilidad financiera.
#[derive(Debug)]
pub enum ErrorDeServicioDeEmbeddings<E> {
    /// El saldo disponible resultó insuficiente para cubrir la estimación previa del lote.
    PresupuestoAgotado {
        /// Saldo disponible en el momento de la comprobación.
        disponible: i64,
        /// Monto requerido por la estimación previa.
        requerido: i64,
    },
    /// El proveedor de incrustaciones subyacente devolvió un error de red o formato.
    Proveedor(E),
    /// Error de persistencia en el repositorio de sesiones.
    Almacen(ErrorDeAlmacen),
}

impl<E: fmt::Display> fmt::Display for ErrorDeServicioDeEmbeddings<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PresupuestoAgotado {
                disponible,
                requerido,
            } => {
                write!(
                    f,
                    "saldo de presupuesto insuficiente para embeddings: disponible {disponible}, requerido {requerido}"
                )
            }
            Self::Proveedor(err) => write!(f, "error del proveedor de embeddings: {err}"),
            Self::Almacen(err) => write!(f, "error de persistencia en embeddings: {err}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ErrorDeServicioDeEmbeddings<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PresupuestoAgotado { .. } => None,
            Self::Proveedor(err) => Some(err),
            Self::Almacen(err) => Some(err),
        }
    }
}

/// Servicio de aplicación que envuelve un [`ProveedorDeEmbeddings`] con contabilidad financiera en dos fases.
pub struct ServicioDeEmbeddings<P>
where
    P: ProveedorDeEmbeddings,
{
    proveedor: P,
    repositorio: Arc<RepositorioDeSesiones>,
}

impl<P> ServicioDeEmbeddings<P>
where
    P: ProveedorDeEmbeddings,
{
    /// Construye una nueva instancia del servicio vinculando el proveedor y el repositorio de sesiones.
    pub fn nuevo(proveedor: P, repositorio: Arc<RepositorioDeSesiones>) -> Self {
        Self {
            proveedor,
            repositorio,
        }
    }

    /// Ejecuta la generación de incrustaciones para un lote aplicando reserva y conciliación atómica.
    ///
    /// Flujo de ejecución:
    /// 1. Calcula la estimación de coste para los textos del lote vía [`estimar_coste_de_lote`].
    /// 2. Solicita la reserva de ingesta vía [`RepositorioDeSesiones::reservar_presupuesto_de_ingesta`].
    ///    Si es rechazada, aborta sin emitir peticiones HTTP y devuelve [`ErrorDeServicioDeEmbeddings::PresupuestoAgotado`].
    /// 3. Invoca `incrustar_lote` sobre el proveedor.
    /// 4. Ante éxito (`Ok`), concilia la reserva con las unidades reales o contra la estimación si faltan metadatos.
    /// 5. Ante error (`Err`), libera la reserva íntegra para no bloquear saldo y propaga la avería.
    pub async fn incrustar_lote(
        &self,
        peticion: PeticionDeEmbeddings,
        marca_temporal: SystemTime,
    ) -> Result<RespuestaDeEmbeddings, ErrorDeServicioDeEmbeddings<P::Error>> {
        let estimacion = estimar_coste_de_lote(&peticion.textos);

        let id_reserva = match self
            .repositorio
            .reservar_presupuesto_de_ingesta(estimacion, marca_temporal)
        {
            Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) => id_reserva,
            Ok(VeredictoDeReserva::Rechazada {
                disponible,
                requerido,
            }) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "presupuesto_rechazado")
                        .con_detalle(format!("requerido: {requerido}, disponible: {disponible}")),
                );
                return Err(ErrorDeServicioDeEmbeddings::PresupuestoAgotado {
                    disponible,
                    requerido,
                });
            }
            Err(error) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                        .con_detalle(format!("fallo al reservar presupuesto de ingesta: {error}")),
                );
                return Err(ErrorDeServicioDeEmbeddings::Almacen(error));
            }
        };

        match self.proveedor.incrustar_lote(peticion).await {
            Ok(respuesta) => {
                let unidades_a_conciliar = if respuesta.unidades_consumidas > 0 {
                    respuesta.unidades_consumidas
                } else {
                    emitir(
                        EntradaDeRegistro::nueva(
                            NivelDeRegistro::Aviso,
                            "embeddings_uso_ausente",
                        )
                        .con_detalle(
                            "metadatos de uso ausentes en respuesta de embeddings; conciliando contra estimación previa",
                        ),
                    );
                    estimacion
                };

                match self.repositorio.conciliar_presupuesto(
                    id_reserva,
                    unidades_a_conciliar,
                    marca_temporal,
                ) {
                    Ok(ResultadoDeResolucion::Resuelta {
                        deficit_no_cubierto,
                        ..
                    }) => {
                        if deficit_no_cubierto > 0 {
                            emitir(
                                EntradaDeRegistro::nueva(
                                    NivelDeRegistro::Aviso,
                                    "presupuesto_deficit_no_cubierto",
                                )
                                .con_detalle(format!("déficit no cubierto: {deficit_no_cubierto}")),
                            );
                        }
                    }
                    Ok(ResultadoDeResolucion::ReservaNoActiva) => {}
                    Err(error) => {
                        emitir(
                            EntradaDeRegistro::nueva(
                                NivelDeRegistro::Error,
                                "fallo_de_persistencia",
                            )
                            .con_detalle(format!(
                                "fallo al conciliar presupuesto de embeddings: {error}"
                            )),
                        );
                    }
                }

                Ok(respuesta)
            }
            Err(averia) => {
                if let Err(error) = self
                    .repositorio
                    .liberar_presupuesto(id_reserva, marca_temporal)
                {
                    emitir(
                        EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                            .con_detalle(format!(
                                "fallo al liberar presupuesto de embeddings: {error}"
                            )),
                    );
                }
                Err(ErrorDeServicioDeEmbeddings::Proveedor(averia))
            }
        }
    }
}
