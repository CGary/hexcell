//! Adaptador `ChannelAdapter` simulado: implementación en memoria con semántica de Cloud API.
//!
//! Convención de entrega de eventos (`docs/adr/adr-0016-convencion-de-entrega-de-eventos.md`): el
//! trait `ChannelAdapter` de `hexcell-core` declara solo `send` y `estado_ventana` — el mecanismo
//! de entrega de `EventoEntrante` no es uno de los siete elementos de FR-12 y se decide en esta
//! misma etapa. Este adaptador crea y posee un canal `tokio::sync::mpsc` **acotado** — acotado
//! para que una ráfaga aplique contrapresión en vez de crecer sin límite contra el presupuesto de
//! memoria de NFR-01 — y entrega su extremo receptor al `Motor` en el momento de construirse. La
//! etapa A-3 (whatsmeow) ya cerrada adopta la misma convención: cada adaptador entrega sus eventos
//! por un canal propio, no por un método nuevo del trait.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use hexcell_core::canal::{
    ChannelAdapter, DURACION_VENTANA_SERVICIO, EstadoVentanaServicio, EventoEntrante,
    MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use hexcell_storage::{AlmacenDeIdentidad, ErrorDeAlmacen};
use tokio::sync::mpsc;

use crate::reloj::Reloj;

/// Avería de transporte del adaptador simulado.
///
/// No es `std::convert::Infallible` a propósito: un tipo de error deshabitado dejaría el brazo
/// `Err` del `Motor` inalcanzable en la práctica, y el propósito de este adaptador es precisamente
/// permitir que un test fuerce esa avería y compruebe que el motor la trata sin `unwrap()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorDelAdaptadorSimulado {
    /// Avería de transporte forzada a voluntad por el test mediante `forzar_averia()`.
    AveriaDeTransporteSimulada,
}

impl fmt::Display for ErrorDelAdaptadorSimulado {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AveriaDeTransporteSimulada => {
                write!(
                    f,
                    "avería de transporte simulada, forzada a propósito por el test"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDelAdaptadorSimulado {}

/// Fallo de `inyectar_desde_contacto`: o el canal ya se cerró, o el almacén de identidad no
/// respondió al resolver o registrar el contacto.
///
/// No se aplana en un solo caso: confundir un fallo de almacenamiento con uno de envío
/// enmascararía justo la corrupción que la tarea de respaldo y restauración existe para detectar.
#[derive(Debug)]
pub enum ErrorDeInyeccion {
    /// El canal `mpsc` hacia el `Motor` ya se cerró.
    Envio(mpsc::error::SendError<EventoEntrante>),
    /// El almacén de identidad del adaptador falló al resolver o registrar el contacto.
    Almacen(ErrorDeAlmacen),
}

impl fmt::Display for ErrorDeInyeccion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envio(error) => write!(f, "fallo al entregar el evento al motor: {error}"),
            Self::Almacen(error) => {
                write!(f, "fallo del almacén de identidad del adaptador: {error}")
            }
        }
    }
}

impl std::error::Error for ErrorDeInyeccion {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Envio(error) => Some(error),
            Self::Almacen(error) => Some(error),
        }
    }
}

impl From<mpsc::error::SendError<EventoEntrante>> for ErrorDeInyeccion {
    fn from(error: mpsc::error::SendError<EventoEntrante>) -> Self {
        Self::Envio(error)
    }
}

/// El mapa de contactos del adaptador: en memoria (comportamiento histórico de `nuevo`) o
/// persistido en el almacén de identidad propio del adaptador (`nuevo_con_almacen`, `adr-0010`).
///
/// Un enumerado y no dos structs distintos: solo `inyectar_desde_contacto` distingue el caso, y
/// el resto del adaptador —incluida la ventana de servicio y los envíos forzados— no sabe ni
/// necesita saber cuál de los dos está en uso.
enum AlmacenDeContactos {
    /// Comportamiento histórico: el mapa vive y muere con el proceso.
    EnMemoria(HashMap<String, IdConversacion>),
    /// Comportamiento persistente: el mapa vive en `adapter_identity.db`, y sobrevive a un
    /// reinicio o a una restauración. Compartido por `Arc` y no poseído, porque `Motor` toma
    /// posesión del adaptador y el respaldo de la célula todavía necesita el almacén después.
    Persistente(Arc<AlmacenDeIdentidad>),
}

