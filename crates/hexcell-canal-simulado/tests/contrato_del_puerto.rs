//! Prueba de que la batería de `hexcell-canal-contrato` es genuinamente reutilizable.
//!
//! Este archivo consume la batería **desde fuera** del crate que la define, exactamente como hará
//! la etapa B-1 contra el adaptador de la Cloud API real: implementa `BancoDeCanal` y
//! `BancoConRelojControlable` para un tipo de banco propio de este archivo, que envuelve
//! `AdaptadorSimulado` junto con su `RelojDePrueba`, y a partir de ahí solo llama a las funciones
//! públicas de `hexcell_canal_contrato`. Es el único lugar de esta tarea donde el adaptador
//! simulado y la batería se encuentran.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hexcell_canal_contrato::{BancoConRelojControlable, BancoDeCanal, bateria};
use hexcell_canal_simulado::{AdaptadorSimulado, Reloj, RelojDePrueba};
use hexcell_core::canal::{EventoEntrante, ResultadoEnvio};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use tokio::sync::mpsc;

/// Banco de pruebas que envuelve un `AdaptadorSimulado` y su `RelojDePrueba`.
///
/// Guarda también el extremo receptor del canal `mpsc` que el adaptador crea al construirse: si
/// se soltara, el `Sender` interno del adaptador dejaría de tener a quién entregar, y
/// `inyectar` empezaría a devolver `Err` en vez de completar la inyección que la batería necesita.
struct BancoSimulado {
    adaptador: AdaptadorSimulado,
    reloj: RelojDePrueba,
    _receptor: mpsc::Receiver<EventoEntrante>,
}

impl BancoSimulado {
    fn nuevo() -> Self {
        let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
        let (adaptador, receptor) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 8);
        Self {
            adaptador,
            reloj,
            _receptor: receptor,
        }
    }
}

impl BancoDeCanal for BancoSimulado {
    type Adaptador = AdaptadorSimulado;

    fn adaptador(&self) -> &Self::Adaptador {
        &self.adaptador
    }

    async fn inyectar_evento(&self, conversacion: &IdConversacion) {
        let evento = EventoEntrante {
            remitente: IdRemitente::nuevo("remitente-de-contrato"),
            conversacion: conversacion.clone(),
            contenido: "evento de contrato".to_string(),
            marca_temporal: self.reloj.ahora(),
            deduplicacion: IdDeduplicacion::nuevo("dedup-de-contrato"),
        };
        self.adaptador
            .inyectar(evento)
            .await
            .expect("el canal recién creado del banco debe aceptar el evento");
    }

    fn forzar_resultado(&self, conversacion: &IdConversacion, resultado: ResultadoEnvio) {
        self.adaptador.forzar(conversacion, resultado);
    }
}

impl BancoConRelojControlable for BancoSimulado {
    fn avanzar(&self, duracion: Duration) {
        self.reloj.avanzar(duracion);
    }
}

#[tokio::test]
async fn la_bateria_de_contrato_completa_pasa_contra_el_adaptador_simulado() {
    let banco = BancoSimulado::nuevo();
    bateria::verificar_contrato_del_puerto(&banco).await;
}

#[tokio::test]
async fn escenario_de_los_cuatro_rechazos_forzables_pasa_de_forma_aislada() {
    let banco = BancoSimulado::nuevo();
    bateria::verifica_los_cuatro_rechazos_son_forzables_y_distinguibles(&banco).await;
}

#[tokio::test]
async fn escenario_de_rechazo_forzado_de_una_sola_llamada_pasa_de_forma_aislada() {
    let banco = BancoSimulado::nuevo();
    bateria::verifica_un_rechazo_forzado_se_aplica_a_una_sola_llamada(&banco).await;
}

#[tokio::test]
async fn escenario_de_asimetria_de_plantilla_pasa_de_forma_aislada() {
    let banco = BancoSimulado::nuevo();
    bateria::verifica_plantilla_se_acepta_donde_respuesta_libre_se_rechaza(&banco).await;
}

#[tokio::test]
async fn escenario_de_estado_de_ventana_concorde_pasa_de_forma_aislada() {
    let banco = BancoSimulado::nuevo();
    bateria::verifica_estado_ventana_concuerda_con_send(&banco).await;
}

#[tokio::test]
async fn escenario_de_ventana_abierta_pasa_de_forma_aislada() {
    let banco = BancoSimulado::nuevo();
    bateria::verifica_ventana_abierta_acepta_respuesta_libre(&banco).await;
}

#[tokio::test]
async fn escenario_de_expiracion_de_ventana_pasa_de_forma_aislada() {
    let banco = BancoSimulado::nuevo();
    bateria::verifica_ventana_expira_tras_duracion_de_servicio(&banco).await;
}
