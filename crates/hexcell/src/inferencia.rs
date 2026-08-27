//! Proveedor de inferencia simulado: implementación determinista de `ProveedorDeInferencia`.
//!
//! Vive como módulo de este binario y no como un octavo crate del workspace: nada fuera de
//! `crates/hexcell` lo consume. `hexcell-canal-simulado` sí ganó su propio crate porque
//! `hexcell-canal-contrato` lo consume independientemente del binario; promover este módulo a
//! crate, si algún día hace falta, es mecánico.
//!
//! # Por qué la respuesta no es un eco
//!
//! La respuesta es una huella FNV-1a de 64 bits del contenido de la petición, formateada como
//! texto, y deliberadamente **no** el contenido de entrada repetido. Un eco no se puede distinguir
//! de un valor fijo escrito a mano en el procesador, y AC-4 exige justo eso: que un test pruebe que
//! la respuesta salió del proveedor y no de `ProcesadorDeEco`. Por construcción, no por promesa:
//!
//! * Sin `rand`: nada de esta función depende de una fuente de aleatoriedad.
//! * Sin leer ningún reloj, ni de pared ni monotónico: nada de esta función consulta la hora.
//! * Sin el hasher por defecto de la biblioteca estándar: su salida no es estable entre procesos,
//!   así que dos ejecuciones del mismo binario podrían no coincidir; FNV-1a sí lo es, por
//!   construcción.
//! * Sin orden de iteración de ningún `HashMap`: la huella se calcula byte a byte, en el orden en
//!   que el contenido llega.
//!
//! La latencia artificial opcional (`Duration`, por defecto cero) no cambia ninguna salida y por
//! tanto no debilita ese determinismo: con cero no se crea ningún temporizador, y con un valor
//! positivo solo retrasa cuándo llega la misma respuesta. Existe para que el test de apagado
//! ordenado (AC-7) pueda demostrar que un evento en vuelo se completa: sin ella, la inferencia
//! simulada responde en microsegundos y un SIGTERM enviado justo después de inyectar casi siempre
//! llegaría con el evento ya persistido, y el criterio sería indistinguible de una implementación
//! que trunca el trabajo en curso.
//!
//! # Metadatos de consumo deterministas
//!
//! `ProveedorSimulado::generar` calcula `unidades_consumidas` como
//! `estimar_coste(&peticion.contenido) + estimar_coste(&contenido_de_respuesta)`.
//! Este valor excede deliberadamente la estimación previa calculada solo sobre el prompt, lo que
//! permite ejercitar la rama de déficit de la conciliación en la ruta ordinaria sin necesidad de
//! esperar a la llegada del proveedor real.

use std::fmt;
use std::time::Duration;

use hexcell_core::identidad::IdConversacion;
use hexcell_core::inferencia::{
    PeticionDeInferencia, ProveedorDeInferencia, RespuestaDeInferencia,
};
use hexcell_core::presupuesto::estimar_coste;

/// Desplazamiento inicial del FNV-1a de 64 bits (constante del algoritmo, no arbitraria).
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
/// Primo del FNV-1a de 64 bits (constante del algoritmo, no arbitraria).
const FNV_PRIME: u64 = 0x100000001b3;

/// Calcula la huella FNV-1a de 64 bits de una cadena, sin ninguna dependencia externa.
///
/// El algoritmo recorre cada byte de la entrada, lo combina por XOR con el acumulador y multiplica
/// por el primo fijo: ni aleatorio, ni dependiente del reloj, ni del orden de un `HashMap`. La
/// misma entrada produce siempre la misma huella, en cualquier proceso.
pub fn huella_determinista(contenido: &str) -> u64 {
    let mut huella = FNV_OFFSET_BASIS;
    for byte in contenido.as_bytes() {
        huella ^= u64::from(*byte);
        huella = huella.wrapping_mul(FNV_PRIME);
    }
    huella
}

/// Avería del proveedor simulado. No es `std::convert::Infallible` a propósito: un tipo de error
/// deshabitado dejaría el brazo `Err` del consumidor inalcanzable, y el propósito de este tipo es
/// precisamente que un test pueda forzar el fallo y comprobar que ni el motor ni el procesador
/// entran en pánico ni inventan una respuesta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDeInferenciaSimulada {
    /// Avería forzada a voluntad por el test mediante `ProveedorSimulado::forzar_averia`.
    AveriaSimulada,
}

