//! Servicio de aplicación `AdaptadorWhatsmeow`: cliente IPC que implementa `ChannelAdapter` y
//! `CicloDeVidaSesion`.
//!
//! El adaptador conecta al socket Unix del sidecar (que escucha), ejecuta el saludo de versión,
//! y a partir de ahí lee mensajes entrantes en una tarea de fondo. Los eventos entrantes se
//! entregan al núcleo a través de un `tokio::sync::mpsc` acotado, siguiendo la convención de
//! `adr-0016`. El estado de sesión se difunde por un `tokio::sync::watch`.
//!
//! # Envío por IPC (tarea 12, 2026-08-09)
//!
//! `send` reenvía por el cable v4 hacia la cola de salida del sidecar y devuelve `Aceptado`
//! cuando el frame quedó escrito: «aceptado para entrega posterior», que la cola materializa
//! con TTL absoluto y reintentos acotados. El puente provisional en memoria de HEX-015 quedó
//! sustituido; su registro histórico vive en `adr-0011`.
//!
//! # Política de ventana de servicio
//!
//! `estado_ventana` **siempre** responde `Abierta`: este transporte no impone ninguna ventana
//! de 24 horas y fabricar una sería degradar el producto para parecerse a un canal que la célula
//! no usa (`adr-0010`, distinción TIPO/POLÍTICA).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, watch};

