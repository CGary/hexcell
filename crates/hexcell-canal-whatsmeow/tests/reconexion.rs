mod comun;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::EstadoSesion;
use tokio::time::Duration;

#[tokio::test]
async fn reconexion_y_reentrega() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, mut rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );

    adaptador.arrancar();

    // 1. Conexión inicial
    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(5, "celula-1").await;

    // Evento que SÍ se confirma
    sidecar
        .enviar_evento("dedup-1", "c1", "r1", "hola", 0)
        .await;
    let evento1 = rx.recv().await.unwrap();
    assert_eq!(evento1.contenido, "hola");
    let conf = sidecar.leer_confirmacion().await;
    assert_eq!(conf.id_deduplicacion, "dedup-1");

    // Evento que NO se confirma antes de la caída (el sidecar lo cierra abruptamente)
    sidecar
        .enviar_evento("dedup-2", "c1", "r1", "mundo", 0)
        .await;
    let evento2 = rx.recv().await.unwrap();
    assert_eq!(evento2.contenido, "mundo");

    // 2. Caída del sidecar
    sidecar.cerrar();

    // 3. Reconexión
    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(5, "celula-1").await;

    // Evento "dedup-1" NO debe ser reentregado por el sidecar en la vida real porque fue confirmado,
    // pero si probamos la reentrega, enviamos el "dedup-2" de nuevo.
    sidecar
        .enviar_evento("dedup-2", "c1", "r1", "mundo", 0)
        .await;
    let evento2_reentrega = rx.recv().await.unwrap();
    assert_eq!(evento2_reentrega.contenido, "mundo");
    let conf2 = sidecar.leer_confirmacion().await;
    assert_eq!(conf2.id_deduplicacion, "dedup-2");
}

#[tokio::test]
async fn estado_sesion_se_difunde_y_reconecta() {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );

    // El estado inicial es Reconectando
    assert_eq!(adaptador.estado_actual(), EstadoSesion::Reconectando);

    adaptador.arrancar();

    sidecar.aceptar_conexion().await;
    let _ = sidecar.leer_saludo().await;
    sidecar.enviar_saludo(5, "celula-1").await;

    // Espera a que el estado pase a Activa
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(adaptador.estado_actual(), EstadoSesion::Activa);

    // Se cae la conexión
    sidecar.cerrar();

    // El adaptador debe detectar la desconexión y volver a Reconectando
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(adaptador.estado_actual(), EstadoSesion::Reconectando);
}

/// Cobertura del mapeo de las cuatro variantes del cable (`src/adaptador.rs:246-261`), usando
/// el ayudante `SidecarSimulado::enviar_estado_sesion` que hasta ahora no tenía ninguna llamada.
#[tokio::test]
async fn estado_sesion_del_cable_mapea_las_cuatro_variantes() {
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
    sidecar.enviar_saludo(5, "celula-1").await;

    let casos = [
        ("activa", EstadoSesion::Activa),
        ("reconectando", EstadoSesion::Reconectando),
        ("desvinculada", EstadoSesion::Desvinculada),
        ("pausada", EstadoSesion::Pausada),
    ];

    for (valor_de_cable, esperado) in casos {
        sidecar.enviar_estado_sesion(valor_de_cable, "", 0, 0).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            adaptador.estado_actual(),
            esperado,
            "el valor de cable '{valor_de_cable}' debe mapear a {esperado:?}"
        );
    }
}

/// Un valor de `estado` desconocido en el cable es un error de protocolo que cierra la conexión
/// (rama `otro` de `src/adaptador.rs:255-259`), tal como cualquier otro error de la sección 8.
#[tokio::test]
async fn estado_sesion_del_cable_con_valor_desconocido_cierra_la_conexion() {
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
    sidecar.enviar_saludo(5, "celula-1").await;

    sidecar.enviar_estado_sesion("suspendida", "", 0, 0).await;

    // La conexión se cierra: leer más allá devuelve una línea vacía (EOF).
    let linea = sidecar.leer_linea().await;
    assert_eq!(linea, "");
}