/// Estado interno mutable del adaptador, agrupado para que un único `Mutex` lo proteja entero.
struct EstadoInterno {
    /// Ancla de la ventana de servicio de cada conversación: el instante del último evento
    /// entrante inyectado para ella.
    anclas_de_ventana: HashMap<IdConversacion, std::time::SystemTime>,
    /// Resultados forzados específicos de una conversación, consumidos uno por llamada.
    forzados_por_conversacion: HashMap<IdConversacion, VecDeque<ResultadoEnvio>>,
    /// Resultados forzados sin conversación asociada, consumidos uno por llamada a `send`.
    forzados_siguientes: VecDeque<ResultadoEnvio>,
    /// Si está activo, la próxima llamada a `send` devuelve `Err` y lo desactiva.
    forzar_averia: bool,
    /// Copia de cada envío realizado, para que el test la inspeccione con `envios_capturados()`.
    envios_capturados: Vec<(IdConversacion, MensajeSaliente, ResultadoEnvio)>,
    /// Almacén de identidad del adaptador (`adr-0010`, puntos 5 y 6): mapa de un contacto
    /// simulado a su identificador interno de conversación. Se declara clave por contacto **y
    /// nunca por dispositivo**, para que un re-emparejamiento —que solo cambia
    /// `dispositivo_actual`— deje este mapa intacto y el hilo de conversación sobreviva.
    contactos: AlmacenDeContactos,
    /// Identificador del dispositivo actualmente emparejado. Cambia con `re_emparejar` y no
    /// participa nunca como clave de `contactos`: es precisamente la credencial de sesión del
    /// transporte de la que `adr-0010` separa la identidad de contacto.
    dispositivo_actual: String,
}

/// Adaptador `ChannelAdapter` en memoria que imita la semántica restrictiva de la Cloud API.
///
/// Imita la Cloud API, **no** whatsmeow: la ventana de servicio de 24 horas, `PlantillaRequerida`
/// fuera de ella y los cuatro rechazos de FR-12 son el caso restrictivo que el puerto admite, no
/// una obligación del canal propio (`hexcell_core::canal`, distinción TIPO/POLÍTICA).
pub struct AdaptadorSimulado {
    reloj: Arc<dyn Reloj + Send + Sync>,
    remitente_eventos: mpsc::Sender<EventoEntrante>,
    estado: Mutex<EstadoInterno>,
}

impl AdaptadorSimulado {
    /// Crea el adaptador simulado y el receptor que el `Motor` debe consumir.
    ///
    /// `capacidad` acota el canal `mpsc`: por debajo de ella `inyectar` se completa de inmediato,
    /// por encima aplica contrapresión, igual que hará cualquier adaptador real.
    pub fn nuevo(
        reloj: Arc<dyn Reloj + Send + Sync>,
        capacidad: usize,
    ) -> (Self, mpsc::Receiver<EventoEntrante>) {
        Self::construir(
            reloj,
            capacidad,
            AlmacenDeContactos::EnMemoria(HashMap::new()),
        )
    }

    /// Crea el adaptador simulado con el almacén de identidad **persistente** del adaptador
    /// (`adr-0010`) en vez del mapa en memoria: el mismo contacto sigue resolviendo al mismo
    /// identificador interno después de un reinicio o de una restauración desde respaldo.
    ///
    /// `almacen` se comparte por `Arc`, no se posee: `Motor` toma posesión del adaptador y el
    /// respaldo de la célula sigue necesitando el almacén después de construirlo.
    pub fn nuevo_con_almacen(
        reloj: Arc<dyn Reloj + Send + Sync>,
        capacidad: usize,
        almacen: Arc<AlmacenDeIdentidad>,
    ) -> (Self, mpsc::Receiver<EventoEntrante>) {
        Self::construir(reloj, capacidad, AlmacenDeContactos::Persistente(almacen))
    }

    fn construir(
        reloj: Arc<dyn Reloj + Send + Sync>,
        capacidad: usize,
        contactos: AlmacenDeContactos,
    ) -> (Self, mpsc::Receiver<EventoEntrante>) {
        let (remitente_eventos, receptor_eventos) = mpsc::channel(capacidad);
        let adaptador = Self {
            reloj,
            remitente_eventos,
            estado: Mutex::new(EstadoInterno {
                anclas_de_ventana: HashMap::new(),
                forzados_por_conversacion: HashMap::new(),
                forzados_siguientes: VecDeque::new(),
                forzar_averia: false,
                envios_capturados: Vec::new(),
                contactos,
                dispositivo_actual: "dispositivo-inicial".to_string(),
            }),
        };
        (adaptador, receptor_eventos)
    }