use hexcell_core::canal::{
    ChannelAdapter, Emparejamiento, EstadoSesion, EstadoVentanaServicio, EventoEntrante,
    MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

use crate::conexion::Conexion;
use crate::error::ErrorCanalWhatsmeow;
use crate::mensajes::MensajeEntrante;
use crate::reconexion::Retroceso;

/// Límite duro de conversaciones distintas que [`MarcasDeOrigen`] retiene en memoria.
///
/// Sin este límite el mapa crece sin cota con el número de conversaciones distintas a lo largo
/// de la vida del proceso: es memoria de proceso, no una tabla con purga, así que una célula de
/// larga duración lo convertiría en una fuga lenta contra el presupuesto de NFR-01 (≤ 80 MB por
/// célula). Al alcanzar el límite se descarta la conversación insertada hace más tiempo (orden
/// FIFO de inserción), nunca la que acaba de recibir un evento.
const CAPACIDAD_MAXIMA_MARCAS_DE_ORIGEN: usize = 10_000;

/// Mapa acotado de la marca temporal de origen más reciente por conversación.
///
/// No es un LRU propiamente dicho -no distingue conversaciones activas de inactivas-, solo
/// impone el tope de [`CAPACIDAD_MAXIMA_MARCAS_DE_ORIGEN`] que un `HashMap` simple no tendría,
/// purgando por orden de inserción cuando se supera.
#[derive(Default)]
struct MarcasDeOrigen {
    valores: HashMap<IdConversacion, i64>,
    orden_de_insercion: VecDeque<IdConversacion>,
}

impl MarcasDeOrigen {
    /// Registra o actualiza la marca de origen de una conversación, purgando la entrada más
    /// antigua si insertar una conversación nueva supera la capacidad máxima.
    fn insertar(&mut self, conversacion: IdConversacion, marca_temporal_ms: i64) {
        if !self.valores.contains_key(&conversacion) {
            self.orden_de_insercion.push_back(conversacion.clone());
            if self.orden_de_insercion.len() > CAPACIDAD_MAXIMA_MARCAS_DE_ORIGEN
                && let Some(mas_antigua) = self.orden_de_insercion.pop_front()
            {
                self.valores.remove(&mas_antigua);
            }
        }
        self.valores.insert(conversacion, marca_temporal_ms);
    }

    /// Consulta la marca de origen de una conversación, si el adaptador ha visto pasar algún
    /// evento entrante de ella por su bucle de lectura.
    fn obtener(&self, conversacion: &IdConversacion) -> Option<i64> {
        self.valores.get(conversacion).copied()
    }

    #[cfg(test)]
    fn longitud(&self) -> usize {
        self.valores.len()
    }
}

/// Evento interno de emparejamiento para enrutar desde el bucle de lectura hacia el llamante.
#[derive(Debug)]
pub(crate) enum EventoDeEmparejamiento {
    Codigo(crate::mensajes::CodigoEmparejamiento),
    Acuse(crate::mensajes::AcuseEmparejamiento),
}

/// Adaptador `ChannelAdapter` + `CicloDeVidaSesion` sobre IPC con el sidecar whatsmeow.
///
/// Implementa la semántica del canal propio: ventana siempre abierta, sin plantilla requerida,
/// y los cuatro estados de sesión del protocolo.
pub struct AdaptadorWhatsmeow {
    /// Ruta del socket Unix del sidecar.
    ruta_socket: PathBuf,
    /// Identificador de la célula, para el saludo.
    id_celula: String,
    /// Emisor de eventos entrantes hacia el motor.
    remitente_eventos: mpsc::Sender<EventoEntrante>,
    /// Estado de sesión actual, difundido por watch.
    estado_sesion: watch::Sender<EstadoSesion>,
    /// Receptor del estado de sesión, para consultas.
    receptor_estado: watch::Receiver<EstadoSesion>,
    /// Retroceso de reconexión, protegido por mutex para uso desde la tarea de fondo.
    retroceso: Arc<Mutex<Retroceso>>,
    /// Extremo de escritura compartido con la conexión activa.
    escritor_compartido:
        Arc<tokio::sync::Mutex<Option<tokio::io::WriteHalf<tokio::net::UnixStream>>>>,
    /// Marcas de tiempo de origen por conversación para acuses, acotadas en tamaño.
    marcas_de_origen: Arc<Mutex<MarcasDeOrigen>>,
    /// Contador para generar IDs de mensaje únicos.
    contador_mensajes: Arc<AtomicUsize>,
    /// Acuses de respaldo del sqlstore pendientes de correlación por identificador de ronda.
    respaldo_pendiente: Arc<
        tokio::sync::Mutex<
            HashMap<String, tokio::sync::oneshot::Sender<crate::mensajes::AcuseRespaldoSqlstore>>,
        >,
    >,
    /// Canal de eventos de emparejamiento en curso, si lo hay.
    emparejamiento_pendiente: Arc<tokio::sync::Mutex<Option<mpsc::Sender<EventoDeEmparejamiento>>>>,
}

impl AdaptadorWhatsmeow {
    /// Crea el adaptador y el receptor de eventos que el `Motor` debe consumir.
    ///
    /// `capacidad` acota el canal `mpsc` de eventos: por debajo de ella las entregas se completan
    /// de inmediato, por encima aplican contrapresión, como cualquier adaptador real.
    ///
    /// `retroceso` inyecta la política de reconexión; los tests la sustituyen por una con
    /// tiempos mínimos para no dormir sobre el reloj de pared.
    pub fn nuevo(
        ruta_socket: impl Into<PathBuf>,
        id_celula: impl Into<String>,
        capacidad: usize,
        retroceso: Retroceso,
    ) -> (Self, mpsc::Receiver<EventoEntrante>) {
        let (remitente_eventos, receptor_eventos) = mpsc::channel(capacidad);
        let (estado_tx, estado_rx) = watch::channel(EstadoSesion::Reconectando);

        let adaptador = Self {
            ruta_socket: ruta_socket.into(),
            id_celula: id_celula.into(),
            remitente_eventos,
            estado_sesion: estado_tx,
            receptor_estado: estado_rx,
            retroceso: Arc::new(Mutex::new(retroceso)),
            escritor_compartido: Arc::new(tokio::sync::Mutex::new(None)),
            marcas_de_origen: Arc::new(Mutex::new(MarcasDeOrigen::default())),
            contador_mensajes: Arc::new(AtomicUsize::new(0)),
            respaldo_pendiente: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            emparejamiento_pendiente: Arc::new(tokio::sync::Mutex::new(None)),
        };

        (adaptador, receptor_eventos)
    }

    /// Arranca la tarea de fondo que conecta, saluda y lee mensajes del sidecar.
    ///
    /// Debe llamarse una sola vez después de construir el adaptador. La tarea se reconecta
    /// automáticamente con retroceso exponencial ante cualquier desconexión.
    pub fn arrancar(&self) {
        let ruta = self.ruta_socket.clone();
        let id_celula = self.id_celula.clone();
        let remitente = self.remitente_eventos.clone();
        let estado_tx = self.estado_sesion.clone();
        let retroceso = Arc::clone(&self.retroceso);
        let escritor = Arc::clone(&self.escritor_compartido);
        let marcas = Arc::clone(&self.marcas_de_origen);
        let respaldo_pendiente = Arc::clone(&self.respaldo_pendiente);
        let emparejamiento_pendiente = Arc::clone(&self.emparejamiento_pendiente);

        tokio::spawn(async move {
            bucle_de_conexion(
                ruta,
                id_celula,
                remitente,
                estado_tx,
                retroceso,
                escritor,
                marcas,
                respaldo_pendiente,
                emparejamiento_pendiente,
            )
            .await;
        });
    }

    /// Ruta del socket Unix configurada.
    pub fn ruta_socket(&self) -> &Path {
        &self.ruta_socket
    }

    /// Estado de sesión actual.
    pub fn estado_actual(&self) -> EstadoSesion {
        *self.receptor_estado.borrow()
    }

    /// Suscribe un receptor a las actualizaciones del estado de sesión del canal.
    pub fn suscribir_estado(&self) -> watch::Receiver<EstadoSesion> {
        self.receptor_estado.clone()
    }

    /// Ordena un emparejamiento al sidecar y procesa el flujo de códigos rotativos hasta el acuse terminal.
    ///
    /// Registra el canal de eventos antes de enviar la orden para evitar carreras. Cada código recibido
    /// invoca el `manejador` de forma síncrona. La espera completa está acotada por un único `plazo`
    /// que no se reinicia con la llegada de códigos nuevos.
    pub async fn ordenar_emparejamiento(
        &self,
        metodo: &str,
        plazo: Duration,
        mut manejador: impl FnMut(&crate::mensajes::CodigoEmparejamiento) + Send,
    ) -> Result<crate::mensajes::AcuseEmparejamiento, ErrorCanalWhatsmeow> {
        if self.escritor_compartido.lock().await.is_none() {
            return Err(ErrorCanalWhatsmeow::SinConexion);
        }

        let (tx, mut rx) = mpsc::channel(32);
        {
            let mut pendiente = self.emparejamiento_pendiente.lock().await;
            *pendiente = Some(tx);
        }

        let orden = crate::mensajes::OrdenEmparejar {
            version: crate::mensajes::VERSION_PROTOCOLO,
            tipo: "orden_emparejar".to_string(),
            metodo: metodo.to_string(),
        };

        if let Err(e) =
            crate::conexion::enviar_orden_emparejar(&self.escritor_compartido, &orden).await
        {
            let mut pendiente = self.emparejamiento_pendiente.lock().await;
            *pendiente = None;
            return Err(e);
        }

        let limite = tokio::time::Instant::now() + plazo;
        loop {
            match tokio::time::timeout_at(limite, rx.recv()).await {
                Ok(Some(EventoDeEmparejamiento::Codigo(codigo))) => {
                    manejador(&codigo);
                }
                Ok(Some(EventoDeEmparejamiento::Acuse(acuse))) => {
                    return Ok(acuse);
                }
                Ok(None) => {
                    let mut pendiente = self.emparejamiento_pendiente.lock().await;
                    *pendiente = None;
                    return Err(ErrorCanalWhatsmeow::EmparejamientoSinAcuse);
                }
                Err(_agotado) => {
                    let mut pendiente = self.emparejamiento_pendiente.lock().await;
                    *pendiente = None;
                    return Err(ErrorCanalWhatsmeow::EmparejamientoSinAcuse);
                }
            }
        }
    }

    /// Ordena un respaldo del sqlstore al sidecar y espera el acuse correspondiente.
    ///
    /// Registra el canal de respuesta por `identificador_de_ronda` antes de enviar la orden
    /// para evitar carreras, y espera hasta `plazo` antes de devolver [`ErrorCanalWhatsmeow::RespaldoSinAcuse`].
    pub async fn ordenar_respaldo_sqlstore(
        &self,
        destino: &str,
        identificador_de_ronda: &str,
        plazo: Duration,
    ) -> Result<crate::mensajes::AcuseRespaldoSqlstore, ErrorCanalWhatsmeow> {
        if self.escritor_compartido.lock().await.is_none() {
            return Err(ErrorCanalWhatsmeow::SinConexion);
        }

        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pendientes = self.respaldo_pendiente.lock().await;
            pendientes.insert(identificador_de_ronda.to_string(), tx);
        }

        let orden = crate::mensajes::OrdenRespaldoSqlstore {
            version: crate::mensajes::VERSION_PROTOCOLO,
            tipo: "orden_respaldo_sqlstore".to_string(),
            orden: "respaldar_sqlstore".to_string(),
            destino: destino.to_string(),
            identificador_de_ronda: identificador_de_ronda.to_string(),
        };

        if let Err(e) =
            crate::conexion::enviar_orden_respaldo_sqlstore(&self.escritor_compartido, &orden).await
        {
            let mut pendientes = self.respaldo_pendiente.lock().await;
            pendientes.remove(identificador_de_ronda);
            return Err(e);
        }

        match tokio::time::timeout(plazo, rx).await {
            Ok(Ok(acuse)) => Ok(acuse),
            Ok(Err(_oneshot_caido)) => {
                let mut pendientes = self.respaldo_pendiente.lock().await;
                pendientes.remove(identificador_de_ronda);
                Err(ErrorCanalWhatsmeow::RespaldoSinAcuse)
            }
            Err(_agotado) => {
                let mut pendientes = self.respaldo_pendiente.lock().await;
                pendientes.remove(identificador_de_ronda);
                Err(ErrorCanalWhatsmeow::RespaldoSinAcuse)
            }
        }
    }
}

