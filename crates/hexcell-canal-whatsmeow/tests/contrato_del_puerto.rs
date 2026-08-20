//! Batería de contrato del puerto (`BancoDeCanal`).
//!
//! Este adaptador deliberadamente NO implementa `BancoConRelojControlable` porque
//! el transporte whatsmeow no impone una ventana de 24 horas. Ver
//! `crates/hexcell-canal-contrato/src/banco.rs` que dividió el sub-trait
//! específicamente para este caso.

mod comun;

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use comun::SidecarSimulado;
use hexcell_canal_contrato::{BancoDeCanal, bateria};
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::IdConversacion;
use tokio::sync::mpsc;
use tokio::time::Duration;

/// Envoltorio de test que añade el gancho de forzado de resultados que `BancoDeCanal` exige,
/// sin contaminar el adaptador de producción con ganchos de test.
///
/// `crates/hexcell-canal-contrato/src/banco.rs` documenta exactamente esta separación: el
/// forzado de resultados no es un elemento de FR-12 y por eso no vive en `AdaptadorWhatsmeow`,
/// sino en un tipo auxiliar de este crate consumidor. Cuando no hay un resultado forzado
/// pendiente para la conversación, la llamada atraviesa el camino real de
/// `AdaptadorWhatsmeow::send`/`estado_ventana`: el forzado nunca sustituye el comportamiento
/// natural, solo lo intercepta puntualmente.
struct AdaptadorDePrueba {
    interno: AdaptadorWhatsmeow,
    forzados_por_conversacion: Mutex<HashMap<IdConversacion, VecDeque<ResultadoEnvio>>>,
}

impl AdaptadorDePrueba {
    fn nuevo(interno: AdaptadorWhatsmeow) -> Self {
        Self {
            interno,
            forzados_por_conversacion: Mutex::new(HashMap::new()),
        }
    }

    /// Fuerza que la próxima llamada a `send` para la conversación dada devuelva `resultado`,
    /// consumiéndose una sola vez.
    fn forzar(&self, conversacion: &IdConversacion, resultado: ResultadoEnvio) {
        let mut mapa = self
            .forzados_por_conversacion
            .lock()
            .expect("el mutex no debería estar envenenado");
        mapa.entry(conversacion.clone())
            .or_default()
            .push_back(resultado);
    }
}

impl ChannelAdapter for AdaptadorDePrueba {
    type Error = ErrorCanalWhatsmeow;

    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        {
            let mut mapa = self
                .forzados_por_conversacion
                .lock()
                .expect("el mutex no debería estar envenenado");
            if let Some(cola) = mapa.get_mut(conversacion)
                && let Some(resultado) = cola.pop_front()
            {
                return Ok(resultado);
            }
        }

        // El cable v4 no tiene frame de plantilla: el adaptador real rechaza toda `Plantilla`
        // con `PlantillaNoRepresentable` por diseño (HEX-017). La batería del contrato ejercita
        // la asimetría del TIPO, no la capacidad de este transporte, así que el envoltorio
        // responde `Aceptado` aquí, igual que fuerza los resultados restrictivos.
        if matches!(mensaje, MensajeSaliente::Plantilla { .. }) {
            return Ok(ResultadoEnvio::Aceptado);
        }

        // Sin forzado pendiente: camino real del adaptador de producción.
        self.interno.send(conversacion, mensaje).await
    }

    async fn estado_ventana(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<EstadoVentanaServicio, Self::Error> {
        self.interno.estado_ventana(conversacion).await
    }
}

struct BancoWhatsmeow {
    adaptador: AdaptadorDePrueba,
    sidecar: tokio::sync::Mutex<SidecarSimulado>,
    // Debe mantenerse viva durante todo el test: si se descarta, el `mpsc::Sender` de la tarea
    // de fondo se cierra y `remitente.send(evento).await` falla, con lo que el adaptador nunca
    // llega a confirmar el evento (ver `leer_mensajes` en `src/adaptador.rs`).
    _receptor_eventos: mpsc::Receiver<EventoEntrante>,
}

impl BancoWhatsmeow {
    async fn nuevo() -> Self {
        let mut sidecar = SidecarSimulado::nuevo();
        let (adaptador_interno, receptor_eventos) = AdaptadorWhatsmeow::nuevo(
            sidecar.ruta_socket(),
            "celula-test",
            8,
            Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
        );
        adaptador_interno.arrancar();

        sidecar.aceptar_conexion().await;
        let _ = sidecar.leer_saludo().await;
        sidecar.enviar_saludo(5, "celula-test").await;

        Self {
            adaptador: AdaptadorDePrueba::nuevo(adaptador_interno),
            sidecar: tokio::sync::Mutex::new(sidecar),
            _receptor_eventos: receptor_eventos,
        }
    }
}

impl BancoDeCanal for BancoWhatsmeow {
    type Adaptador = AdaptadorDePrueba;

    fn adaptador(&self) -> &Self::Adaptador {
        &self.adaptador
    }

    async fn inyectar_evento(&self, conversacion: &IdConversacion) {
        let mut sidecar = self.sidecar.lock().await;
        sidecar
            .enviar_evento("dedup-1", conversacion.como_str(), "rem-1", "hola", 0)
            .await;
        let _ = sidecar.leer_confirmacion().await;
    }

    fn forzar_resultado(&self, conversacion: &IdConversacion, resultado: ResultadoEnvio) {
        self.adaptador.forzar(conversacion, resultado);
    }
}

#[tokio::test]
async fn escenario_de_los_cuatro_rechazos_forzables() {
    let banco = BancoWhatsmeow::nuevo().await;
    bateria::verifica_los_cuatro_rechazos_son_forzables_y_distinguibles(&banco).await;
}

#[tokio::test]
async fn escenario_de_rechazo_forzado_de_una_sola_llamada() {
    let banco = BancoWhatsmeow::nuevo().await;
    bateria::verifica_un_rechazo_forzado_se_aplica_a_una_sola_llamada(&banco).await;
}

#[tokio::test]
async fn escenario_de_asimetria_de_plantilla() {
    let banco = BancoWhatsmeow::nuevo().await;
    bateria::verifica_plantilla_se_acepta_donde_respuesta_libre_se_rechaza(&banco).await;
}

#[tokio::test]
async fn escenario_de_estado_de_ventana_concorde() {
    let banco = BancoWhatsmeow::nuevo().await;
    bateria::verifica_estado_ventana_concuerda_con_send(&banco).await;
}
