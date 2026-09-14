//! Evaluación y emisión de alertas operativas sobre señales ya existentes.
//!
//! Siete de las ocho condiciones de alerta de la tarea 20 del plan (`docs/plan/fase-a-6-empaquetado-cli.md`,
//! líneas 320-331), cada una consumiendo una señal que ya existe en el sidecar (etapa A-3) o en el
//! núcleo (etapa A-4). La octava condición —bucle de reinicios de cualquiera de los dos contenedores—
//! queda explícitamente fuera de esta tarea por decisión humana del 2026-09-13: no existe ningún
//! productor de señal para ella en el repositorio.
//!
//! # Arquitectura
//!
//! - [`CodigoDeAlerta`]: valor opaco que identifica cada condición.
//! - [`UmbralesDeAlerta`]: parámetros de configuración, sin ningún valor normativo afirmado.
//! - [`EstadoDeEvaluacion`]: entidad que recuerda qué condición está activa y si ya se emitió,
//!   para garantizar la regla de «exactamente una notificación» por ocurrencia.
//! - [`EvaluadorDeAlertas`]: servicio de aplicación puro —sin I/O, sin reloj propio— que evalúa
//!   las señales observadas y devuelve las notificaciones pendientes.
//! - [`EmisorDeAlertas`]: combina el evaluador con el sumidero de notificación y despacha.
//!
//! # Invariante de exactly-one
//!
//! Cada criterio de aceptación exige «exactamente una notificación». Un canal `watch` re-entrega
//! en cada observación y el tick de 60 s re-evalúa indefinidamente, así que el evaluador mantiene
//! estado por condición: si la condición persiste, la segunda evaluación no produce nada. Cuando
//! la condición se despeja, el estado se reinicia.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hexcell_core::canal::EstadoSesion;
use hexcell_core::identidad::IdConversacion;
use hexcell_core::notificacion::{
    CodigoDeNotificacion, Notificacion, SumideroDeNotificaciones, ValorDeDato,
};
use tokio::sync::Mutex;

use crate::metricas::InstantaneaDeMetricas;
use crate::notificacion::SumideroDeCelula;

// ---------------------------------------------------------------------------
// Códigos de alerta
// ---------------------------------------------------------------------------

/// Código de alerta para la condición de baneo temporal detectado (AC-2).
pub const CODIGO_BANEO_TEMPORAL: &str = "baneo_temporal_detectado";
/// Código de alerta para la condición de sesión de canal desvinculada (AC-3).
pub const CODIGO_SESION_DESVINCULADA: &str = "sesion_desvinculada";
/// Código de alerta para la condición de sidecar sin reconectar tras la ventana configurada (AC-4).
pub const CODIGO_SIN_RECONECTAR: &str = "sidecar_sin_reconectar";
/// Código de alerta para la condición de saldo LLM agotado o modo degradado (AC-6).
pub const CODIGO_SALDO_AGOTADO: &str = "saldo_llm_agotado_o_modo_degradado";
/// Código de alerta para la condición de tasa anómala de descartes GCRA (AC-7).
pub const CODIGO_DESCARTES_GCRA: &str = "tasa_descartes_gcra_anomala";
/// Código de alerta para la condición de descarte de envío no solicitado (AC-8, proxy).
pub const CODIGO_ENVIO_NO_SOLICITADO: &str = "descarte_envio_no_solicitado";
/// Código de alerta para la condición de caída anómala del ratio de acuses por contacto (AC-9).
pub const CODIGO_CAIDA_ACUSES: &str = "caida_anomala_ratio_acuses_por_contacto";

/// Valor opaco que identifica una condición de alerta.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodigoDeAlerta(String);

impl CodigoDeAlerta {
    /// Construye el código a partir de un valor ya decidido.
    pub fn nuevo(valor: impl Into<String>) -> Self {
        Self(valor.into())
    }

    /// Vista prestada del valor opaco.
    pub fn como_str(&self) -> &str {
        &self.0
    }
}

impl From<CodigoDeAlerta> for CodigoDeNotificacion {
    fn from(codigo: CodigoDeAlerta) -> Self {
        CodigoDeNotificacion::nuevo(codigo.0)
    }
}

// ---------------------------------------------------------------------------
// Umbrales
// ---------------------------------------------------------------------------

