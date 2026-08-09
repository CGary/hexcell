//! Puerto de canal `ChannelAdapter`: la frontera entre el núcleo y el transporte de WhatsApp.
//!
//! Aquí solo hay **declaración**. Ningún adaptador se implementa en esta etapa: el de whatsmeow
//! llega en la etapa A-3 y el simulado, junto con la batería de tests de contrato, en la A-2.
//!
//! # Qué normaliza el puerto
//!
//! Los siete elementos que enumera `docs/PRD.md` (FR-12), ni uno más: evento entrante canónico,
//! envío tipado, resultado tipado del envío, estado de la ventana de servicio, identidad de
//! conversación (en el módulo [`crate::identidad`]), acuses normalizados y ciclo de vida de
//! sesión como sub-trait opcional.
//!
//! # La regla que hace viable la convivencia
//!
//! El puerto se abstrae **hacia el caso más restrictivo**, que es la Meta Cloud API, con esta
//! distinción: **el TIPO admite el resultado restrictivo; la POLÍTICA de cada adaptador decide
//! si lo produce**. Que [`ChannelAdapter::send`] pueda devolver [`ResultadoEnvio::FueraDeVentana`]
//! obliga al núcleo a saber reaccionar, pero **no obliga al adaptador del canal propio a imponer
//! una ventana de 24 horas artificial**: ese adaptador no produce ese resultado porque su
//! transporte no lo impone. Los dos canales conviven en células distintas del mismo servidor.
//!
//! El cotejo de cada variante contra la documentación oficial de la Cloud API vive en
//! `docs/cotejo-puerto-de-canal-cloud-api.md`, porque cotejar solo contra el PRD trasladaría
//! intacto cualquier error del PRD.
//!
//! # Por qué los métodos se escriben con `-> impl Future`
//!
//! No se usa la forma abreviada asíncrona dentro del trait. Sobre rustc 1.92.0 dispara el aviso
//! `async_fn_in_trait`, activo por omisión, que `cargo clippy --workspace -- -D warnings`
//! convierte en error. Escribir el retorno como `impl Future<Output = ...> + Send` evita el
//! aviso sin silenciarlo y, además, permite declarar hoy la cota `Send` que el consumidor de la
//! etapa A-2 necesitará para lanzar la tarea. El coste está registrado en
//! `docs/adr/adr-0002-estructura-workspace.md`: el trait no es compatible con objetos de trait,
//! de modo que `Box<dyn ChannelAdapter>` no compila y la selección de canal es estática.

use std::time::{Duration, SystemTime};

use crate::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

/// Duración de la ventana de servicio del caso restrictivo: 24 horas.
///
/// Se nombra una sola vez y aquí para que ningún adaptador la reinvente. Sobre canal propio no
/// se usa: ese transporte no impone ninguna ventana y su adaptador no la fabrica.
pub const DURACION_VENTANA_SERVICIO: Duration = Duration::from_secs(24 * 60 * 60);

/// Evento entrante canónico (FR-12, elemento 1).
///
/// Es lo que el adaptador entrega al núcleo tras normalizar lo que llegó por su transporte: un
/// webhook verificado de la Meta Graph API o un mensaje del websocket de whatsmeow. Todos sus
/// identificadores están **ya traducidos**; ninguno es un identificador de transporte.
///
/// En esta etapa el tipo se declara y no se consume: el mecanismo de entrega —suscripción,
/// flujo o retrollamada— no es uno de los siete elementos de FR-12 y se decide en la etapa A-2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventoEntrante {
    /// Quién escribió, en identidad interna.
    pub remitente: IdRemitente,
    /// A qué hilo pertenece el mensaje, en identidad interna.
    pub conversacion: IdConversacion,
    /// Contenido textual ya normalizado.
    pub contenido: String,
    /// Momento del evento según el transporte, normalizado a tiempo absoluto.
    pub marca_temporal: SystemTime,
    /// Identificador para descartar reentregas del mismo evento.
    pub deduplicacion: IdDeduplicacion,
}

/// Mensaje saliente tipado (FR-12, elemento 2).
///
/// La distinción no es cosmética: fuera de la ventana de servicio, la Cloud API solo acepta
/// plantillas previamente aprobadas. Un `String` suelto no podría expresar esa diferencia y
/// obligaría al núcleo a adivinarla.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MensajeSaliente {
    /// Texto libre. Es lo que el canal propio envía siempre.
    RespuestaLibre(String),
    /// Plantilla previamente aprobada, con sus parámetros posicionales.
    Plantilla {
        /// Nombre de la plantilla tal y como está aprobada en el canal.
        id: String,
        /// Valores de los parámetros variables, en orden.
        parametros: Vec<String>,
    },
}

/// Resultado tipado del envío (FR-12, elemento 3).
///
/// `send()` no devuelve un booleano ni un error opaco: enumera los fallos del caso restrictivo,
/// y el núcleo debe distinguirlos porque cada uno exige una reacción distinta. Ninguno de ellos
/// es un fallo de programación, y por eso viajan como resultado del dominio y no como error del
/// tipo asociado [`ChannelAdapter::Error`], que queda reservado a las averías del transporte.
///
/// El enumerado se declara **cerrado a propósito**, sin atributo que lo abra: así, un crate
/// externo que lo consuma —incluidas las pruebas de `tests/`— puede recorrerlo con un `match`
/// sin brazo comodín, y añadir o quitar una variante rompe la compilación de esas pruebas. Un
/// enumerado abierto obligaría a un brazo comodín y anularía exactamente esa garantía.
///
/// El conjunto de variantes lo fija FR-12 y **no se amplía aquí**: ampliarlo es una decisión de
/// producto sobre el PRD. La brecha detectada al cotejar contra la documentación oficial queda
/// registrada como hallazgo abierto en `docs/cotejo-puerto-de-canal-cloud-api.md` y como
/// decisión pendiente en `docs/STATUS.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultadoEnvio {
    /// El canal aceptó el mensaje para su entrega. El desenlace llega después, por [`Acuse`].
    Aceptado,
    /// La ventana de servicio está cerrada para esa conversación.
    FueraDeVentana,
    /// El canal exige una plantilla aprobada y se le entregó texto libre.
    PlantillaRequerida,
    /// El canal está limitando la tasa de envío.
    LimiteDeTasa,
    /// El destinatario no es válido o no puede recibir el mensaje.
    DestinatarioInvalido,
}

