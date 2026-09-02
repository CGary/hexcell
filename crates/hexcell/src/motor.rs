//! Motor de mensajería: consume eventos, despacha al procesador y envía por el puerto de canal.
//!
//! El motor no conoce ningún transporte concreto: es genérico sobre cualquier implementación de
//! `ChannelAdapter` (`hexcell_core::canal`) y sobre cualquier `ProcesadorDeMensajes`
//! (`crate::procesador`). Recibe ambos por inyección en su constructor, nunca fija el tipo de un
//! adaptador concreto.
//!
//! # Convención de entrega de eventos
//!
//! El puerto `ChannelAdapter` declara solo `send` y `estado_ventana`; el mecanismo de entrega de
//! `EventoEntrante` no es uno de los siete elementos de FR-12 y se decide en esta etapa
//! (`docs/adr/adr-0016-convencion-de-entrega-de-eventos.md`). La convención, documentada aquí sin
//! nombrar ningún transporte concreto, es: todo adaptador entrega sus eventos por un canal
//! `tokio::sync::mpsc` acotado que él mismo crea y posee, y cuyo extremo receptor pasa a este
//! motor en el momento de construirse.
//!
//! # Admisión, concurrencia y bucle secuencial
//!
//! Antes de cualquier otra política, cada evento atraviesa dos compuertas en este orden fijo:
//! **admisión GCRA** primero (HEX-037) y **semáforo de concurrencia** después (FR-09), ambas
//! antes de la deduplicación. Un evento descartado por cualquiera de las dos no toca la base ni
//! genera trabajo posterior: solo deja su registro (`admision_descartada` o
//! `concurrencia_descartada`) y retorna.
//!
//! Salvedad deliberada (decisión del 23 de agosto de 2026): el bucle de
//! `Motor::ejecutar` procesa los eventos **secuencialmente** (runtime `current_thread`, sin
//! `tokio::spawn` por evento), por las invariantes de orden documentadas en este módulo
//! (deduplicación, drenaje cronológico de diferidas, apagado no cancelable). Hoy, por tanto, el
//! semáforo actúa como compuerta estructural: el límite queda aplicado en el único punto por el
//! que pasará todo despacho futuro, y acotará tareas en vuelo reales el día que se introduzca
//! despacho concurrente, que es una tarea distinta y mayor.
//!
//! # Orden de las tres políticas nuevas por evento
//!
//! El orden es la propia política, no un detalle de implementación:
//!
//! 1. **Deduplicación primero.** Se consulta el registro con el identificador de deduplicación y
//!    la marca temporal del evento; un veredicto de duplicado hace `continue` sin despachar al
//!    procesador y sin enviar nada (AC-7).
//! 2. **Drenaje de diferidas, antes de la respuesta del propio evento.** Que llegue un evento
//!    nuevo para una conversación es, precisamente, que el cliente ha vuelto a escribir, y eso es
//!    lo que reabre la ventana de servicio en el adaptador simulado. Las respuestas que quedaron
//!    diferidas para esa conversación se reintentan **antes** de la respuesta del evento que
//!    acaba de llegar, para que el hilo se mantenga cronológico.
//! 3. **Registro, despacho y envío**, como hacía el motor antes de esta tarea, salvo que el brazo
//!    `FueraDeVentana` ya no se limita a registrar un mensaje: aplica la política (encolar la
//!    respuesta como diferida) en vez de tratar el rechazo como los demás.
//!
//! # Dos políticas ante un fallo de persistencia
//!
//! Desde HEX-006 el registro de deduplicación y el historial viven en `sessions.db`, así que las
//! dos operaciones pueden fallar. Ninguna de las dos mata la célula, y cada una falla en la
//! dirección que menos daño hace al negocio del cliente:
//!
//! * **Deduplicación: `fail-open`.** Si la base no responde, el evento se procesa **como nuevo**.
//!   El residuo es el mismo que el plan ya aceptó para una reentrega tardía —duplicar el trabajo
//!   conversacional— y es estrictamente mejor que enmudecer ante un cliente que está escribiendo.
//! * **Historial: se reporta y se sigue.** Que no se pueda anotar lo ocurrido no es razón para no
//!   contestar: la respuesta sale igualmente y el fallo se registra estructuradamente.
//!
//! Las dos quedan escritas aquí a propósito. Un `fail-open` sin justificación al lado se lee, seis
//! meses después, como un caso de error que alguien olvidó tratar.
//!
//! # Política ante `FueraDeVentana`: diferir, no escalar
//!
//! Se eligió **diferir** (encolar la respuesta hasta que el cliente vuelva a escribir) en vez de
//! **escalar a un humano**. La escalada se descartó por falta de dónde aterrizar, no por
//! preferencia: hasta esta misma tarea no existía ningún registro estructurado ni ninguna vía de
//! notificación a un operador, y el plano de CLI de administración llega en la etapa A-6; una rama
//! de escalada seguiría sin tener adónde ir. Diferir, en cambio, es implementable, observable y
//! probable ahora mismo.
//!
//! La cola de diferidas es **acotada por conversación**
//! (`crate::conversaciones::EstadoDeConversaciones`) con una regla de descarte del más antiguo en
//! el tope: una cola sin límite de respuestas no entregables es exactamente la fuga lenta que el
//! presupuesto de ≤ 80 MB por célula de NFR-01 no puede absorber. No hay bucle de reintento, ni
//! temporizador de `backoff`, ni tarea de fondo: las diferidas se reintentan únicamente cuando
//! llega un evento **posterior** para esa misma conversación, y una respuesta rechazada de nuevo
//! al drenar vuelve a encolarse, sujeta al mismo tope. Un temporizador necesitaría una fuente de
//! tiempo dentro del motor, exactamente el acoplamiento que este módulo evita a propósito.
//!
//! # Apagado ordenado (HEX-007)
//!
//! `ejecutar` recibe una [`SenalDeApagado`](crate::apagado::SenalDeApagado) y corre un
//! `tokio::select!` con `biased` sobre exactamente dos ramas: la señal y `receptor_eventos.recv()`.
//! El trabajo de cada evento se espera **dentro** del cuerpo de esa segunda rama, nunca como una
//! rama más del propio `select!`, así que el `select!` nunca puede estar sondeando mientras un
//! evento está a medias: no hay forma de cancelarlo. Al recibir la señal, el motor cierra
//! `receptor_eventos` (`close()`): a partir de ese instante ningún emisor puede encolar nada más,
//! pero `recv()` sigue entregando lo que ya estuviera en la cola hasta vaciarla. El drenaje que
//! sigue comprueba el límite temporal **entre** eventos, nunca envolviendo el drenaje entero en un
//! temporizador de expiración global: eso cortaría el futuro en curso en cualquier punto en que
//! estuviera, posiblemente entre el envío y la anotación en el historial — precisamente el corte a
//! medias que esta tarea existe para impedir.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use hexcell_core::admision::{ConfiguracionGcra, RegistroDeAdmision, ResultadoDeAdmision};
use hexcell_core::canal::{ChannelAdapter, EventoEntrante, MensajeSaliente, ResultadoEnvio};
use hexcell_core::identidad::IdConversacion;
use hexcell_storage::{ErrorDeAlmacen, RepositorioDeSesiones};
use tokio::sync::mpsc;