/// Parámetros de configuración de las alertas, sin ningún valor normativo afirmado como correcto.
///
/// Los valores de respaldo permiten que la célula arranque sin configuración, pero ningún criterio
/// de aceptación fija ninguno de ellos como el valor correcto. La calibración definitiva se hará
/// contra datos reales de producción.
#[derive(Clone, Debug)]
pub struct UmbralesDeAlerta {
    /// Ventana de silencio de reconexión antes de alertar (AC-4).
    pub ventana_reconexion: Duration,
    /// Suelo de presupuesto disponible por debajo del cual se alerta (AC-6).
    pub suelo_balance_disponible: i64,
    /// Límite superior de la tasa de descartes GCRA (descartados/admitidos) (AC-7).
    pub limite_tasa_descartes: f64,
    /// Límite inferior del ratio de acuses por contacto por debajo del cual se alerta (AC-9).
    pub limite_caida_ratio_acuses: f64,
    /// Mínimo de envíos observados para una conversación antes de evaluar su ratio de acuses (AC-9).
    pub minimo_envios_para_evaluar_acuse: u64,
}

impl UmbralesDeAlerta {
    /// Valores de respaldo para que la célula arranque sin configuración.
    ///
    /// **Ninguno de estos valores se afirma como correcto.** Son puntos de arranque provisionales;
    /// la calibración definitiva se hará contra datos reales.
    pub fn por_defecto() -> Self {
        Self {
            ventana_reconexion: Duration::from_secs(300),
            suelo_balance_disponible: 0,
            limite_tasa_descartes: 0.5,
            limite_caida_ratio_acuses: 0.5,
            minimo_envios_para_evaluar_acuse: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Estado de evaluación
// ---------------------------------------------------------------------------

/// Estado mutable que el evaluador mantiene para garantizar exactly-one por condición.
///
/// Recuerda qué condición está activa y si ya se emitió la notificación correspondiente.
/// Cuando la condición se despeja, la entrada se reinicia.
#[derive(Debug, Default)]
pub struct EstadoDeEvaluacion {
    /// Último estado de sesión observado.
    ultimo_estado_sesion: Option<EstadoSesion>,
    /// Si ya se emitió alerta para el estado de sesión actual.
    alerta_emitida_para_estado: bool,
    /// Instante en que se observó el inicio de Reconectando (para AC-4).
    inicio_reconectando: Option<SystemTime>,
    /// Si ya se emitió alerta de balance agotado.
    alerta_balance_emitida: bool,
    /// Si ya se emitió alerta de tasa GCRA anómala.
    alerta_gcra_emitida: bool,
    /// Último valor observado del contador de rechazos de construcción (para delta).
    ultimo_rechazos_construccion: u64,
    /// Si ya se emitió alerta de envío no solicitado en el último ciclo.
    alerta_envio_no_solicitado_emitida: bool,
    /// Conjunto de conversaciones para las que ya se emitió alerta de caída de acuses.
    contactos_con_alerta_de_acuses: HashSet<IdConversacion>,
}

// ---------------------------------------------------------------------------
// Evaluador
// ---------------------------------------------------------------------------

/// Evaluador puro de alertas: recibe señales observadas y devuelve notificaciones pendientes.
///
/// No tiene I/O ni reloj propio; el instante se recibe como parámetro para que los tests lo
/// controlen. Mantiene [`EstadoDeEvaluacion`] internamente para la regla de exactly-one.
pub struct EvaluadorDeAlertas {
    umbrales: UmbralesDeAlerta,
    estado: EstadoDeEvaluacion,
}

impl EvaluadorDeAlertas {
    /// Construye el evaluador con los umbrales dados.
    pub fn nuevo(umbrales: UmbralesDeAlerta) -> Self {
        Self {
            umbrales,
            estado: EstadoDeEvaluacion::default(),
        }
    }

    /// Evalúa las condiciones de alerta derivadas del estado de sesión (AC-2, AC-3, AC-4).
    ///
    /// - AC-2: `EstadoSesion::Pausada` con fecha de expiración → baneo temporal.
    /// - AC-3: `EstadoSesion::Desvinculada` → sesión desvinculada.
    /// - AC-4: `EstadoSesion::Reconectando` sostenido más allá de la ventana → sin reconectar.
    pub fn evaluar_estado_de_sesion(
        &mut self,
        estado: EstadoSesion,
        expira_en: Option<SystemTime>,
        ahora: SystemTime,
    ) -> Vec<Notificacion> {
        let mut notificaciones = Vec::new();

        let estado_cambio = self.estado.ultimo_estado_sesion != Some(estado);

        if estado_cambio {
            self.estado.alerta_emitida_para_estado = false;
            self.estado.ultimo_estado_sesion = Some(estado);
            if estado == EstadoSesion::Reconectando {
                self.estado.inicio_reconectando = Some(ahora);
            } else {
                self.estado.inicio_reconectando = None;
            }
        }

        match estado {
            EstadoSesion::Pausada => {
                if !self.estado.alerta_emitida_para_estado {
                    self.estado.alerta_emitida_para_estado = true;
                    let mut n =
                        Notificacion::nueva(CodigoDeNotificacion::nuevo(CODIGO_BANEO_TEMPORAL));
                    if let Some(expira) = expira_en {
                        n = n.con_dato("expira_en", ValorDeDato::Instante(expira));
                    }
                    notificaciones.push(n);
                }
            }
            EstadoSesion::Desvinculada => {
                if !self.estado.alerta_emitida_para_estado {
                    self.estado.alerta_emitida_para_estado = true;
                    notificaciones.push(Notificacion::nueva(CodigoDeNotificacion::nuevo(
                        CODIGO_SESION_DESVINCULADA,
                    )));
                }
            }
            EstadoSesion::Reconectando => {
                if !self.estado.alerta_emitida_para_estado
                    && let Some(inicio) = self.estado.inicio_reconectando
                    && ahora.duration_since(inicio).unwrap_or(Duration::ZERO)
                        >= self.umbrales.ventana_reconexion
                {
                    self.estado.alerta_emitida_para_estado = true;
                    notificaciones.push(Notificacion::nueva(CodigoDeNotificacion::nuevo(
                        CODIGO_SIN_RECONECTAR,
                    )));
                }
            }
            EstadoSesion::Activa => {}
        }

        notificaciones
    }

    /// Evalúa las condiciones de alerta derivadas de la instantánea de métricas (AC-6, AC-7, AC-8).
    ///
    /// - AC-6: `disponible <= suelo` → saldo agotado o modo degradado.
    /// - AC-7: `descartados_admision / admitidos > límite` → tasa GCRA anómala.
    /// - AC-8: `rechazos_de_construccion` delta > 0 → proxy de envío no solicitado.
    pub fn evaluar_instantanea(
        &mut self,
        instantanea: &InstantaneaDeMetricas,
        rechazos_construccion_actual: u64,
    ) -> Vec<Notificacion> {
        let mut notificaciones = Vec::new();

        let balance_bajo = instantanea.disponible <= self.umbrales.suelo_balance_disponible;
        if balance_bajo && !self.estado.alerta_balance_emitida {
            self.estado.alerta_balance_emitida = true;
            notificaciones.push(Notificacion::nueva(CodigoDeNotificacion::nuevo(
                CODIGO_SALDO_AGOTADO,
            )));
        } else if !balance_bajo {
            self.estado.alerta_balance_emitida = false;
        }

        let tasa_descartes = if instantanea.admitidos > 0 {
            instantanea.descartados_admision as f64 / instantanea.admitidos as f64
        } else {
            0.0
        };
        let gcra_anomalo = tasa_descartes > self.umbrales.limite_tasa_descartes;
        if gcra_anomalo && !self.estado.alerta_gcra_emitida {
            self.estado.alerta_gcra_emitida = true;
            notificaciones.push(Notificacion::nueva(CodigoDeNotificacion::nuevo(
                CODIGO_DESCARTES_GCRA,
            )));
        } else if !gcra_anomalo {
            self.estado.alerta_gcra_emitida = false;
        }

        let delta_rechazos =
            rechazos_construccion_actual.saturating_sub(self.estado.ultimo_rechazos_construccion);
        self.estado.ultimo_rechazos_construccion = rechazos_construccion_actual;

        if delta_rechazos > 0 && !self.estado.alerta_envio_no_solicitado_emitida {
            self.estado.alerta_envio_no_solicitado_emitida = true;
            notificaciones.push(Notificacion::nueva(CodigoDeNotificacion::nuevo(
                CODIGO_ENVIO_NO_SOLICITADO,
            )));
        } else if delta_rechazos == 0 {
            self.estado.alerta_envio_no_solicitado_emitida = false;
        }

        notificaciones
    }

    /// Evalúa la condición de alerta de caída anómala del ratio de acuses por contacto (AC-9).
    ///
    /// La evaluación es **por contacto** (`id_conversacion`), nunca en agregado. Un contacto cuyo
    /// ratio caiga por debajo del umbral produce una notificación con su identificador; los demás
    /// contactos sanos no producen nada.
    ///
    /// `contadores` es una lista de `(id_conversacion, enviados, acusados)`.
    pub fn evaluar_acuses_por_contacto(
        &mut self,
        contadores: &[(IdConversacion, u64, u64)],
    ) -> Vec<Notificacion> {
        let mut notificaciones = Vec::new();
        let mut contactos_vistos = HashSet::new();

        for (id_conversacion, enviados, acusados) in contadores {
            contactos_vistos.insert(id_conversacion.clone());

            if *enviados < self.umbrales.minimo_envios_para_evaluar_acuse {
                continue;
            }

            let ratio = if *enviados > 0 {
                *acusados as f64 / *enviados as f64
            } else {
                1.0
            };

            let caida_anomala = ratio < self.umbrales.limite_caida_ratio_acuses;

            if caida_anomala
                && !self
                    .estado
                    .contactos_con_alerta_de_acuses
                    .contains(id_conversacion)
            {
                self.estado
                    .contactos_con_alerta_de_acuses
                    .insert(id_conversacion.clone());
                notificaciones.push(
                    Notificacion::nueva(CodigoDeNotificacion::nuevo(CODIGO_CAIDA_ACUSES)).con_dato(
                        "conversacion_afectada",
                        ValorDeDato::Conversacion(id_conversacion.clone()),
                    ),
                );
            } else if !caida_anomala {
                self.estado
                    .contactos_con_alerta_de_acuses
                    .remove(id_conversacion);
            }
        }

        let contactos_a_limpiar: Vec<_> = self
            .estado
            .contactos_con_alerta_de_acuses
            .difference(&contactos_vistos)
            .cloned()
            .collect();
        for id in contactos_a_limpiar {
            self.estado.contactos_con_alerta_de_acuses.remove(&id);
        }

        notificaciones
    }

    /// Devuelve una referencia al estado de evaluación actual, para inspección en tests.
    pub fn estado(&self) -> &EstadoDeEvaluacion {
        &self.estado
    }
}

// ---------------------------------------------------------------------------
// Emisor
// ---------------------------------------------------------------------------

/// Servicio de aplicación que combina el evaluador con el sumidero y despacha notificaciones.
///
/// La interior mutabilidad del evaluador permite compartir el emisor por `Arc` entre varias tareas
/// (el watcher de estado de sesión y el tick de métricas).
pub struct EmisorDeAlertas {
    evaluador: Mutex<EvaluadorDeAlertas>,
    sumidero: SumideroDeCelula,
}

impl EmisorDeAlertas {
    /// Construye el emisor a partir del evaluador y el sumidero de la célula.
    pub fn nuevo(umbrales: UmbralesDeAlerta, sumidero: SumideroDeCelula) -> Self {
        Self {
            evaluador: Mutex::new(EvaluadorDeAlertas::nuevo(umbrales)),
            sumidero,
        }
    }

    /// Evalúa las condiciones de estado de sesión y emite las notificaciones resultantes.
    pub async fn evaluar_y_emitir_estado(
        &self,
        estado: EstadoSesion,
        expira_en: Option<SystemTime>,
        ahora: SystemTime,
    ) {
        let notificaciones = {
            let mut evaluador = self.evaluador.lock().await;
            evaluador.evaluar_estado_de_sesion(estado, expira_en, ahora)
        };
        for n in notificaciones {
            let _ = self.sumidero.notificar(n).await;
        }
    }

    /// Evalúa las condiciones de instantánea de métricas y emite las notificaciones resultantes.
    pub async fn evaluar_y_emitir_instantanea(
        &self,
        instantanea: &InstantaneaDeMetricas,
        rechazos_construccion_actual: u64,
    ) {
        let notificaciones = {
            let mut evaluador = self.evaluador.lock().await;
            evaluador.evaluar_instantanea(instantanea, rechazos_construccion_actual)
        };
        for n in notificaciones {
            let _ = self.sumidero.notificar(n).await;
        }
    }

    /// Evalúa la condición de acuses por contacto y emite las notificaciones resultantes.
    pub async fn evaluar_y_emitir_acuses(&self, contadores: &[(IdConversacion, u64, u64)]) {
        let notificaciones = {
            let mut evaluador = self.evaluador.lock().await;
            evaluador.evaluar_acuses_por_contacto(contadores)
        };
        for n in notificaciones {
            let _ = self.sumidero.notificar(n).await;
        }
    }
}

/// Convierte un valor de `expira_en_ms` (milisegundos absolutos desde la época Unix) a `SystemTime`.
pub fn expira_en_ms_a_system_time(expira_en_ms: i64) -> Option<SystemTime> {
    if expira_en_ms > 0 {
        Some(UNIX_EPOCH + Duration::from_millis(expira_en_ms as u64))
    } else {
        None
    }
}
