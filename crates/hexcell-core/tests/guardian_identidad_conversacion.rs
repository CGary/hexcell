//! Guardián del mapeo de identidad de conversación.
//!
//! `adr-0010` (punto 5) reparte así la responsabilidad: **el adaptador traduce** el identificador
//! de transporte —el `wa_id` de Meta, el JID de whatsmeow— a un identificador interno, y el
//! núcleo recibe ese identificador ya traducido y lo trata como opaco. Esta prueba fija ese
//! reparto y falla si el identificador de transporte se filtra al dominio.
//!
//! Es una prueba **externa** a propósito: solo ve la API visible de `hexcell-core`, igual que la
//! verá el adaptador de la etapa A-3.
//!
//! El sustituto local de más abajo imita la tabla de mapeo que en producción vivirá dentro del
//! adaptador, sobre su propio almacén. **No implementa el puerto de canal**, y no puede hacerlo:
//! la etapa A-1 declara el contrato y no implementa ningún adaptador, ni real ni simulado.
//!
//! Y una advertencia que conviene dejar escrita: comprobar que ninguna firma nombra `wa_id` o
//! JID es una prueba **léxica**, necesaria pero insuficiente, porque el mismo error de diseño
//! puede repetirse con otro nombre. Lo que esta prueba añade es la parte semántica: que el
//! evento canónico se pueda construir **solo** con identidad ya traducida, y que lo que el
//! dominio muestre no contenga rastro del transporte. El experimento que demuestra que muerde
//! —introducir la filtración a propósito— está registrado en
//! `docs/cotejo-puerto-de-canal-cloud-api.md`.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hexcell_core::canal::EventoEntrante;
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};

/// Identificadores de transporte de muestra, con las dos formas que el producto verá.
///
/// Las dos primeras son JID al estilo de whatsmeow; la tercera, un `wa_id` al estilo de la Cloud
/// API. Ninguna de ellas debe aparecer jamás en lo que el núcleo maneja.
const IDENTIFICADORES_DE_TRANSPORTE: [&str; 3] = [
    "5491122334455@s.whatsapp.net",
    "5491199887766@s.whatsapp.net",
    "5491133221100",
];

/// Sustituto local de la tabla de mapeo que pertenece al adaptador.
///
/// Existe solo dentro de esta prueba. En producción esta tabla persiste en el almacén propio del
/// adaptador, separado de las credenciales de sesión del transporte para sobrevivir al
/// re-emparejamiento que sigue a una desvinculación (`adr-0010`, punto 6).
#[derive(Default)]
struct TablaDeMapeo {
    asignados: HashMap<String, IdConversacion>,
}

impl TablaDeMapeo {
    /// Traduce un identificador de transporte al identificador interno de conversación.
    fn conversacion(&mut self, identificador_de_transporte: &str) -> IdConversacion {
        let huella = huella_estable(identificador_de_transporte);
        self.asignados
            .entry(identificador_de_transporte.to_owned())
            .or_insert_with(|| IdConversacion::nuevo(format!("conv-{huella:016x}")))
            .clone()
    }

    /// Traduce un identificador de transporte al identificador interno de remitente.
    fn remitente(&self, identificador_de_transporte: &str) -> IdRemitente {
        let huella = huella_estable(identificador_de_transporte);
        IdRemitente::nuevo(format!("rem-{huella:016x}"))
    }
}

fn huella_estable(valor: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    valor.hash(&mut hasher);
    hasher.finish()
}

/// Fragmentos del identificador de transporte que no deben aparecer en el dominio.
///
/// Se comprueba el valor entero y también su parte local, porque un mapeo perezoso que se
/// limitase a recortar el dominio del JID seguiría arrastrando el número de teléfono.
fn fragmentos_prohibidos(identificador_de_transporte: &str) -> Vec<&str> {
    let parte_local = identificador_de_transporte
        .split('@')
        .next()
        .unwrap_or(identificador_de_transporte);
    vec![identificador_de_transporte, parte_local]
}

#[test]
fn el_mapeo_es_estable_para_el_mismo_identificador_de_transporte() {
    let mut tabla = TablaDeMapeo::default();
    for identificador in IDENTIFICADORES_DE_TRANSPORTE {
        let primera = tabla.conversacion(identificador);
        let segunda = tabla.conversacion(identificador);
        assert_eq!(
            primera, segunda,
            "el mismo contacto debe caer siempre en el mismo hilo"
        );
    }
}

#[test]
fn el_mapeo_es_inyectivo_entre_identificadores_de_transporte_distintos() {
    let mut tabla = TablaDeMapeo::default();
    let mut vistos: Vec<IdConversacion> = Vec::new();
    for identificador in IDENTIFICADORES_DE_TRANSPORTE {
        let conversacion = tabla.conversacion(identificador);
        assert!(
            !vistos.contains(&conversacion),
            "dos contactos distintos cayeron en la misma conversación"
        );
        vistos.push(conversacion);
    }
    assert_eq!(vistos.len(), IDENTIFICADORES_DE_TRANSPORTE.len());
}

#[test]
fn el_identificador_interno_no_contiene_rastro_del_transporte() {
    let mut tabla = TablaDeMapeo::default();
    for identificador in IDENTIFICADORES_DE_TRANSPORTE {
        let conversacion = tabla.conversacion(identificador);
        let representacion = format!("{conversacion:?}");
        for fragmento in fragmentos_prohibidos(identificador) {
            assert!(
                !representacion.contains(fragmento),
                "el identificador interno {representacion} filtra el fragmento {fragmento}"
            );
        }
    }
}

#[test]
fn el_evento_canonico_se_construye_solo_con_identidad_traducida() {
    let mut tabla = TablaDeMapeo::default();
    let identificador = IDENTIFICADORES_DE_TRANSPORTE[0];

    // Si alguien añade al evento un campo con dato de transporte, esta construcción deja de
    // compilar y la prueba muerde antes incluso de ejecutarse. Es el efecto buscado.
    let evento = EventoEntrante {
        remitente: tabla.remitente(identificador),
        conversacion: tabla.conversacion(identificador),
        contenido: "hola, ¿tienen stock?".to_owned(),
        marca_temporal: UNIX_EPOCH + Duration::from_secs(1_785_000_000),
        deduplicacion: IdDeduplicacion::nuevo("evt-000000000001"),
    };

    assert!(evento.marca_temporal >= UNIX_EPOCH);
    assert!(evento.marca_temporal <= SystemTime::now());

    let representacion = format!("{evento:?}");
    for fragmento in fragmentos_prohibidos(identificador) {
        assert!(
            !representacion.contains(fragmento),
            "el evento canónico filtra el fragmento {fragmento} del transporte"
        );
    }
}