/// Bucle de conexión con reconexión automática.
#[allow(clippy::too_many_arguments)]
async fn bucle_de_conexion(
    ruta: PathBuf,
    id_celula: String,
    remitente: mpsc::Sender<EventoEntrante>,
    estado_tx: watch::Sender<EstadoSesion>,
    retroceso: Arc<Mutex<Retroceso>>,
    escritor_compartido: Arc<
        tokio::sync::Mutex<Option<tokio::io::WriteHalf<tokio::net::UnixStream>>>,
    >,
    marcas_de_origen: Arc<Mutex<MarcasDeOrigen>>,
    respaldo_pendiente: Arc<
        tokio::sync::Mutex<
            HashMap<String, tokio::sync::oneshot::Sender<crate::mensajes::AcuseRespaldoSqlstore>>,
        >,
    >,
    emparejamiento_pendiente: Arc<tokio::sync::Mutex<Option<mpsc::Sender<EventoDeEmparejamiento>>>>,
) {
    loop {
        // Intentar conectar.
        match Conexion::conectar(&ruta, Arc::clone(&escritor_compartido)).await {
            Ok(mut conexion) => {
                // Ejecutar saludo.
                match conexion.saludar(&id_celula).await {
                    Ok(_saludo) => {
                        // Conexión establecida y saludo exitoso.
                        let _ = estado_tx.send(EstadoSesion::Activa);
                        {
                            let mut r = retroceso
                                .lock()
                                .expect("el mutex de retroceso no debería estar envenenado");
                            r.reiniciar();
                        }

                        // Leer mensajes hasta desconexión.
                        if let Err(_e) = leer_mensajes(
                            &mut conexion,
                            &remitente,
                            &estado_tx,
                            &marcas_de_origen,
                            &respaldo_pendiente,
                            &emparejamiento_pendiente,
                        )
                        .await
                        {
                            // La conexión se perdió; pasar a reconectando.
                            let _ = estado_tx.send(EstadoSesion::Reconectando);
                        }

                        // Limpiar escritor al desconectar
                        {
                            let mut lock = escritor_compartido.lock().await;
                            *lock = None;
                        }
                    }
                    Err(ErrorCanalWhatsmeow::DesajusteDeVersion { propia, remota }) => {
                        // Limpiar escritor
                        {
                            let mut lock = escritor_compartido.lock().await;
                            *lock = None;
                        }
                        // Desajuste de versión: registrar y reintentar. No se negocia.
                        eprintln!(
                            "hexcell-canal-whatsmeow: desajuste de versión IPC: \
                             propia={propia}, remota={remota}"
                        );
                        let _ = estado_tx.send(EstadoSesion::Reconectando);
                    }
                    Err(_e) => {
                        // Limpiar escritor
                        {
                            let mut lock = escritor_compartido.lock().await;
                            *lock = None;
                        }
                        // Error de saludo: reconectar.
                        let _ = estado_tx.send(EstadoSesion::Reconectando);
                    }
                }
            }
            Err(_e) => {
                // Limpiar escritor
                {
                    let mut lock = escritor_compartido.lock().await;
                    *lock = None;
                }
                // No se pudo conectar; ya estamos en Reconectando.
                let _ = estado_tx.send(EstadoSesion::Reconectando);
            }
        }

        // Esperar antes de reintentar.
        let espera = {
            let mut r = retroceso
                .lock()
                .expect("el mutex de retroceso no debería estar envenenado");
            r.siguiente()
        };
        tokio::time::sleep(espera).await;
    }
}

