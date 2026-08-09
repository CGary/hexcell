//! Servicio de aplicación `AdaptadorWhatsmeow`: cliente IPC que implementa `ChannelAdapter` y
//! `CicloDeVidaSesion`.
//!
//! El adaptador conecta al socket Unix del sidecar (que escucha), ejecuta el saludo de versión,
//! y a partir de ahí lee mensajes entrantes en una tarea de fondo. Los eventos entrantes se
//! entregan al núcleo a través de un `tokio::sync::mpsc` acotado, siguiendo la convención de
//! `adr-0016`. El estado de sesión se difunde por un `tokio::sync::watch`.
//!
//! # Puente provisional de envío (decisión humana del 2026-08-08)
//!
//! Hasta que la tarea 12 de la etapa A-3 proporcione el cable de salida, `send` acepta en un
//! búfer en memoria y devuelve `Aceptado`. Esto es semánticamente defendible porque `Aceptado`
//! ya significa «aceptado para entrega posterior». El puente se registra en `adr-0011` y se
//! sustituye por la tarea 12 antes de cualquier uso real (tarea 15).
//!
//! # Política de ventana de servicio
//!
//! `estado_ventana` **siempre** responde `Abierta`: este transporte no impone ninguna ventana
//! de 24 horas y fabricar una sería degradar el producto para parecerse a un canal que la célula
//! no usa (`adr-0010`, distinción TIPO/POLÍTICA).

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
    /// Puente provisional de envío: cuenta de mensajes aceptados en el búfer.
    /// Se sustituye por el cable de salida real en la tarea 12 de A-3.
    envios_aceptados: Arc<AtomicUsize>,
    /// Búfer provisional de mensajes salientes.
    bufer_saliente: Arc<Mutex<Vec<(IdConversacion, MensajeSaliente)>>>,
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
            envios_aceptados: Arc::new(AtomicUsize::new(0)),
            bufer_saliente: Arc::new(Mutex::new(Vec::new())),
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

        tokio::spawn(async move {
            bucle_de_conexion(ruta, id_celula, remitente, estado_tx, retroceso).await;
        });
    }

    /// Número de mensajes aceptados en el búfer provisional de envío.
    ///
    /// Expuesto para los tests: confirma que `send` efectivamente almacenó el mensaje.
    pub fn envios_aceptados(&self) -> usize {
        self.envios_aceptados.load(Ordering::Relaxed)
    }

    /// Ruta del socket Unix configurada.
    pub fn ruta_socket(&self) -> &Path {
        &self.ruta_socket
    }

    /// Estado de sesión actual.
    pub fn estado_actual(&self) -> EstadoSesion {
        *self.receptor_estado.borrow()
    }
}

/// Bucle de conexión con reconexión automática.
async fn bucle_de_conexion(
    ruta: PathBuf,
    id_celula: String,
    remitente: mpsc::Sender<EventoEntrante>,
    estado_tx: watch::Sender<EstadoSesion>,
    retroceso: Arc<Mutex<Retroceso>>,
) {
    loop {
        // Intentar conectar.
        match Conexion::conectar(&ruta).await {
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
                        if let Err(_e) = leer_mensajes(&mut conexion, &remitente, &estado_tx).await
                        {
                            // La conexión se perdió; pasar a reconectando.
                            let _ = estado_tx.send(EstadoSesion::Reconectando);
                        }
                    }
                    Err(ErrorCanalWhatsmeow::DesajusteDeVersion { propia, remota }) => {
                        // Desajuste de versión: registrar y reintentar. No se negocia.
                        eprintln!(
                            "hexcell-canal-whatsmeow: desajuste de versión IPC: \
                             propia={propia}, remota={remota}"
                        );
                        let _ = estado_tx.send(EstadoSesion::Reconectando);
                    }
                    Err(_e) => {
                        // Error de saludo: reconectar.
                        let _ = estado_tx.send(EstadoSesion::Reconectando);
                    }
                }
            }
            Err(_e) => {
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
) -> Result<(), ErrorCanalWhatsmeow> {
    loop {
        let mensaje = conexion.leer_mensaje().await?;

        match mensaje {
            MensajeEntrante::EventoEntrante(evento_ipc) => {
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
            MensajeEntrante::CodigoEmparejamiento(_) => {
                // Los códigos de emparejamiento se procesan por CicloDeVidaSesion; por ahora
                // se registran y se descartan. La implementación completa llega con la tarea
                // de emparejamiento.
            }
            MensajeEntrante::AcuseEmparejamiento(_) => {
                // Igual que el código de emparejamiento: se consume cuando CicloDeVidaSesion
                // esté completo.
            }
            MensajeEntrante::AcuseRespaldoSqlstore(_) => {
                // El acuse del respaldo se consume por el módulo de respaldo (tarea separada).
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

    /// Acepta el mensaje en un búfer en memoria y devuelve `Aceptado`.
    ///
    /// **Puente provisional**: hasta que la tarea 12 de A-3 proporcione el cable de salida,
    /// `send` almacena el mensaje localmente. `Aceptado` ya significa «aceptado para entrega
    /// posterior» — la semántica no cambia, solo falta el transporte. El puente se sustituye
    /// antes de cualquier uso real (tarea 15).
    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        // Registrar en el búfer provisional.
        {
            let mut bufer = self
                .bufer_saliente
                .lock()
                .expect("el mutex del búfer saliente no debería estar envenenado");
            bufer.push((conversacion.clone(), mensaje));
        }
        self.envios_aceptados.fetch_add(1, Ordering::Relaxed);

        // PUENTE PROVISIONAL (HEX-015, 2026-08-08): el cable de salida IPC no existe todavía.
        // La tarea 12 de A-3 lo proporciona; hasta entonces, `Aceptado` sin transporte.
        // Registrado en docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md.
        eprintln!(
            "hexcell-canal-whatsmeow: envío aceptado en búfer provisional \
             (cable de salida pendiente de A-3 tarea 12)"
        );

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
