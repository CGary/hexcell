//! AC-6: ningún identificador de transporte —JID, teléfono, id de dispositivo o push name—
//! llega al núcleo ni a la superficie del adaptador.
//!
//! El test anterior formateaba en `Debug` los cuatro valores de `EstadoSesion`, un enum sin
//! campos: su salida es una constante en tiempo de compilación y la aserción nunca podía
//! fallar. Este test conecta un `SidecarSimulado` real, envía un `evento_entrante` y las cuatro
//! variantes de `estado_sesion` por el cable, y examina los valores que el adaptador realmente
//! entrega: el `EventoEntrante` que llega al motor y el `EstadoSesion` que proyecta, incluso
//! cuando el propio cable transporta términos proscritos en los campos que el diseño confina a
//! este crate (`causa`, `codigo`, `expira_en_ms` de `estado_sesion`, que nunca se elevan al
//! dominio: ver `src/mensajes.rs` y `src/adaptador.rs`).

mod comun;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::EstadoSesion;
use tokio::time::Duration;

const TERMINOS_PROSCRITOS: [&str; 6] = [
    "jid",
    "telefono",
    "phone",
    "dispositivo",
    "device",
    "numero",
];

fn sin_terminos_proscritos(texto: &str) -> bool {
    let normalizado = texto.to_lowercase();
    TERMINOS_PROSCRITOS.iter().all(|p| !normalizado.contains(p))
}

#[tokio::test]
async fn evento_y_estado_de_sesion_no_filtran_identificadores_de_transporte() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut receptor_eventos) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-privacidad",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(5, "celula-privacidad").await;

    // El evento entrante lleva ids opacos normales, tal como los minta el almacén de identidad
    // del sidecar (HEX-014). `contenido` es la única superficie pensada para texto libre de
    // usuario, así que puede llevar cualquier cosa —incluido un número de teléfono— sin que eso
    // sea una fuga de este adaptador.
    sidecar
        .enviar_evento(
            "dedup-privacidad-1",
            "conv-opaca-1",
            "rem-opaco-1",
            "mi telefono es 5511999999999",
            0,
        )
        .await;
    let evento = receptor_eventos
        .recv()
        .await
        .expect("el motor debe recibir el evento");
    assert_eq!(evento.remitente.como_str(), "rem-opaco-1");
    assert_eq!(evento.conversacion.como_str(), "conv-opaca-1");
    assert!(
        sin_terminos_proscritos(evento.remitente.como_str()),
        "el identificador de remitente entregado al núcleo no debe llevar términos de \
         transporte proscritos"
    );
    assert!(
        sin_terminos_proscritos(evento.conversacion.como_str()),
        "el identificador de conversación entregado al núcleo no debe llevar términos de \
         transporte proscritos"
    );
    let _ = sidecar.leer_confirmacion().await;

    // El cable SÍ transporta causa/código/expiración con texto que, si el adaptador los elevara
    // al dominio —lo que el diseño prohíbe expresamente—, filtraría justo los términos
    // proscritos. Se prueban los cuatro estados de sesión del protocolo.
    let casos = [
        ("activa", EstadoSesion::Activa),
        ("reconectando", EstadoSesion::Reconectando),
        ("desvinculada", EstadoSesion::Desvinculada),
        ("pausada", EstadoSesion::Pausada),
    ];

    for (valor_de_cable, esperado) in casos {
        sidecar
            .enviar_estado_sesion(
                valor_de_cable,
                "device_removed jid=5511999999999@s.whatsapp.net numero telefono dispositivo",
                63,
                1_735_689_600_000,
            )
            .await;
        // Da tiempo a la tarea de fondo a procesar el estado antes de leerlo.
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(adaptador.estado_actual(), esperado);

        let proyectado = format!("{:?}", adaptador.estado_actual()).to_lowercase();
        assert!(
            sin_terminos_proscritos(&proyectado),
            "el estado de sesión proyectado al núcleo no debe llevar la causa cruda del cable: \
             {proyectado}"
        );
    }
}
