//! Pruebas de exhaustividad sobre los enumerados del puerto de canal.
//!
//! Viven en `tests/`, es decir, en un **crate externo** que solo ve la API visible de
//! `hexcell-core`. La ubicación es deliberada: como `ResultadoEnvio` se declara cerrado, un
//! consumidor externo lo sigue viendo exhaustivo y puede recorrerlo sin brazo comodín. Esa es la
//! propiedad fuerte que aquí se fija por escrito, y es más de lo que probaría un módulo interno.
//!
//! Ningún `match` de este archivo tiene brazo comodín ni patrón de resto. La consecuencia
//! buscada es que **añadir o quitar una variante rompa la compilación de estas pruebas**, no
//! solo la del crate: quien cambie la forma del enumerado tropieza aquí y tiene que decidir a
//! conciencia qué reacción corresponde al caso nuevo. El experimento que lo demuestra está
//! registrado en `docs/cotejo-puerto-de-canal-cloud-api.md`.
//!
//! Que estas pruebas pasen no certifica que el diseño del puerto sea semánticamente correcto.
//! Certifican su forma. La semántica restrictiva la certifican los tests de contrato de la
//! etapa A-2 contra el adaptador simulado.

use std::collections::HashSet;

use hexcell_core::canal::{Acuse, ResultadoEnvio};

/// Las cinco variantes del resultado de envío: un éxito y los cuatro fallos que fija FR-12.
const TODOS_LOS_RESULTADOS: [ResultadoEnvio; 5] = [
    ResultadoEnvio::Aceptado,
    ResultadoEnvio::FueraDeVentana,
    ResultadoEnvio::PlantillaRequerida,
    ResultadoEnvio::LimiteDeTasa,
    ResultadoEnvio::DestinatarioInvalido,
];

/// Los cuatro acuses normalizados que fija FR-12.
const TODOS_LOS_ACUSES: [Acuse; 4] = [
    Acuse::Enviado,
    Acuse::Entregado,
    Acuse::Leido,
    Acuse::Fallido,
];

/// Reacción que el núcleo debe tener ante cada desenlace del envío.
///
/// El valor devuelto es una etiqueta, no una política: la política ante `FueraDeVentana`
/// —encolar hasta que el cliente vuelva a escribir, o escalar a un humano— se decide en la etapa
/// A-2. Lo que esta función fija hoy es que **cada variante exige una reacción propia**, que es
/// justamente el motivo por el que FR-12 pide un resultado tipado y no un booleano.
fn reaccion_del_nucleo(resultado: ResultadoEnvio) -> &'static str {
    match resultado {
        ResultadoEnvio::Aceptado => "seguir; el desenlace llegará por acuse",
        ResultadoEnvio::FueraDeVentana => "no reintentar igual: la ventana está cerrada",
        ResultadoEnvio::PlantillaRequerida => "reintentar con una plantilla aprobada",
        ResultadoEnvio::LimiteDeTasa => "reintentar más tarde con retroceso",
        ResultadoEnvio::DestinatarioInvalido => "no reintentar nunca: el destino no sirve",
    }
}

/// Etapa del ciclo de vida del mensaje saliente que representa cada acuse.
///
/// Devuelve una etiqueta y no un booleano a propósito: un `match` que solo produjese `true` o
/// `false` colapsaría cuatro estados en dos y perdería la distinción que FR-12 pide normalizar.
fn etapa_del_acuse(acuse: Acuse) -> &'static str {
    match acuse {
        Acuse::Enviado => "aceptado por el canal",
        Acuse::Entregado => "recibido por el dispositivo",
        Acuse::Leido => "leído por el destinatario",
        Acuse::Fallido => "entrega fallida y definitiva",
    }
}

#[test]
fn el_resultado_de_envio_tiene_exactamente_cinco_variantes() {
    assert_eq!(
        TODOS_LOS_RESULTADOS.len(),
        5,
        "FR-12 fija el conjunto: un éxito y cuatro fallos. Ampliarlo es decisión del PRD."
    );
}

#[test]
fn cada_variante_del_resultado_exige_una_reaccion_distinta() {
    let mut reacciones = HashSet::new();
    for resultado in TODOS_LOS_RESULTADOS {
        assert!(
            reacciones.insert(reaccion_del_nucleo(resultado)),
            "dos variantes comparten reacción: {resultado:?}. Si de verdad la comparten, sobra una."
        );
    }
    assert_eq!(reacciones.len(), TODOS_LOS_RESULTADOS.len());
}

#[test]
fn los_cuatro_fallos_de_fr_12_estan_declarados_y_son_distintos_del_exito() {
    let fallos = [
        ResultadoEnvio::FueraDeVentana,
        ResultadoEnvio::PlantillaRequerida,
        ResultadoEnvio::LimiteDeTasa,
        ResultadoEnvio::DestinatarioInvalido,
    ];
    for fallo in fallos {
        assert_ne!(fallo, ResultadoEnvio::Aceptado);
        assert!(TODOS_LOS_RESULTADOS.contains(&fallo));
    }
    assert_eq!(fallos.len() + 1, TODOS_LOS_RESULTADOS.len());
}

#[test]
fn el_acuse_normalizado_tiene_cuatro_variantes_con_etapas_distintas() {
    assert_eq!(TODOS_LOS_ACUSES.len(), 4);
    let mut etapas = HashSet::new();
    for acuse in TODOS_LOS_ACUSES {
        assert!(
            etapas.insert(etapa_del_acuse(acuse)),
            "dos acuses comparten etapa: {acuse:?}"
        );
    }
    assert_eq!(etapas.len(), TODOS_LOS_ACUSES.len());
}
