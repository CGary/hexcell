//! Puerto de notificación operativa `SumideroDeNotificaciones`: la frontera entre el motor y quien
//! avisa a un humano de que algo pasó en la célula.
//!
//! Sigue el precedente de `crate::inferencia` y `crate::embeddings`, no el de `crate::canal`:
//! `ChannelAdapter` ganó crates propios porque FR-12 exige dos adaptadores vivos a la vez en
//! células distintas del mismo servidor, con pruebas de contrato cruzadas. Una notificación
//! operativa tiene un único sumidero real (Telegram) más un doble de prueba, exactamente la forma
//! de `ProveedorDeInferencia`: el puerto vive aquí, en el núcleo, y las dos implementaciones viven
//! en `crates/hexcell`, que sí puede depender de un cliente HTTP.
//!
//! # Qué NO lleva esta versión, y por qué
//!
//! [`CodigoDeNotificacion`] es un identificador opaco, no una enumeración de las ocho condiciones
//! de alerta (baneo temporal, sesión desvinculada, etc.): enumerarlas aquí implementaría el
//! alcance de la tarea hermana HEX-077-b dentro de esta. Tampoco lleva severidad, política de
//! reintento, backoff ni ventana de deduplicación: escribir esas firmas antes de que exista un
//! consumidor real es exactamente D-09 en `docs/bitacora-de-descartes.md`.
//!
//! # Por qué el método se declara `-> impl Future` y no `async fn`
//!
//! La misma razón que ya documentan `crate::canal` y `crate::inferencia`: sobre rustc 1.92.0,
//! `async fn` dentro de un trait dispara el aviso `async_fn_in_trait`, activo por omisión, que
//! `cargo clippy --workspace -- -D warnings` convierte en error. El trait resultante no es
//! compatible con objetos de trait (`dyn`); en `crates/hexcell` se consume mediante la
//! enumeración de selección estática `SumideroDeCelula`, nunca como `Box<dyn
//! SumideroDeNotificaciones>`.

use std::time::SystemTime;

use crate::identidad::IdConversacion;

/// Código opaco que identifica qué condición de alerta motiva la notificación.
///
/// Deliberadamente **no** es una enumeración cerrada de las ocho condiciones de alerta: esa
/// enumeración pertenece a HEX-077-b, que es quien conoce las condiciones concretas. Este tipo
/// solo transporta el valor que el consumidor real elija.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodigoDeNotificacion(String);

impl CodigoDeNotificacion {
    /// Construye el código a partir de un valor ya decidido por el consumidor.
    pub fn nuevo(valor: impl Into<String>) -> Self {
        Self(valor.into())
    }

    /// Vista prestada del valor opaco.
    pub fn como_str(&self) -> &str {
        &self.0
    }
}

/// Contenedor de una célula cuyo bucle de reintento importa distinguir en una alerta de reinicio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponenteDeCelula {
    /// El contenedor del núcleo Rust.
    Nucleo,
    /// El contenedor del sidecar Go (whatsmeow).
    Sidecar,
}

/// Dato tipado que puede llevar una notificación, cerrado a propósito.
///
/// Siguiendo el criterio de `ResultadoEnvio` en `crate::canal`: quien haga `match` sobre esta
/// enumeración lo hace sin brazo comodín, así que añadir una variante es un error de compilación
/// en cada consumidor, no una omisión silenciosa en producción.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValorDeDato {
    /// Texto libre, para lo que no encaja en las otras variantes.
    Texto(String),
    /// Un instante, por ejemplo la fecha de expiración de un baneo temporal.
    Instante(SystemTime),
    /// Una conversación afectada, siempre como identificador interno opaco (`adr-0010`,
    /// `adr-0019`); nunca un identificador de transporte crudo.
    Conversacion(IdConversacion),
    /// Qué contenedor de la célula protagoniza la condición, por ejemplo cuál entró en bucle de
    /// reinicio.
    Componente(ComponenteDeCelula),
}

/// Un dato con nombre dentro del payload de una notificación.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatoDeNotificacion {
    /// Clave descriptiva del dato, elegida por quien construye la notificación.
    pub clave: String,
    /// Valor tipado del dato.
    pub valor: ValorDeDato,
}

/// Notificación operativa: un código opaco más un payload de datos tipados.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notificacion {
    /// Código que identifica la condición de alerta que motiva la notificación.
    pub codigo: CodigoDeNotificacion,
    /// Datos tipados asociados a la notificación, en el orden en que se añadieron.
    pub datos: Vec<DatoDeNotificacion>,
}

impl Notificacion {
    /// Construye una notificación vacía de datos a partir de su código.
    pub fn nueva(codigo: CodigoDeNotificacion) -> Self {
        Self {
            codigo,
            datos: Vec::new(),
        }
    }

    /// Añade un dato con nombre y devuelve la notificación, para encadenar la construcción.
    #[must_use]
    pub fn con_dato(mut self, clave: impl Into<String>, valor: ValorDeDato) -> Self {
        self.datos.push(DatoDeNotificacion {
            clave: clave.into(),
            valor,
        });
        self
    }
}

/// Puerto de notificación: todo sumidero de avisos operativos se implementa detrás de este trait.
pub trait SumideroDeNotificaciones {
    /// Avería del sumidero: la llamada de red falló, la respuesta no fue exitosa, etc.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Entrega una notificación al sumidero.
    fn notificar(
        &self,
        notificacion: Notificacion,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