use crate::apagado::SenalDeApagado;
use crate::concurrencia::{
    LIMITE_DE_CONCURRENCIA_POR_DEFECTO, LimitadorDeConcurrencia, MotivoDescarteConcurrencia,
};
use crate::conversaciones::{EstadoDeConversaciones, EventoDeHistorial};
use crate::deduplicacion::{RegistroDeDeduplicacion, VeredictoDeDeduplicacion};
use crate::metricas::RegistroDeMetricas;
use crate::procesador::ProcesadorDeMensajes;
use crate::registro::{EntradaDeRegistro, NivelDeRegistro, emitir};

/// Motor de mensajería de una célula: bucle asíncrono sobre un adaptador y un procesador.
pub struct Motor<A, P>
where
    A: ChannelAdapter,
    P: ProcesadorDeMensajes,
{
    adaptador: A,
    procesador: P,
    receptor_eventos: mpsc::Receiver<EventoEntrante>,
    admision: RegistroDeAdmision,
    concurrencia: LimitadorDeConcurrencia,
    deduplicacion: RegistroDeDeduplicacion,
    conversaciones: EstadoDeConversaciones,
    metricas: Arc<RegistroDeMetricas>,
}

impl<A, P> Motor<A, P>
where
    A: ChannelAdapter,
    P: ProcesadorDeMensajes,
{
    /// Construye el motor a partir del adaptador, el procesador, el receptor de eventos que el
    /// propio adaptador entregó al crearse (siguiendo la convención de entrega descrita arriba),
    /// la ventana de retención con la que arranca el registro de deduplicación
    /// (`Configuracion::ventana_deduplicacion` en producción) y el repositorio de `sessions.db`
    /// que respalda tanto ese registro como el historial.
    pub fn nuevo(
        adaptador: A,
        procesador: P,
        receptor_eventos: mpsc::Receiver<EventoEntrante>,
        ventana_deduplicacion: Duration,
        repositorio: Arc<RepositorioDeSesiones>,
    ) -> Self {
        Self {
            adaptador,
            procesador,
            receptor_eventos,
            admision: RegistroDeAdmision::nuevo(ConfiguracionGcra::default()),
            concurrencia: LimitadorDeConcurrencia::nuevo(LIMITE_DE_CONCURRENCIA_POR_DEFECTO),
            deduplicacion: RegistroDeDeduplicacion::nuevo(
                Arc::clone(&repositorio),
                ventana_deduplicacion,
            ),
            conversaciones: EstadoDeConversaciones::nuevo(repositorio),
            metricas: Arc::new(RegistroDeMetricas::nuevo()),
        }
    }

    /// Reemplaza el registro de admisión GCRA del motor con la configuración dada.
    pub fn con_configuracion_gcra(mut self, configuracion: ConfiguracionGcra) -> Self {
        self.admision = RegistroDeAdmision::nuevo(configuracion);
        self
    }

    /// Reemplaza el limitador de concurrencia del motor con la instancia dada.
    pub fn con_limite_de_concurrencia(mut self, limitador: LimitadorDeConcurrencia) -> Self {
        self.concurrencia = limitador;
        self
    }

    /// Reemplaza el registro de métricas del motor con la instancia dada.
    pub fn con_metricas(mut self, metricas: Arc<RegistroDeMetricas>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Historial persistido de una conversación, para que los tests observen su continuidad.
    pub fn historial(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<Vec<EventoDeHistorial>, ErrorDeAlmacen> {
        self.conversaciones.historial(conversacion)
    }

    /// Ejecuta el bucle de consumo hasta que llega la señal de apagado o el canal de eventos se
    /// cierra por su cuenta.
    ///
    /// Ver la sección «Apagado ordenado» en la documentación del módulo para el porqué exacto de
    /// la forma de este bucle.
    pub async fn ejecutar(&mut self, mut senal: SenalDeApagado) {
        loop {
            tokio::select! {
                biased;
                () = senal.recibida() => {
                    emitir(EntradaDeRegistro::nueva(NivelDeRegistro::Info, "apagado_solicitado"));
                    self.receptor_eventos.close();
                    break;
                }
                evento = self.receptor_eventos.recv() => {
                    match evento {
                        Some(evento) => self.procesar_evento(evento).await,
                        None => return,
                    }
                }
            }
        }

        self.drenar_con_limite(senal.limite_de_drenaje()).await;
    }

    /// Tras la señal de apagado, drena lo que ya estuviera en la cola, comprobando el límite
    /// temporal **antes** de aceptar el siguiente evento, nunca alrededor de uno en curso.
    async fn drenar_con_limite(&mut self, limite: Duration) {
        let inicio_del_drenaje = Instant::now();
        loop {
            if inicio_del_drenaje.elapsed() >= limite {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "drenaje_incompleto")
                        .con_detalle(format!(
                            "límite de drenaje agotado con {} eventos pendientes",
                            self.receptor_eventos.len()
                        )),
                );
                return;
            }

            match self.receptor_eventos.recv().await {
                Some(evento) => self.procesar_evento(evento).await,
                None => {
                    emitir(EntradaDeRegistro::nueva(
                        NivelDeRegistro::Info,
                        "drenaje_completado",
                    ));
                    return;
                }
            }
        }
    }

    /// Procesa un único evento: control de admisión GCRA (FR-08), deduplicación, drenaje de
    /// diferidas, registro, despacho al procesador y envío. Es el cuerpo que tanto el bucle
    /// principal como el drenaje comparten.
    async fn procesar_evento(&mut self, evento: EventoEntrante) {
        let inicio = Instant::now();

        // Control de admisión GCRA (FR-08): evaluado inmediatamente al consumir el evento
        // del canal normalizado, estrictamente antes de la deduplicación, la carga de contexto
        // conversacional y la inferencia.
        if let ResultadoDeAdmision::Descartado { clave, motivo } =
            self.admision.admitir(evento.conversacion.como_str())
        {
            self.metricas.anotar_descarte_por_admision();
            // FR-08: Visibilidad de descartes por control de admisión. Métricas (A-4 t11) y alertas (A-6) diferidas.
            emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "admision_descartada")
                    .con_id_evento(evento.deduplicacion.como_str().to_string())
                    .con_id_conversacion(clave)
                    .con_latencia_ms(latencia_ms(inicio))
                    .con_detalle(motivo.to_string()),
            );
            return;
        }

        self.metricas.anotar_evento_admitido();

        // Límite de concurrencia por contenedor (FR-09): evaluado inmediatamente después del
        // control de admisión GCRA y estrictamente antes de la deduplicación.
        let _permiso_concurrencia = match self.concurrencia.intentar_adquirir() {
            Some(permiso) => permiso,
            None => {
                self.metricas.anotar_descarte_por_concurrencia();
                let motivo = MotivoDescarteConcurrencia::Saturacion;
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "concurrencia_descartada")
                        .con_id_evento(evento.deduplicacion.como_str().to_string())
                        .con_id_conversacion(evento.conversacion.como_str().to_string())
                        .con_latencia_ms(latencia_ms(inicio))
                        .con_detalle(motivo.to_string()),
                );
                return;
            }
        };

        let id_evento = evento.deduplicacion.como_str().to_string();
        let id_conversacion = evento.conversacion.como_str().to_string();

        emitir(
            EntradaDeRegistro::nueva(NivelDeRegistro::Info, "evento_recibido")
                .con_id_evento(id_evento.clone())
                .con_id_conversacion(id_conversacion.clone())
                .con_latencia_ms(latencia_ms(inicio)),
        );

        let veredicto = match self
            .deduplicacion
            .procesar(evento.deduplicacion.clone(), evento.marca_temporal)
        {
            Ok(veredicto) => veredicto,
            Err(error) => {
                // `fail-open`: ver la sección «Dos políticas ante un fallo de persistencia» en la
                // documentación de este módulo.
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                        .con_id_evento(id_evento.clone())
                        .con_id_conversacion(id_conversacion.clone())
                        .con_latencia_ms(latencia_ms(inicio))
                        .con_detalle(format!(
                            "fallo al consultar la deduplicación persistida: {error}"
                        )),
                );
                VeredictoDeDeduplicacion::Nuevo
            }
        };
        if veredicto == VeredictoDeDeduplicacion::Duplicado {
            emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Info, "evento_duplicado")
                    .con_id_evento(id_evento)
                    .con_id_conversacion(id_conversacion)
                    .con_latencia_ms(latencia_ms(inicio)),
            );
            return;
        }

        self.drenar_diferidas(&evento.conversacion, evento.marca_temporal, inicio)
            .await;

        if let Err(error) = self.conversaciones.registrar_entrante(
            &evento.conversacion,
            &evento.remitente,
            &evento.contenido,
            evento.marca_temporal,
        ) {
            emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                    .con_id_evento(id_evento.clone())
                    .con_id_conversacion(id_conversacion.clone())
                    .con_latencia_ms(latencia_ms(inicio))
                    .con_detalle(format!(
                        "no se pudo anotar el evento entrante en el historial: {error}"
                    )),
            );
        }

        emitir(
            EntradaDeRegistro::nueva(NivelDeRegistro::Info, "inferencia_iniciada")
                .con_id_evento(id_evento.clone())
                .con_id_conversacion(id_conversacion.clone())
                .con_latencia_ms(latencia_ms(inicio)),
        );

        let Some(mensaje) = self.procesador.procesar(&evento).await else {
            // El procesador devuelve `None` tanto si decide no responder como si el proveedor
            // de inferencia falló (RISK-12: qué contesta la célula ante ese fallo es una decisión
            // de producto diferida a la etapa A-4, y este procesador no la resuelve). El motor no
            // distingue esos dos casos porque el procesador no se lo dice, pero sí deja constancia
            // de que el evento terminó sin enviar nada, igual que hace con cada otro desenlace.
            emitir(
                EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "inferencia_sin_respuesta")
                    .con_id_evento(id_evento)
                    .con_id_conversacion(id_conversacion)
                    .con_latencia_ms(latencia_ms(inicio)),
            );
            return;
        };

        self.enviar_y_registrar(&evento.conversacion, mensaje, evento.marca_temporal, inicio)
            .await;
    }

    /// Reintenta, en orden de llegada, cada respuesta que quedó diferida para esta conversación.
    async fn drenar_diferidas(
        &mut self,
        conversacion: &IdConversacion,
        marca_temporal: SystemTime,
        inicio: Instant,
    ) {
        for mensaje in self.conversaciones.drenar_diferidas(conversacion) {
            self.enviar_y_registrar(conversacion, mensaje, marca_temporal, inicio)
                .await;
        }
    }

    /// Envía un mensaje y aplica la política que corresponda a cada desenlace del puerto.
    ///
    /// La marca temporal con la que se anota la salida es la del evento entrante que la provocó,
    /// no una lectura de la hora del sistema: el motor no tiene ninguna fuente de tiempo propia
    /// para lo que persiste, y todo lo que persiste está medido en el tiempo del canal. `inicio`
    /// es la única lectura de reloj monótono del motor, y mide exclusivamente la latencia de
    /// procesamiento para el registro estructurado.
    async fn enviar_y_registrar(
        &mut self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
        marca_temporal: SystemTime,
        inicio: Instant,
    ) {
        let id_conversacion = conversacion.como_str().to_string();
        let resultado = self.adaptador.send(conversacion, mensaje.clone()).await;

        match resultado {
            Ok(ResultadoEnvio::Aceptado) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Info, "envio_aceptado")
                        .con_id_conversacion(id_conversacion.clone())
                        .con_latencia_ms(latencia_ms(inicio)),
                );
                if let Err(error) =
                    self.conversaciones
                        .registrar_saliente(conversacion, &mensaje, marca_temporal)
                {
                    emitir(
                        EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                            .con_id_conversacion(id_conversacion)
                            .con_latencia_ms(latencia_ms(inicio))
                            .con_detalle(format!(
                                "no se pudo anotar la respuesta enviada en el historial: {error}"
                            )),
                    );
                }
            }
            Ok(ResultadoEnvio::FueraDeVentana) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Info, "envio_diferido")
                        .con_id_conversacion(id_conversacion)
                        .con_latencia_ms(latencia_ms(inicio)),
                );
                self.conversaciones.encolar_diferida(conversacion, mensaje);
            }
            Ok(ResultadoEnvio::PlantillaRequerida) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "envio_rechazado")
                        .con_id_conversacion(id_conversacion)
                        .con_latencia_ms(latencia_ms(inicio))
                        .con_detalle("el canal exige una plantilla aprobada"),
                );
            }
            Ok(ResultadoEnvio::LimiteDeTasa) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "envio_rechazado")
                        .con_id_conversacion(id_conversacion)
                        .con_latencia_ms(latencia_ms(inicio))
                        .con_detalle("el canal está limitando la tasa de envío"),
                );
            }
            Ok(ResultadoEnvio::DestinatarioInvalido) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "envio_rechazado")
                        .con_id_conversacion(id_conversacion)
                        .con_latencia_ms(latencia_ms(inicio))
                        .con_detalle("el destinatario no es válido"),
                );
            }
            Err(averia) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Error, "averia_de_transporte")
                        .con_id_conversacion(id_conversacion)
                        .con_latencia_ms(latencia_ms(inicio))
                        .con_detalle(format!("avería de transporte al enviar: {averia}")),
                );
            }
        }
    }
}