/// Lee mensajes de una conexión activa y los despacha.
async fn leer_mensajes(
    conexion: &mut Conexion,
    remitente: &mpsc::Sender<EventoEntrante>,
    estado_tx: &watch::Sender<EstadoSesion>,
    marcas_de_origen: &Arc<Mutex<MarcasDeOrigen>>,
    respaldo_pendiente: &Arc<
        tokio::sync::Mutex<
            HashMap<String, tokio::sync::oneshot::Sender<crate::mensajes::AcuseRespaldoSqlstore>>,
        >,
    >,
    emparejamiento_pendiente: &Arc<
        tokio::sync::Mutex<Option<mpsc::Sender<EventoDeEmparejamiento>>>,
    >,
) -> Result<(), ErrorCanalWhatsmeow> {
    loop {
        let mensaje = conexion.leer_mensaje().await?;

        match mensaje {
            MensajeEntrante::EventoEntrante(evento_ipc) => {
                // Registrar la marca temporal de origen para uso en envíos
                {
                    let mut marcas = marcas_de_origen
                        .lock()
                        .expect("el mutex no debería estar envenenado");
                    marcas.insertar(
                        IdConversacion::nuevo(evento_ipc.id_conversacion.clone()),
                        evento_ipc.marca_temporal_ms,
                    );
                }

                // Convertir marca_temporal_ms (milisegundos Unix absolutos) a SystemTime.
                let marca_temporal = if evento_ipc.marca_temporal_ms > 0 {
                    UNIX_EPOCH + Duration::from_millis(evento_ipc.marca_temporal_ms as u64)
                } else {
                    UNIX_EPOCH
                };

                let evento = EventoEntrante {
                    remitente: IdRemitente::nuevo(evento_ipc.id_remitente),
                    conversacion: IdConversacion::nuevo(evento_ipc.id_conversacion),
                    contenido: evento_ipc.contenido,
                    marca_temporal,
                    deduplicacion: IdDeduplicacion::nuevo(evento_ipc.id_deduplicacion.clone()),
                };

                // Entregar al motor; si el canal está lleno, aplica contrapresión.
                if remitente.send(evento).await.is_err() {
                    // El motor cerró el receptor; salir del bucle.
                    return Ok(());
                }

                // Confirmar el evento con su id_deduplicacion, nunca un número de secuencia.
                //
                // BRECHA CONOCIDA (decisión humana del 2026-08-08, ver `adr-0011`): la sección 4
                // del protocolo exige confirmar solo cuando el evento queda registrado de forma
                // durable del lado del núcleo. Aquí se confirma tras la entrega al `mpsc` en
                // memoria, no tras un registro durable, porque el núcleo todavía no tiene consumo
                // durable propio de este evento en esta etapa. Un caído del proceso entre este
                // punto y el registro real degrada la entrega de «al menos una vez» a «como mucho
                // una vez». La brecha se cierra con la tarea que construya el consumo durable del
                // lado Rust; queda registrada en `adr-0011` y en `docs/STATUS.md`, no oculta.
                conexion.confirmar(&evento_ipc.id_deduplicacion).await?;
            }
            MensajeEntrante::EstadoSesion(estado_ipc) => {
                // Mapear el estado del cable a EstadoSesion del dominio.
                // causa, codigo y expira_en_ms se quedan DENTRO de este crate: son taxonomía
                // de whatsmeow y no pertenecen al puerto.
                let estado = match estado_ipc.estado.as_str() {
                    "activa" => EstadoSesion::Activa,
                    "reconectando" => EstadoSesion::Reconectando,
                    "desvinculada" => EstadoSesion::Desvinculada,
                    "pausada" => EstadoSesion::Pausada,
                    otro => {
                        return Err(ErrorCanalWhatsmeow::ErrorDeProtocolo(format!(
                            "estado_sesion con valor desconocido: '{otro}'"
                        )));
                    }
                };
                let _ = estado_tx.send(estado);
            }
            MensajeEntrante::CodigoEmparejamiento(codigo) => {
                let remitente = {
                    let lock = emparejamiento_pendiente.lock().await;
                    lock.clone()
                };
                if let Some(tx) = remitente {
                    let _ = tx.send(EventoDeEmparejamiento::Codigo(codigo)).await;
                } else {
                    eprintln!("hexcell-canal-whatsmeow: codigo_emparejamiento huérfano recibido");
                }
            }
            MensajeEntrante::AcuseEmparejamiento(acuse) => match acuse.resultado.as_str() {
                "completado" | "expirado" | "fallido" => {
                    let remitente = {
                        let mut lock = emparejamiento_pendiente.lock().await;
                        lock.take()
                    };
                    if let Some(tx) = remitente {
                        let _ = tx.send(EventoDeEmparejamiento::Acuse(acuse)).await;
                    } else {
                        eprintln!(
                            "hexcell-canal-whatsmeow: acuse_emparejamiento huérfano recibido"
                        );
                    }
                }
                otro => {
                    eprintln!(
                        "hexcell-canal-whatsmeow: acuse_emparejamiento con resultado desconocido descartado: '{otro}'"
                    );
                }
            },
            MensajeEntrante::AcuseRespaldoSqlstore(acuse) => {
                let remitente = {
                    let mut pendientes = respaldo_pendiente.lock().await;
                    pendientes.remove(&acuse.identificador_de_ronda)
                };
                if let Some(tx) = remitente {
                    let _ = tx.send(acuse);
                } else {
                    eprintln!(
                        "hexcell-canal-whatsmeow: acuse_respaldo_sqlstore huérfano recibido para ronda: {}",
                        acuse.identificador_de_ronda
                    );
                }
            }
            MensajeEntrante::AcuseEnvio(_) => {
                // Los acuses de envío se consumen sin elevar la taxonomía de whatsmeow al puerto.
            }
            MensajeEntrante::Saludo(_) => {
                // Un segundo saludo después del inicial es un error de protocolo.
                return Err(ErrorCanalWhatsmeow::ErrorDeProtocolo(
                    "saludo inesperado tras el saludo inicial".to_string(),
                ));
            }
        }
    }
}

