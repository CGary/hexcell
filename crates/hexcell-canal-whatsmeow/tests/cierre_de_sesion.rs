//! Pruebas de contrato del cierre de sesión (desvinculación) sobre el doble `SidecarSimulado`.
//!
//! Cubren AC-3 y la mitad comprobable de AC-4: `cerrar_sesion` debe emitir una
//! `orden_cierre_de_sesion` en versión 6 y resolver según el acuse real del sidecar —`Ok(())` en
//! `completado`, `Err` en `fallido`—, nunca el `SinConexion` incondicional del stub anterior.
//! La mitad viva de AC-4 (que `client.Logout` desvincule el dispositivo de verdad y destruya las
//! credenciales) exige un dispositivo emparejado real y queda diferida a la aceptación de A-3.

mod comun;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::CicloDeVidaSesion;
use tokio::time::Duration;

#[tokio::test]
async fn cerrar_sesion_emite_orden_y_resuelve_ok_en_completado() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move { adaptador.cerrar_sesion().await });

    let orden = sidecar.leer_orden_cierre_de_sesion().await;
    assert_eq!(orden.tipo, "orden_cierre_de_sesion");
    assert_eq!(orden.version, 6);

    sidecar
        .enviar_acuse_cierre_de_sesion("completado", "")
        .await;

    tarea
        .await
        .unwrap()
        .expect("el cierre de sesión debe resolverse Ok en completado");
}

#[tokio::test]
async fn cerrar_sesion_devuelve_error_en_fallido() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(6, "celula-1").await;

    let tarea = tokio::spawn(async move { adaptador.cerrar_sesion().await });

    let orden = sidecar.leer_orden_cierre_de_sesion().await;
    assert_eq!(orden.tipo, "orden_cierre_de_sesion");
    assert_eq!(orden.version, 6);

    sidecar
        .enviar_acuse_cierre_de_sesion("fallido", "desvinculación rechazada por el sidecar")
        .await;

    // Nunca el `SinConexion` incondicional del stub: el fallo llega por el acuse real.
    let err = tarea.await.unwrap().expect_err("el cierre debe fallar");
    match err {
        ErrorCanalWhatsmeow::ErrorDeProtocolo(detalle) => {
            assert!(
                detalle.contains("desvinculación rechazada por el sidecar"),
                "el error debe transportar el motivo del sidecar: {detalle}"
            );
        }
        otro => panic!("se esperaba ErrorDeProtocolo, se obtuvo {otro:?}"),
    }
}