/// Milisegundos transcurridos desde `inicio`, medidos con el reloj monótono del proceso.
///
/// Único punto de este módulo —y de todo `crates/hexcell/src/`, salvo aquí— donde se permite leer
/// `Instant::now()`: mide exclusivamente latencia de procesamiento para el registro estructurado y
/// nunca alimenta la deduplicación ni el historial, que siguen midiéndose contra la marca temporal
/// del propio evento (`docs/adr/adr-0018-apagado-ordenado.md`).
fn latencia_ms(inicio: Instant) -> u64 {
    u64::try_from(Instant::now().duration_since(inicio).as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procesador::ProcesadorDeEco;
    use crate::registro;
    use hexcell_core::canal::{
        EstadoVentanaServicio, EstadoVentanaServicio::Abierta, ResultadoEnvio::Aceptado,
    };
    use hexcell_core::identidad::{IdDeduplicacion, IdRemitente};
    use hexcell_storage::{GestorDePools, RepositorioDeSesiones};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    type R = Result<ResultadoEnvio, std::convert::Infallible>;
    type V = Result<EstadoVentanaServicio, std::convert::Infallible>;
    type M = Motor<Dummy, ProcesadorDeEco>;

    struct Dummy;
    impl ChannelAdapter for Dummy {
        type Error = std::convert::Infallible;
        async fn send(&self, _: &IdConversacion, _: MensajeSaliente) -> R {
            Ok(Aceptado)
        }
        async fn estado_ventana(&self, _: &IdConversacion) -> V {
            Ok(Abierta {
                expira_en: SystemTime::UNIX_EPOCH,
            })
        }
    }

    /// Contador de directorios temporales de este proceso. Sustituye a la lectura de nanosegundos
    /// del reloj: su granularidad no garantiza unicidad, así que dos ayudantes construidos a la vez
    /// en hilos distintos podían leer el mismo instante y compartir directorio. Un contador atómico
    /// los distingue **por construcción**, sin depender del reloj del sistema.
    static SECUENCIA_DE_DIRECTORIOS: AtomicU64 = AtomicU64::new(0);

    fn motor(c: ConfiguracionGcra) -> (M, std::path::PathBuf) {
        let id_unico = SECUENCIA_DE_DIRECTORIOS.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("hx-m-{}-{}", std::process::id(), id_unico));
        // Antes este error se descartaba con `let _ =` y el fallo aparecía después, disfrazado de
        // fallo al abrir el pool. Nombrar la ruta y el error de origen es lo único que hace
        // diagnosticable un fallo intermitente a partir de la salida del test.
        if let Err(error) = std::fs::create_dir_all(&dir) {
            panic!(
                "no se pudo crear el directorio temporal del test «{}»: {error}",
                dir.display()
            );
        }
        let p = match GestorDePools::abrir(&dir) {
            Ok(p) => p,
            Err(error) => panic!(
                "no se pudo abrir el gestor de pools sobre «{}»: {error:?}",
                dir.display()
            ),
        };
        let repo = Arc::new(RepositorioDeSesiones::nuevo(Arc::new(p)));
        let (_, rx) = mpsc::channel(8);
        (
            M::nuevo(Dummy, ProcesadorDeEco, rx, Duration::from_secs(3600), repo)
                .con_configuracion_gcra(c),
            dir,
        )
    }

    fn evt(c: &IdConversacion, id: &str) -> EventoEntrante {
        EventoEntrante {
            remitente: IdRemitente::nuevo("r"),
            conversacion: c.clone(),
            contenido: "t".to_string(),
            marca_temporal: SystemTime::UNIX_EPOCH,
            deduplicacion: IdDeduplicacion::nuevo(id),
        }
    }

    #[tokio::test]
    async fn ac_1_ac_2_ac_3_discriminacion_descarte_y_admision() {
        let cfg = match ConfiguracionGcra::nueva(1.0, 0) {
            Ok(c) => c,
            Err(_) => panic!(),
        };
        let (mut m, dir) = motor(cfg);
        let conv_d = IdConversacion::nuevo("conv-descarte");
        let conv_a = IdConversacion::nuevo("conv-admitida");

        registro::pruebas::instalar();
        m.procesar_evento(evt(&conv_d, "dedup-1")).await;
        m.procesar_evento(evt(&conv_d, "dedup-2")).await;

        let logs = registro::pruebas::tomar();
        let desc: Vec<_> = logs
            .into_iter()
            .filter(|e| e.evento == "admision_descartada")
            .collect();
        assert_eq!(desc.len(), 1);
        assert_eq!(desc[0].nivel, NivelDeRegistro::Aviso);
        assert_eq!(desc[0].id_conversacion.as_deref(), Some("conv-descarte"));
        assert_eq!(desc[0].id_evento.as_deref(), Some("dedup-2"));
        assert!(desc[0].latencia_ms.is_some() && desc[0].detalle.is_some());

        registro::pruebas::instalar();
        m.procesar_evento(evt(&conv_a, "dedup-admitido")).await;

        let logs_a = registro::pruebas::tomar();
        assert!(!logs_a.is_empty());
        assert_eq!(
            logs_a
                .into_iter()
                .filter(|e| e.evento == "admision_descartada")
                .count(),
            0
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn descarte_por_saturacion_de_concurrencia_y_recuperacion() {
        let (mut m, dir) = motor(ConfiguracionGcra::default());
        let limitador = LimitadorDeConcurrencia::nuevo(1);
        m = m.con_limite_de_concurrencia(limitador.clone());

        let conv = IdConversacion::nuevo("conv-concurrencia");

        // Saturar externamente el limitador
        let permiso = match limitador.intentar_adquirir() {
            Some(p) => p,
            None => panic!(),
        };

        registro::pruebas::instalar();
        m.procesar_evento(evt(&conv, "dedup-conc-1")).await;

        let logs = registro::pruebas::tomar();
        // El descarte por saturación no debe dejar rastro de procesamiento posterior:
        // ni recepción, ni deduplicación, ni respuesta.
        assert_eq!(
            logs.iter()
                .filter(|e| e.evento != "concurrencia_descartada")
                .count(),
            0
        );
        let desc: Vec<_> = logs
            .into_iter()
            .filter(|e| e.evento == "concurrencia_descartada")
            .collect();
        assert_eq!(desc.len(), 1);
        assert_eq!(desc[0].nivel, NivelDeRegistro::Aviso);
        assert_eq!(
            desc[0].id_conversacion.as_deref(),
            Some("conv-concurrencia")
        );
        assert_eq!(desc[0].id_evento.as_deref(), Some("dedup-conc-1"));
        assert!(desc[0].latencia_ms.is_some() && desc[0].detalle.is_some());

        // Liberar el permiso y verificar que el siguiente evento sí se admite
        drop(permiso);

        registro::pruebas::instalar();
        m.procesar_evento(evt(&conv, "dedup-conc-2")).await;

        let logs_rec = registro::pruebas::tomar();
        assert_eq!(
            logs_rec
                .iter()
                .filter(|e| e.evento == "concurrencia_descartada")
                .count(),
            0
        );
        assert_eq!(
            logs_rec
                .iter()
                .filter(|e| e.evento == "evento_recibido")
                .count(),
            1
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn ac_1_conteo_de_admision_en_registro() {
        let Ok(cfg) = ConfiguracionGcra::nueva(1.0, 0) else {
            panic!("la configuración GCRA de prueba debe ser válida");
        };
        let (m, dir) = motor(cfg);
        let metricas = Arc::new(RegistroDeMetricas::nuevo());
        let mut m = m.con_metricas(metricas.clone());
        let conv_d = IdConversacion::nuevo("conv-descarte");

        m.procesar_evento(evt(&conv_d, "dedup-1")).await;
        m.procesar_evento(evt(&conv_d, "dedup-2")).await;

        assert_eq!(metricas.admitidos.load(Ordering::Relaxed), 1);
        assert_eq!(metricas.descartados_admision.load(Ordering::Relaxed), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn ac_2_conteo_de_concurrencia_en_registro() {
        let (m, dir) = motor(ConfiguracionGcra::default());
        let limitador = LimitadorDeConcurrencia::nuevo(1);
        let metricas = Arc::new(RegistroDeMetricas::nuevo());
        let mut m = m
            .con_limite_de_concurrencia(limitador.clone())
            .con_metricas(metricas.clone());
        let conv = IdConversacion::nuevo("conv-concurrencia");

        let Some(_permiso) = limitador.intentar_adquirir() else {
            panic!("el único permiso del limitador debe estar disponible");
        };

        registro::pruebas::instalar();
        m.procesar_evento(evt(&conv, "dedup-conc-1")).await;

        let logs = registro::pruebas::tomar();
        assert_eq!(
            logs.iter()
                .filter(|e| e.evento == "concurrencia_descartada")
                .count(),
            1
        );

        // El contador de admitidos cuenta el paso de la compuerta GCRA (definicion del
        // blueprint): un evento admitido por GCRA y descartado por concurrencia suma en ambos.
        assert_eq!(metricas.admitidos.load(Ordering::Relaxed), 1);
        assert_eq!(metricas.descartados_admision.load(Ordering::Relaxed), 0);
        assert_eq!(metricas.descartados_concurrencia.load(Ordering::Relaxed), 1);

        let _ = std::fs::remove_dir_all(dir);
    }
}