    /// Inyecta un evento entrante de forma determinista: lo entrega al `Motor` por el canal y
    /// ancla (o refresca) la ventana de servicio de su conversación en `reloj.ahora()`.
    ///
    /// Devuelve un error si el canal ya se cerró (el `Motor` dejó de escuchar), que no es un caso
    /// que el simulado deba enmascarar.
    pub async fn inyectar(
        &self,
        evento: EventoEntrante,
    ) -> Result<(), mpsc::error::SendError<EventoEntrante>> {
        {
            let mut estado = self.estado.lock().expect(
                "el mutex interno de AdaptadorSimulado no debería estar envenenado en un test",
            );
            estado
                .anclas_de_ventana
                .insert(evento.conversacion.clone(), self.reloj.ahora());
        }
        self.remitente_eventos.send(evento).await
    }

    /// Inyecta un evento entrante que llega desde un contacto simulado, resolviendo (o creando)
    /// el identificador interno de conversación de ese contacto en el almacén de identidad del
    /// adaptador (`adr-0010`).
    ///
    /// A diferencia de `inyectar`, que recibe un `EventoEntrante` ya construido con la
    /// conversación que decide el test, este método es el que hace observable —y no vacía— la
    /// propiedad de AC-5: el mismo `contacto` siempre resuelve al mismo `IdConversacion`, pase lo
    /// que pase con `dispositivo_actual`, porque `contactos` se indexa solo por contacto.
    ///
    /// Con el almacén persistente (`nuevo_con_almacen`), el identificador de un contacto nuevo se
    /// acuña a partir de `contactos_registrados()` —cuántos contactos había ya, no del propio
    /// nombre del contacto— así que depende del **orden** en el que cada contacto se vio por
    /// primera vez. Es lo que hace observable que una restauración es real: un almacén vacío
    /// asignaría el mismo primer identificador que uno restaurado, pero no el segundo ni los
    /// siguientes.
    pub async fn inyectar_desde_contacto(
        &self,
        contacto: &str,
        contenido: impl Into<String>,
        deduplicacion: IdDeduplicacion,
    ) -> Result<IdConversacion, ErrorDeInyeccion> {
        let evento = {
            let mut estado = self.estado.lock().expect(
                "el mutex interno de AdaptadorSimulado no debería estar envenenado en un test",
            );

            let conversacion = match &mut estado.contactos {
                AlmacenDeContactos::EnMemoria(mapa) => mapa
                    .entry(contacto.to_string())
                    .or_insert_with(|| IdConversacion::nuevo(format!("conversacion-de-{contacto}")))
                    .clone(),
                AlmacenDeContactos::Persistente(almacen) => {
                    let existente = almacen
                        .buscar(contacto)
                        .map_err(ErrorDeInyeccion::Almacen)?;
                    match existente {
                        Some(identificador) => IdConversacion::nuevo(identificador),
                        None => {
                            let orden_de_llegada = almacen
                                .contactos_registrados()
                                .map_err(ErrorDeInyeccion::Almacen)?;
                            // El PRIMER contacto que ve un almacén vacío no puede, por
                            // construcción, distinguirse de un almacén restaurado que solo tuviera
                            // ese mismo contacto: los dos le asignan la posición cero. Por eso el
                            // sufijo de orden se añade a partir del SEGUNDO contacto en adelante,
                            // que es donde un almacén vacío y uno restaurado sí divergen. El primer
                            // contacto conserva el formato histórico `conversacion-de-{contacto}`
                            // (el mismo que ya usaba el mapa en memoria), y `main.rs` depende de
                            // ese formato exacto para su único evento sintético de arranque.
                            let identificador = if orden_de_llegada == 0 {
                                format!("conversacion-de-{contacto}")
                            } else {
                                format!("conversacion-de-{contacto}-{orden_de_llegada}")
                            };
                            almacen
                                .registrar(contacto, &identificador)
                                .map_err(ErrorDeInyeccion::Almacen)?;
                            IdConversacion::nuevo(identificador)
                        }
                    }
                }
            };

            let ahora = self.reloj.ahora();
            estado.anclas_de_ventana.insert(conversacion.clone(), ahora);

            EventoEntrante {
                remitente: IdRemitente::nuevo(contacto),
                conversacion,
                contenido: contenido.into(),
                marca_temporal: ahora,
                deduplicacion,
            }
        };
        let conversacion_asignada = evento.conversacion.clone();
        self.remitente_eventos.send(evento).await?;
        Ok(conversacion_asignada)
    }