impl ChannelAdapter for AdaptadorWhatsmeow {
    type Error = ErrorCanalWhatsmeow;

    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        let texto = match mensaje {
            MensajeSaliente::RespuestaLibre { texto, .. } => texto,
            MensajeSaliente::Plantilla { .. } => {
                return Err(ErrorCanalWhatsmeow::PlantillaNoRepresentable);
            }
        };

        // Comprobar la conexión antes que la marca de origen: sin conexión activa el envío ya
        // es imposible, sin importar si la marca se conoce o no, así que ese es el error que
        // debe salir primero. `enviar_saliente` vuelve a comprobarlo más abajo (la conexión
        // puede caerse entre este punto y ese), así que esto no reemplaza esa comprobación,
        // solo prioriza cuál de los dos motivos de rechazo se reporta cuando ambos aplican.
        if self.escritor_compartido.lock().await.is_none() {
            return Err(ErrorCanalWhatsmeow::SinConexion);
        }

        // Nunca se rellena con un centinela: un 0 (época Unix) se leería del lado del sidecar
        // como "ya expirado" y descartaría el mensaje con cero intentos reales de envío, sin
        // que nada lo distinga de una expiración legítima. Si el adaptador no ha visto pasar
        // ningún evento entrante de esta conversación por su bucle de lectura -lo más común
        // justo tras un reinicio del núcleo, porque el mapa es memoria de proceso-, se rechaza
        // explícitamente en vez de enviar con una marca inventada.
        let marca_temporal_origen_ms = {
            let marcas = self
                .marcas_de_origen
                .lock()
                .expect("el mutex no debería estar envenenado");
            marcas
                .obtener(conversacion)
                .ok_or(ErrorCanalWhatsmeow::OrigenDesconocido)?
        };

