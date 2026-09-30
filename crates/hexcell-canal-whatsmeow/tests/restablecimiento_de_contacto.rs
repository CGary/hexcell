//! Pruebas de contrato del restablecimiento de contacto (HEX-091-a) sobre el doble
//! `SidecarSimulado`: el adaptador serializa `incluir_baja` tal cual (`si`/`no`), correlaciona el
//! acuse, y falla cerrado sin conexión, ante un eco incoherente y fuera de plazo. Incluye la guarda
//! de la versión de cable contra el TEXTO del documento del protocolo y contra el sobre emitido.

mod comun;

use comun::SidecarSimulado;
use hexcell_canal_whatsmeow::adaptador::{AdaptadorWhatsmeow, AsaDeSesion};
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::mensajes::{
    AcuseRestablecerContacto, MensajeEntrante, OrdenRestablecerContacto, analizar_mensaje_entrante,
};
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use tokio::time::{Duration, timeout};

const CONTACTO: &str = "ct-0123456789abcdef0123456789abcdef";
const ESPERA: Duration = Duration::from_secs(10);

/// Levanta el adaptador contra un sidecar simulado con el saludo ya completado.
///
/// Devuelve también el receptor de eventos: si se suelta, el adaptador cierra la conexión al
/// intentar entregar un evento entrante.
async fn conectar() -> (
    SidecarSimulado,
    AdaptadorWhatsmeow,
    AsaDeSesion,
    tokio::sync::mpsc::Receiver<hexcell_core::canal::EventoEntrante>,
) {
    let mut sidecar = SidecarSimulado::nuevo();
    let (adaptador, rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    adaptador.arrancar();
    let asa = adaptador.asa_de_sesion("prueba");
    timeout(ESPERA, sidecar.aceptar_conexion())
        .await
        .expect("el adaptador no conectó a tiempo");
    let _ = timeout(ESPERA, sidecar.leer_saludo())
        .await
        .expect("el saludo no llegó a tiempo");
    sidecar.enviar_saludo(7, "celula-1").await;
    (sidecar, adaptador, asa, rx)
}

fn acuse(
    contacto: &str,
    incluir_baja: &str,
    resultado: &str,
    existe: &str,
) -> AcuseRestablecerContacto {
    AcuseRestablecerContacto {
        version: 7,
        tipo: "acuse_restablecer_contacto".to_string(),
        contacto: contacto.to_string(),
        incluir_baja: incluir_baja.to_string(),
        resultado: resultado.to_string(),
        existe: existe.to_string(),
        cortacircuitos: 2,
        presentacion_de_conversacion: 3,
        baja_de_contacto: 0,
        motivo: String::new(),
    }
}

async fn leer_orden_cruda(sidecar: &mut SidecarSimulado) -> String {
    timeout(ESPERA, sidecar.leer_linea())
        .await
        .expect("la orden no llegó a tiempo")
}

async fn enviar_acuse(sidecar: &mut SidecarSimulado, a: &AcuseRestablecerContacto) {
    let linea = serde_json::to_string(a).unwrap();
    sidecar.enviar_linea_cruda(&linea).await;
}

#[tokio::test]
async fn incluir_baja_false_viaja_como_no_y_el_acuse_se_devuelve_campo_a_campo() {
    let (mut sidecar, adaptador, _asa, _rx) = conectar().await;
    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_secs(5))
            .await
    });

    let linea = leer_orden_cruda(&mut sidecar).await;
    let orden: OrdenRestablecerContacto = serde_json::from_str(&linea).unwrap();
    assert_eq!(orden.tipo, "orden_restablecer_contacto");
    assert_eq!(orden.contacto, CONTACTO);
    assert_eq!(orden.incluir_baja, "no");

    let esperado = acuse(CONTACTO, "no", "aplicado", "si");
    enviar_acuse(&mut sidecar, &esperado).await;
    let recibido = tarea.await.unwrap().expect("el acuse debe resolverse Ok");
    assert_eq!(recibido, esperado);
}

#[tokio::test]
async fn incluir_baja_true_viaja_como_si() {
    let (mut sidecar, adaptador, _asa, _rx) = conectar().await;
    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_restablecimiento_de_contacto(CONTACTO, true, Duration::from_secs(5))
            .await
    });

    let linea = leer_orden_cruda(&mut sidecar).await;
    let orden: OrdenRestablecerContacto = serde_json::from_str(&linea).unwrap();
    assert_eq!(orden.incluir_baja, "si");
    assert_eq!(orden.contacto, CONTACTO);

    let mut esperado = acuse(CONTACTO, "si", "aplicado", "si");
    esperado.baja_de_contacto = 1;
    enviar_acuse(&mut sidecar, &esperado).await;
    assert_eq!(tarea.await.unwrap().unwrap(), esperado);
}