    /// Re-empareja el adaptador con un dispositivo nuevo: cambia `dispositivo_actual` y deja el
    /// mapa `contactos` completamente intacto.
    ///
    /// Esto es, literalmente, lo que un re-emparejamiento significa para el adaptador simulado:
    /// el dispositivo vinculado cambia, pero ningún contacto cambia de hilo por ello. El mapa de
    /// identidad vive separado de la credencial de dispositivo precisamente para que esto sea
    /// cierto (`adr-0010`, puntos 5 y 6).
    pub fn re_emparejar(&self, dispositivo_nuevo: impl Into<String>) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.dispositivo_actual = dispositivo_nuevo.into();
    }

    /// Identificador del dispositivo actualmente emparejado, para que un test observe que
    /// `re_emparejar` lo cambió de verdad.
    pub fn dispositivo_actual(&self) -> String {
        let estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.dispositivo_actual.clone()
    }

    /// Encola un resultado forzado para una conversación concreta; se consume en la próxima
    /// llamada a `send` sobre esa misma conversación, y solo en esa llamada.
    pub fn forzar(&self, conversacion: &IdConversacion, resultado: ResultadoEnvio) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado
            .forzados_por_conversacion
            .entry(conversacion.clone())
            .or_default()
            .push_back(resultado);
    }

    /// Encola un resultado forzado para la próxima llamada a `send`, sea cual sea la conversación.
    pub fn forzar_siguiente(&self, resultado: ResultadoEnvio) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.forzados_siguientes.push_back(resultado);
    }

    /// Hace que la próxima llamada a `send` devuelva `Err`, una única vez.
    pub fn forzar_averia(&self) {
        let mut estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.forzar_averia = true;
    }

    /// Instantánea de cada envío realizado hasta ahora, en el orden en que ocurrieron.
    pub fn envios_capturados(&self) -> Vec<(IdConversacion, MensajeSaliente, ResultadoEnvio)> {
        let estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        estado.envios_capturados.clone()
    }
}

impl ChannelAdapter for AdaptadorSimulado {
    type Error = ErrorDelAdaptadorSimulado;

    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        let resultado = {
            let mut estado = self.estado.lock().expect(
                "el mutex interno de AdaptadorSimulado no debería estar envenenado en un test",
            );

            if estado.forzar_averia {
                estado.forzar_averia = false;
                return Err(ErrorDelAdaptadorSimulado::AveriaDeTransporteSimulada);
            }

            let forzado = estado
                .forzados_por_conversacion
                .get_mut(conversacion)
                .and_then(VecDeque::pop_front)
                .or_else(|| estado.forzados_siguientes.pop_front());

            let resultado = forzado.unwrap_or_else(|| {
                let ahora = self.reloj.ahora();
                match &mensaje {
                    MensajeSaliente::Plantilla { .. } => ResultadoEnvio::Aceptado,
                    MensajeSaliente::RespuestaLibre(_) => {
                        match estado.anclas_de_ventana.get(conversacion) {
                            Some(ancla) if ahora >= *ancla + DURACION_VENTANA_SERVICIO => {
                                ResultadoEnvio::FueraDeVentana
                            }
                            _ => ResultadoEnvio::Aceptado,
                        }
                    }
                }
            });

            estado
                .envios_capturados
                .push((conversacion.clone(), mensaje.clone(), resultado));

            resultado
        };

        Ok(resultado)
    }

    async fn estado_ventana(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<EstadoVentanaServicio, Self::Error> {
        let estado = self
            .estado
            .lock()
            .expect("el mutex interno de AdaptadorSimulado no debería estar envenenado en un test");
        let ahora = self.reloj.ahora();
        match estado.anclas_de_ventana.get(conversacion) {
            Some(ancla) if ahora < *ancla + DURACION_VENTANA_SERVICIO => {
                Ok(EstadoVentanaServicio::Abierta {
                    expira_en: *ancla + DURACION_VENTANA_SERVICIO,
                })
            }
            _ => Ok(EstadoVentanaServicio::Cerrada),
        }
    }
}