        let id_mensaje = format!(
            "{}-{}",
            conversacion.como_str(),
            self.contador_mensajes.fetch_add(1, Ordering::Relaxed)
        );

        let msj_ipc = crate::mensajes::MensajeSalienteIpc {
            version: crate::mensajes::VERSION_PROTOCOLO,
            tipo: "mensaje_saliente".to_string(),
            id_mensaje,
            id_conversacion: conversacion.como_str().to_string(),
            contenido: texto,
            marca_temporal_origen_ms,
        };

        crate::conexion::enviar_saliente(&self.escritor_compartido, &msj_ipc).await?;

        Ok(ResultadoEnvio::Aceptado)
    }

    /// **Siempre** responde `Abierta`: este transporte no impone ninguna ventana de 24 horas.
    ///
    /// Fabricar una restricción que el transporte no tiene sería degradar el producto para
    /// parecerse a un canal que la célula no usa (`adr-0010`, distinción TIPO/POLÍTICA;
    /// `hexcell_core::canal`, líneas de documentación del módulo).
    async fn estado_ventana(
        &self,
        _conversacion: &IdConversacion,
    ) -> Result<EstadoVentanaServicio, Self::Error> {
        // La ventana del canal propio nunca se cierra. Se elige un punto de expiración lejano
        // para cumplir con la semántica del enum sin inventar una restricción.
        Ok(EstadoVentanaServicio::Abierta {
            expira_en: SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60),
        })
    }
}