#[tokio::test]
async fn el_asa_de_sesion_serializa_igual_y_devuelve_el_contacto_desconocido() {
    let (mut sidecar, _adaptador, asa, _rx) = conectar().await;
    let tarea = tokio::spawn(async move {
        asa.ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_secs(5))
            .await
    });

    let linea = leer_orden_cruda(&mut sidecar).await;
    let orden: OrdenRestablecerContacto = serde_json::from_str(&linea).unwrap();
    assert_eq!(orden.incluir_baja, "no");

    let mut desconocido = acuse(CONTACTO, "no", "contacto_desconocido", "no");
    desconocido.cortacircuitos = 0;
    desconocido.presentacion_de_conversacion = 0;
    enviar_acuse(&mut sidecar, &desconocido).await;
    assert_eq!(tarea.await.unwrap().unwrap(), desconocido);
}

#[tokio::test]
async fn sin_conexion_devuelve_sin_conexion_y_no_escribe() {
    let sidecar = SidecarSimulado::nuevo();
    let (adaptador, _rx) = AdaptadorWhatsmeow::nuevo(
        sidecar.ruta_socket(),
        "celula-1",
        8,
        Retroceso::nuevo(Duration::from_millis(10), 2, Duration::from_millis(10)),
    );
    // Sin `arrancar()`: no hay conexión con el sidecar.
    let err = adaptador
        .ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_millis(200))
        .await
        .expect_err("sin conexión debe fallar");
    assert!(
        matches!(err, ErrorCanalWhatsmeow::SinConexion),
        "se esperaba SinConexion, se obtuvo {err:?}"
    );
}

#[tokio::test]
async fn un_acuse_con_eco_de_contacto_distinto_es_error_de_protocolo() {
    let (mut sidecar, adaptador, _asa, _rx) = conectar().await;
    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_secs(5))
            .await
    });
    let _ = leer_orden_cruda(&mut sidecar).await;

    let ajeno = acuse(
        "ct-ffffffffffffffffffffffffffffffff",
        "no",
        "aplicado",
        "si",
    );
    enviar_acuse(&mut sidecar, &ajeno).await;
    let err = tarea
        .await
        .unwrap()
        .expect_err("un eco distinto debe fallar");
    assert!(
        matches!(err, ErrorCanalWhatsmeow::ErrorDeProtocolo(_)),
        "se esperaba ErrorDeProtocolo, se obtuvo {err:?}"
    );
}

#[tokio::test]
async fn un_acuse_con_eco_de_incluir_baja_distinto_es_error_de_protocolo() {
    let (mut sidecar, adaptador, _asa, _rx) = conectar().await;
    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_secs(5))
            .await
    });
    let _ = leer_orden_cruda(&mut sidecar).await;

    enviar_acuse(&mut sidecar, &acuse(CONTACTO, "si", "aplicado", "si")).await;
    let err = tarea
        .await
        .unwrap()
        .expect_err("un eco distinto debe fallar");
    assert!(matches!(err, ErrorCanalWhatsmeow::ErrorDeProtocolo(_)));
}

#[tokio::test]
async fn un_acuse_huerfano_no_rompe_la_lectura_y_una_llamada_posterior_correlaciona() {
    let (mut sidecar, adaptador, _asa, _rx) = conectar().await;

    // Acuse sin llamada pendiente: se descarta.
    enviar_acuse(&mut sidecar, &acuse(CONTACTO, "no", "aplicado", "si")).await;
    // El bucle de lectura es secuencial: cuando llega la confirmación de un evento enviado
    // después, el acuse huérfano ya se procesó y la conexión sigue viva.
    sidecar
        .enviar_evento("dedup-huerfano", "conv-1", "rem-1", "hola", 1)
        .await;
    let confirmacion = timeout(ESPERA, sidecar.leer_confirmacion())
        .await
        .expect("la confirmación no llegó a tiempo");
    assert_eq!(confirmacion.id_deduplicacion, "dedup-huerfano");

    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_restablecimiento_de_contacto(CONTACTO, true, Duration::from_secs(5))
            .await
    });
    let _ = leer_orden_cruda(&mut sidecar).await;
    let esperado = acuse(CONTACTO, "si", "aplicado", "si");
    enviar_acuse(&mut sidecar, &esperado).await;
    assert_eq!(tarea.await.unwrap().unwrap(), esperado);
}

#[tokio::test]
async fn sin_acuse_dentro_del_plazo_falla_y_libera_la_espera_para_la_siguiente_llamada() {
    let (mut sidecar, adaptador, asa, _rx) = conectar().await;

    let err = adaptador
        .ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_millis(200))
        .await
        .expect_err("sin acuse debe agotar el plazo");
    assert!(matches!(err, ErrorCanalWhatsmeow::ErrorDeProtocolo(_)));
    // El sidecar sí leyó la primera orden, pero nunca la acusó.
    let _ = leer_orden_cruda(&mut sidecar).await;

    let tarea = tokio::spawn(async move {
        asa.ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_secs(5))
            .await
    });
    let _ = leer_orden_cruda(&mut sidecar).await;
    let esperado = acuse(CONTACTO, "no", "aplicado", "si");
    enviar_acuse(&mut sidecar, &esperado).await;
    assert_eq!(tarea.await.unwrap().unwrap(), esperado);
}

// ---------------------------------------------------------------------------
// Serde: el par rechaza campos de más y booleanos en el cable
// ---------------------------------------------------------------------------

