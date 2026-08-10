mod comun;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::conexion::Conexion;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use tokio::time::Duration;

#[tokio::test]
async fn apreton_de_manos_exitoso() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );

    adaptador.arrancar();

    sidecar.aceptar_conexion().await;

    // El adaptador envía su saludo primero
    let saludo_nucleo = sidecar.leer_saludo().await;
    assert_eq!(saludo_nucleo.emisor, "nucleo");
    assert_eq!(saludo_nucleo.id_celula, "celula-1");
    assert_eq!(saludo_nucleo.version, 4);

    // El sidecar responde con su saludo
    sidecar.enviar_saludo(4, "celula-1").await;

    // Verificamos que el apretón de manos se completó enviando un evento
    sidecar
        .enviar_evento("dedup-1", "conv-1", "rem-1", "hola", 0)
        .await;
    let conf = sidecar.leer_confirmacion().await;
    assert_eq!(conf.id_deduplicacion, "dedup-1"); // Nunca un número de secuencia
}

#[tokio::test]
async fn desajuste_de_version_cierra_conexion() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );

    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _saludo = sidecar.leer_saludo().await;

    // El sidecar responde con versión 3 (núcleo espera 4)
    sidecar.enviar_saludo(3, "celula-1").await;

    // La conexión debería ser cerrada por el adaptador
    let linea = sidecar.leer_linea().await;
    assert_eq!(linea, "");
}

/// AC-2: el desajuste de versión no se negocia y el error resultante nombra AMBAS versiones, no
/// solo la propia. Se prueba con `Conexion` directamente (sin pasar por `AdaptadorWhatsmeow`,
/// que descarta el error tras registrarlo) porque es la única forma de inspeccionar el valor.
#[tokio::test]
async fn desajuste_de_version_surge_con_ambas_versiones() {
    let mut sidecar = SidecarSimulado::nuevo();
    let ruta = sidecar.ruta_socket().clone();

    let manejador = tokio::spawn(async move {
        let escritor = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let mut conexion = Conexion::conectar(&ruta, escritor)
            .await
            .expect("debe poder conectar");
        conexion.saludar("celula-1").await
    });

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(3, "celula-1").await;

    let resultado = manejador.await.expect("la tarea no debe entrar en pánico");
    let error = resultado.expect_err("un desajuste de versión debe ser un error");
    let mensaje = error.to_string();

    match error {
        ErrorCanalWhatsmeow::DesajusteDeVersion { propia, remota } => {
            assert_eq!(propia, 4);
            assert_eq!(remota, 3);
        }
        otro => panic!("se esperaba DesajusteDeVersion, se obtuvo {otro:?}"),
    }
    assert!(
        mensaje.contains("propia=4") && mensaje.contains("remota=3"),
        "el error surgido debe mencionar ambas versiones: {mensaje}"
    );
}

#[tokio::test]
async fn error_de_protocolo_tipo_desconocido() {
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
    sidecar.enviar_saludo(4, "celula-1").await;

    sidecar
        .enviar_linea_cruda(r#"{"version":4,"tipo":"desconocido"}"#)
        .await;

    let linea = sidecar.leer_linea().await;
    assert_eq!(linea, "");
}

#[tokio::test]
async fn error_de_protocolo_campo_desconocido() {
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
    sidecar.enviar_saludo(4, "celula-1").await;

    // Regla 3: deny_unknown_fields
    sidecar.enviar_linea_cruda(r#"{"version":4,"tipo":"evento_entrante","id_deduplicacion":"d","id_conversacion":"c","id_remitente":"r","contenido":"x","marca_temporal_ms":0,"campo_extra":1}"#).await;

    let linea = sidecar.leer_linea().await;
    assert_eq!(linea, "");
}

#[tokio::test]
async fn error_de_protocolo_valor_nulo() {
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
    sidecar.enviar_saludo(4, "celula-1").await;

    sidecar.enviar_linea_cruda(r#"{"version":4,"tipo":"evento_entrante","id_deduplicacion":null,"id_conversacion":"c","id_remitente":"r","contenido":"x","marca_temporal_ms":0}"#).await;

    let linea = sidecar.leer_linea().await;
    assert_eq!(linea, "");
}

#[tokio::test]
async fn error_de_protocolo_linea_demasiado_larga() {
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
    sidecar.enviar_saludo(4, "celula-1").await;

    // Supera LIMITE_DE_LINEA (131072 bytes incluyendo el salto de línea final).
    let muy_larga = "a".repeat(131073);
    sidecar.enviar_linea_cruda(&muy_larga).await;

    let linea = sidecar.leer_linea().await;
    assert_eq!(linea, "");
}