impl hexcell_core::canal::CicloDeVidaSesion for AdaptadorWhatsmeow {
    type Error = ErrorCanalWhatsmeow;

    /// Inicia el emparejamiento enviando una orden al sidecar.
    ///
    /// La implementación completa llega con la integración del emparejamiento; por ahora se
    /// devuelve un error de «sin conexión» si no hay conexión activa.
    async fn iniciar_emparejamiento(&self) -> Result<Emparejamiento, Self::Error> {
        // TODO(A-3): implementar cuando el cable de emparejamiento esté completo.
        Err(ErrorCanalWhatsmeow::SinConexion)
    }

    /// Cierra la sesión y desvincula el dispositivo.
    ///
    /// La implementación completa requiere el cable de salida (tarea 12).
    async fn cerrar_sesion(&self) -> Result<(), Self::Error> {
        // TODO(A-3): implementar cuando el cable de salida esté completo.
        Err(ErrorCanalWhatsmeow::SinConexion)
    }

    /// Consulta el estado actual de la sesión del canal.
    fn estado_sesion(&self) -> EstadoSesion {
        *self.receptor_estado.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marcas_de_origen_purga_la_mas_antigua_al_superar_la_capacidad() {
        let mut marcas = MarcasDeOrigen::default();
        for i in 0..(CAPACIDAD_MAXIMA_MARCAS_DE_ORIGEN + 5) {
            marcas.insertar(IdConversacion::nuevo(format!("conv-{i}")), i as i64);
        }

        assert_eq!(marcas.longitud(), CAPACIDAD_MAXIMA_MARCAS_DE_ORIGEN);

        // Las 5 primeras insertadas ya no están: se purgaron en orden de inserción (FIFO), no
        // al azar, y el mapa nunca superó el tope.
        for i in 0..5 {
            assert!(
                marcas
                    .obtener(&IdConversacion::nuevo(format!("conv-{i}")))
                    .is_none(),
                "la conversación conv-{i} debía haberse purgado por ser la más antigua"
            );
        }

        // La más recientemente insertada sigue presente con su marca correcta.
        let ultima = CAPACIDAD_MAXIMA_MARCAS_DE_ORIGEN + 4;
        assert_eq!(
            marcas.obtener(&IdConversacion::nuevo(format!("conv-{ultima}"))),
            Some(ultima as i64)
        );
    }

    #[test]
    fn marcas_de_origen_actualizar_una_existente_no_cuenta_como_insercion_nueva() {
        let mut marcas = MarcasDeOrigen::default();
        marcas.insertar(IdConversacion::nuevo("conv-1"), 100);
        marcas.insertar(IdConversacion::nuevo("conv-1"), 200);

        assert_eq!(marcas.longitud(), 1);
        assert_eq!(marcas.obtener(&IdConversacion::nuevo("conv-1")), Some(200));
    }
}