/// Estado de la ventana de servicio por conversación (FR-12, elemento 4).
///
/// El núcleo consulta el mismo contrato sea cual sea el canal. Sobre whatsmeow la
/// implementación es trivial —siempre [`EstadoVentanaServicio::Abierta`], porque el transporte
/// no impone ninguna ventana—, y fabricar una restricción que el transporte no tiene sería
/// degradar el producto para parecerse a un canal que la célula no usa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstadoVentanaServicio {
    /// Abierta hasta el momento indicado.
    Abierta {
        /// Instante en que la ventana se cierra.
        expira_en: SystemTime,
    },
    /// Cerrada: solo se admite una plantilla aprobada.
    Cerrada,
}

/// Acuse normalizado del ciclo de vida de un mensaje saliente (FR-12, elemento 6).
///
/// La semántica es la misma sea cual sea el canal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acuse {
    /// El canal aceptó el mensaje y lo puso en camino.
    Enviado,
    /// El dispositivo del destinatario lo recibió.
    Entregado,
    /// El destinatario lo leyó.
    Leido,
    /// La entrega falló de forma definitiva.
    Fallido,
}

/// Datos de emparejamiento que devuelve el sub-trait de ciclo de vida de sesión.
///
/// Solo existen en los canales que necesitan vincular un dispositivo. La persistencia de las
/// credenciales resultantes **no** aparece en el puerto: es asunto interno del adaptador
/// (`adr-0010`, punto 6), y exponerla aquí metería en el núcleo un dato de transporte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Emparejamiento {
    /// Contenido a codificar como QR para que el usuario lo escanee.
    CodigoQr(String),
    /// Código de vinculación que el usuario teclea en su teléfono.
    CodigoDeVinculacion(String),
}

/// Representa los cuatro estados de sesión de la conexión de WhatsApp del sidecar.
/// Solo `Activa` significa que la célula puede procesar mensajes.
/// El detalle específico de transporte del wire (causa, codigo, expira_en_ms del protocolo IPC)
/// NO pertenece aquí — se queda dentro del crate del adaptador porque ponerlo en el puerto
/// empujaría el conocimiento del transporte hacia el núcleo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EstadoSesion {
    /// Sesión de WhatsApp operativa; la célula puede procesar mensajes.
    Activa,
    /// Desconexión transitoria con reintentos en curso.
    Reconectando,
    /// Sesión inválida por cierre o eliminación de dispositivo; requiere recuperación humana.
    Desvinculada,
    /// Baneo temporal detectado; no hay reactivación automática.
    Pausada,
}

/// Puerto de canal: toda integración de WhatsApp se implementa detrás de este trait.
///
/// El núcleo lo consume sin saber qué hay debajo, y por eso sumar un canal es escribir un
/// adaptador y no reescribir el producto (FR-12; `adr-0010`).
///
/// El tipo asociado [`ChannelAdapter::Error`] transporta **averías**: el socket que se cayó, el
/// sidecar que no responde, la respuesta que no se pudo interpretar. Los cuatro fallos que FR-12
/// enumera no son averías, son desenlaces del dominio, y viajan dentro de [`ResultadoEnvio`].
pub trait ChannelAdapter {
    /// Avería del transporte, ajena a los desenlaces de dominio de [`ResultadoEnvio`].
    type Error: std::error::Error + Send + Sync + 'static;

    /// Envía un mensaje tipado a una conversación y devuelve el resultado tipado.
    ///
    /// La conversación se identifica con el identificador interno, ya traducido por el propio
    /// adaptador; el núcleo nunca construye uno a partir de un dato de transporte.
    fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> impl Future<Output = Result<ResultadoEnvio, Self::Error>> + Send;

    /// Consulta el estado de la ventana de servicio de una conversación.
    fn estado_ventana(
        &self,
        conversacion: &IdConversacion,
    ) -> impl Future<Output = Result<EstadoVentanaServicio, Self::Error>> + Send;
}

/// Ciclo de vida de sesión (FR-12, elemento 7): sub-trait **opcional**.
///
/// Se declara aparte y **no** como supertrait de [`ChannelAdapter`] por una razón concreta: si
/// fuera supertrait, el adaptador de la Cloud API tendría que implementarlo para nada, y acabaría
/// devolviendo errores en métodos que su transporte no necesita. Separado, sencillamente no lo
/// implementa. Solo lo implementan los adaptadores que vinculan un dispositivo.
pub trait CicloDeVidaSesion {
    /// Avería del transporte durante las operaciones de sesión.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Inicia el emparejamiento y devuelve lo que el usuario debe escanear o teclear.
    fn iniciar_emparejamiento(
        &self,
    ) -> impl Future<Output = Result<Emparejamiento, Self::Error>> + Send;

    /// Cierra la sesión y desvincula el dispositivo.
    fn cerrar_sesion(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Consulta el estado actual de la sesión del canal.
    fn estado_sesion(&self) -> EstadoSesion;
}