impl fmt::Display for ErrorDeInferenciaSimulada {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AveriaSimulada => {
                write!(
                    f,
                    "avería de inferencia simulada, forzada a propósito por el test"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDeInferenciaSimulada {}

/// Proveedor de inferencia determinista, sin llamada de red, para tests y para el binario
/// mientras no exista un proveedor real (etapa A-4).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProveedorSimulado {
    /// Latencia artificial antes de responder. Cero por defecto: no crea ningún temporizador y no
    /// cambia ninguna salida.
    latencia: Duration,
    /// Si está activo, la próxima llamada a `generar` devuelve `Err` y lo desactiva.
    forzar_averia: bool,
}

impl ProveedorSimulado {
    /// Proveedor simulado sin latencia artificial ni avería forzada.
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Proveedor simulado con una latencia artificial fija antes de cada respuesta.
    ///
    /// Con `Duration::ZERO` no se crea ningún temporizador: la comprobación se hace antes de
    /// llamar a `tokio::time::sleep`, así que el caso por defecto no paga ningún coste.
    pub fn con_latencia(latencia: Duration) -> Self {
        Self {
            latencia,
            forzar_averia: false,
        }
    }

    /// Proveedor simulado que siempre falla, para que un test compruebe que el motor y el
    /// procesador tratan la avería sin `unwrap()` y sin inventar una respuesta.
    ///
    /// No hay mutador de un proveedor ya construido: `generar` recibe `&self`, así que la avería
    /// se fija en la construcción y no cambia a media ejecución, igual de determinista que el
    /// resto del tipo.
    pub fn que_falla() -> Self {
        Self {
            latencia: Duration::ZERO,
            forzar_averia: true,
        }
    }
}

impl ProveedorDeInferencia for ProveedorSimulado {
    type Error = ErrorDeInferenciaSimulada;

    async fn generar(
        &self,
        peticion: PeticionDeInferencia,
    ) -> Result<RespuestaDeInferencia, Self::Error> {
        if !self.latencia.is_zero() {
            tokio::time::sleep(self.latencia).await;
        }

        if self.forzar_averia {
            return Err(ErrorDeInferenciaSimulada::AveriaSimulada);
        }

        let huella = huella_determinista(&peticion.contenido);
        let _conversacion: &IdConversacion = &peticion.conversacion;
        let contenido_de_respuesta = format!("respuesta simulada {huella:016x}");
        let unidades_consumidas =
            estimar_coste(&peticion.contenido) + estimar_coste(&contenido_de_respuesta);
        Ok(RespuestaDeInferencia {
            contenido: contenido_de_respuesta,
            unidades_consumidas,
        })
    }
}

/// Error unificado devuelto por el selector de proveedor de inferencia de la célula.
#[derive(Debug)]
pub enum ErrorDeProveedorDeCelula {
    /// Error devuelto por la inferencia simulada.
    Simulado(ErrorDeInferenciaSimulada),
    /// Error devuelto por la inferencia real del proveedor OpenAI.
    OpenAi(crate::proveedor_openai::ErrorDeProveedorOpenAi),
}

impl fmt::Display for ErrorDeProveedorDeCelula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simulado(e) => write!(f, "{e}"),
            Self::OpenAi(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ErrorDeProveedorDeCelula {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Simulado(e) => Some(e),
            Self::OpenAi(e) => Some(e),
        }
    }
}

/// Selector estático del proveedor de inferencia (simulado o real OpenAI-compatible).
///
/// Dado que `ProveedorDeInferencia` retorna `impl Future` y por tanto no es compatible con
/// objetos de trait (`dyn`), esta enumeración permite seleccionar el proveedor activo en
/// la raíz de composición sin duplicar la construcción del motor.
#[derive(Clone)]
pub enum ProveedorDeCelula {
    /// Proveedor de inferencia simulada sin llamada de red.
    Simulado(ProveedorSimulado),
    /// Proveedor de inferencia HTTPS real sobre la API de OpenAI.
    OpenAi(Box<crate::proveedor_openai::ProveedorOpenAi>),
}

impl ProveedorDeInferencia for ProveedorDeCelula {
    type Error = ErrorDeProveedorDeCelula;

    async fn generar(
        &self,
        peticion: PeticionDeInferencia,
    ) -> Result<RespuestaDeInferencia, Self::Error> {
        match self {
            Self::Simulado(proveedor) => proveedor
                .generar(peticion)
                .await
                .map_err(ErrorDeProveedorDeCelula::Simulado),
            Self::OpenAi(proveedor) => proveedor
                .generar(peticion)
                .await
                .map_err(ErrorDeProveedorDeCelula::OpenAi),
        }
    }
}
