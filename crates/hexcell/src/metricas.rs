//! Registro y emisión de métricas operativas de la célula.
//!
//! Este módulo agrupa los contadores atómicos locales y las utilidades para tomar
//! instantáneas periódicas del rendimiento y estado de la célula.

use crate::concurrencia::LimitadorDeConcurrencia;
use crate::registro::{EntradaDeRegistro, NivelDeRegistro};
use hexcell_storage::RepositorioDeSesiones;
use hexcell_storage::error::ErrorDeAlmacen;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Intervalo periódico para la emisión de instantáneas de métricas.
pub const INTERVALO_DE_INSTANTANEA: Duration = Duration::from_secs(60);

/// Registro en memoria de contadores atómicos locales de la célula.
pub struct RegistroDeMetricas {
    pub(crate) admitidos: AtomicU64,
    pub(crate) descartados_admision: AtomicU64,
    pub(crate) descartados_concurrencia: AtomicU64,
}

impl RegistroDeMetricas {
    /// Crea un nuevo registro con todos los contadores en cero.
    pub fn nuevo() -> Self {
        Self {
            admitidos: AtomicU64::new(0),
            descartados_admision: AtomicU64::new(0),
            descartados_concurrencia: AtomicU64::new(0),
        }
    }

    /// Incrementa el contador de eventos admitidos.
    pub fn anotar_evento_admitido(&self) {
        self.admitidos.fetch_add(1, Ordering::Relaxed);
    }

    /// Incrementa el contador de descartes por admisión.
    pub fn anotar_descarte_por_admision(&self) {
        self.descartados_admision.fetch_add(1, Ordering::Relaxed);
    }

    /// Incrementa el contador de descartes por concurrencia.
    pub fn anotar_descarte_por_concurrencia(&self) {
        self.descartados_concurrencia
            .fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for RegistroDeMetricas {
    fn default() -> Self {
        Self::nuevo()
    }
}

/// Instantánea inmutable que captura el valor actual de todas las métricas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstantaneaDeMetricas {
    /// Total de eventos admitidos procesados en esta ejecución.
    pub admitidos: u64,
    /// Total de eventos descartados por la admisión de la célula.
    pub descartados_admision: u64,
    /// Total de eventos descartados por superar la concurrencia máxima.
    pub descartados_concurrencia: u64,
    /// Cantidad de tareas concurrentes actualmente en vuelo.
    pub en_vuelo: u64,
    /// Saldo de presupuesto disponible.
    pub disponible: i64,
    /// Saldo de presupuesto reservado en holds activos.
    pub reservado: i64,
    /// Desviación acumulada por conciliación de reservas.
    pub desviacion: i64,
}

/// Obtiene una instantánea actual recopilando la información de los diferentes componentes.
pub fn tomar_instantanea(
    registro: &RegistroDeMetricas,
    limitador: &LimitadorDeConcurrencia,
    repositorio: &RepositorioDeSesiones,
) -> Result<InstantaneaDeMetricas, ErrorDeAlmacen> {
    let saldo = repositorio.saldo()?;
    let desviacion = repositorio.desviacion_de_conciliacion()?;
    Ok(InstantaneaDeMetricas {
        admitidos: registro.admitidos.load(Ordering::Relaxed),
        descartados_admision: registro.descartados_admision.load(Ordering::Relaxed),
        descartados_concurrencia: registro.descartados_concurrencia.load(Ordering::Relaxed),
        en_vuelo: limitador.en_vuelo() as u64,
        disponible: saldo.disponible,
        reservado: saldo.reservado,
        desviacion,
    })
}

/// Emite una línea de registro estructurado con los detalles de la instantánea.
pub fn emitir_instantanea(instantanea: &InstantaneaDeMetricas) {
    let detalle = format!(
        "admitidos={} descartados_admision={} descartados_concurrencia={} en_vuelo={} disponible={} reservado={} desviacion={}",
        instantanea.admitidos,
        instantanea.descartados_admision,
        instantanea.descartados_concurrencia,
        instantanea.en_vuelo,
        instantanea.disponible,
        instantanea.reservado,
        instantanea.desviacion
    );
    let entrada = EntradaDeRegistro::nueva(NivelDeRegistro::Info, "metricas_instantanea")
        .con_detalle(detalle);
    crate::registro::emitir(entrada);
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn renderizado_de_instantanea_es_determinista() {
        let instantanea = InstantaneaDeMetricas {
            admitidos: 10,
            descartados_admision: 2,
            descartados_concurrencia: 1,
            en_vuelo: 0,
            disponible: 500,
            reservado: 50,
            desviacion: -5,
        };

        let detalle = format!(
            "admitidos={} descartados_admision={} descartados_concurrencia={} en_vuelo={} disponible={} reservado={} desviacion={}",
            instantanea.admitidos,
            instantanea.descartados_admision,
            instantanea.descartados_concurrencia,
            instantanea.en_vuelo,
            instantanea.disponible,
            instantanea.reservado,
            instantanea.desviacion
        );

        assert_eq!(
            detalle,
            "admitidos=10 descartados_admision=2 descartados_concurrencia=1 en_vuelo=0 disponible=500 reservado=50 desviacion=-5"
        );
    }
}