const ACUSE_JSON_BASE: &str = r#"{"version":7,"tipo":"acuse_restablecer_contacto","contacto":"ct-0123456789abcdef0123456789abcdef","incluir_baja":"no","resultado":"aplicado","existe":"si","cortacircuitos":1,"presentacion_de_conversacion":2,"baja_de_contacto":0,"motivo":""}"#;

#[test]
fn el_acuse_hace_ida_y_vuelta_por_serde_y_por_el_analizador_entrante() {
    let del_analizador = analizar_mensaje_entrante(ACUSE_JSON_BASE).expect("debe analizarse");
    let MensajeEntrante::AcuseRestablecerContacto(a) = del_analizador else {
        panic!("se esperaba AcuseRestablecerContacto");
    };
    assert_eq!(a, {
        let mut e = acuse(CONTACTO, "no", "aplicado", "si");
        e.cortacircuitos = 1;
        e.presentacion_de_conversacion = 2;
        e
    });
    let reserializado = serde_json::to_string(&a).unwrap();
    assert_eq!(reserializado, ACUSE_JSON_BASE);
}

#[test]
fn el_acuse_rechaza_un_campo_de_mas_y_booleanos_en_incluir_baja_y_existe() {
    let con_extra = ACUSE_JSON_BASE.replace(r#""motivo":"""#, r#""motivo":"","extra":"x""#);
    assert!(serde_json::from_str::<AcuseRestablecerContacto>(&con_extra).is_err());
    assert!(analizar_mensaje_entrante(&con_extra).is_err());

    let incluir_booleano =
        ACUSE_JSON_BASE.replace(r#""incluir_baja":"no""#, r#""incluir_baja":false"#);
    assert!(serde_json::from_str::<AcuseRestablecerContacto>(&incluir_booleano).is_err());
    assert!(analizar_mensaje_entrante(&incluir_booleano).is_err());

    let existe_booleano = ACUSE_JSON_BASE.replace(r#""existe":"si""#, r#""existe":true"#);
    assert!(serde_json::from_str::<AcuseRestablecerContacto>(&existe_booleano).is_err());
    assert!(analizar_mensaje_entrante(&existe_booleano).is_err());

    let orden_con_booleano = r#"{"version":7,"tipo":"orden_restablecer_contacto","contacto":"ct-0123456789abcdef0123456789abcdef","incluir_baja":true}"#;
    assert!(serde_json::from_str::<OrdenRestablecerContacto>(orden_con_booleano).is_err());
}

#[test]
fn la_orden_no_es_un_mensaje_entrante_valido() {
    let orden = r#"{"version":7,"tipo":"orden_restablecer_contacto","contacto":"ct-0123456789abcdef0123456789abcdef","incluir_baja":"no"}"#;
    assert!(analizar_mensaje_entrante(orden).is_err());
}

// ---------------------------------------------------------------------------
// Guarda de la versión de cable: documento y sobre emitido, nunca la constante contra sí misma
// ---------------------------------------------------------------------------

/// Extrae del texto del documento la versión de su cabecera y el número de cable que le asigna la
/// tabla de correspondencia (`| 1.6 | `7` |`).
fn version_de_cable_segun_el_documento(documento: &str) -> (String, i64) {
    const MARCA: &str = "**Versión de este protocolo:** ";
    let inicio = documento
        .find(MARCA)
        .expect("el documento no lleva la cabecera de versión");
    let version_del_documento: String = documento[inicio + MARCA.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let prefijo = format!("| {version_del_documento} | `");
    let fila = documento
        .lines()
        .find(|l| l.starts_with(&prefijo))
        .unwrap_or_else(|| panic!("no hay fila de correspondencia para {version_del_documento}"));
    let cable: String = fila[prefijo.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (
        version_del_documento,
        cable.parse().expect("número de cable ilegible"),
    )
}

#[tokio::test]
async fn la_version_de_cable_del_sobre_emitido_coincide_con_el_documento_y_es_siete() {
    let ruta = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/protocolo-ipc-nucleo-sidecar.md");
    let documento = std::fs::read_to_string(&ruta).expect("no se pudo leer el documento");
    let (version_del_documento, cable_segun_el_documento) =
        version_de_cable_segun_el_documento(&documento);
    assert_eq!(version_del_documento, "1.6");
    assert!(
        documento.contains("| 1.6 | `7` |"),
        "el documento no declara la correspondencia 1.6 -> cable 7"
    );

    let (mut sidecar, adaptador, _asa, _rx) = conectar().await;
    let tarea = tokio::spawn(async move {
        adaptador
            .ordenar_restablecimiento_de_contacto(CONTACTO, false, Duration::from_millis(300))
            .await
    });
    let linea = leer_orden_cruda(&mut sidecar).await;
    let valor: serde_json::Value = serde_json::from_str(&linea).unwrap();
    let cable_en_el_sobre = valor["version"].as_i64().expect("version no es entero");
    let _ = tarea.await;

    assert_eq!(cable_en_el_sobre, cable_segun_el_documento);
    assert_eq!(cable_en_el_sobre, 7);
    assert_eq!(cable_segun_el_documento, 7);
}
