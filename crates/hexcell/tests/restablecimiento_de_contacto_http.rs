//! Pruebas de la ruta `POST /admin/contacto/restablecer` (HEX-091-a).
//!
//! Cubren el enrutado, la validación del cuerpo (400 sin emitir ninguna orden), el valor por
//! omisión de `incluir_baja`, el volcado JSON de cada desenlace con el discriminante `existe`
//! explícito, la traducción del acuse crudo y la composición por HTTP real en proceso. Cada
//! cuerpo JSON se compara entero, nunca con `contains`.

mod comun;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use comun::{DirectorioTemporal, abrir_persistencia, peticion_http_post_cruda};
use hexcell::admin::{
    AcuseDeRestablecimientoCrudo, DesenlaceDeRestablecimiento, EstadoDeAdmin, OperacionesDeSesion,
    PlazosDeSesion, RegistroDeSesion, RutaAdmin, SesionDeCanal, SolicitudDeRestablecimiento,
    atender_restablecimiento_de_contacto, enrutar_admin, es_contacto_valido, servir_admin,
    traducir_acuse_de_restablecimiento,
};
use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell_core::canal::EstadoSesion;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};

const CONTACTO: &str = "ct-0123456789abcdef0123456789abcdef";

/// Sesión espía: registra cada solicitud recibida y responde el desenlace fijado.
fn sesion_espia(
    desenlace: DesenlaceDeRestablecimiento,
) -> (
    SesionDeCanal,
    Arc<Mutex<Vec<SolicitudDeRestablecimiento>>>,
    Arc<AtomicUsize>,
) {
    let recibidas = Arc::new(Mutex::new(Vec::new()));
    let llamadas = Arc::new(AtomicUsize::new(0));
    let r = Arc::clone(&recibidas);
    let l = Arc::clone(&llamadas);
    let operaciones = OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        pausar_envio: Box::new(|_| {
            Box::pin(async move { hexcell::admin::DesenlaceDePausa::Aplicado })
        }),
        emparejar: Box::new(|_, _| {
            Box::pin(async move {
                hexcell::admin::DesenlaceDeEmparejamiento::Fallido {
                    motivo: String::new(),
                }
            })
        }),
        estado: Box::new(|| Box::pin(async move { EstadoSesion::Activa })),
        restablecer_contacto: Box::new(move |solicitud, _plazo| {
            l.fetch_add(1, Ordering::SeqCst);
            r.lock().unwrap().push(solicitud);
            let d = desenlace.clone();
            Box::pin(async move { d })
        }),
    };
    (SesionDeCanal::ConSesion(operaciones), recibidas, llamadas)
}

fn aplicado_en_cero(incluir_baja: bool) -> DesenlaceDeRestablecimiento {
    DesenlaceDeRestablecimiento::Aplicado {
        contacto: CONTACTO.to_string(),
        incluir_baja,
        cortacircuitos: 0,
        presentacion_de_conversacion: 0,
        baja_de_contacto: 0,
    }
}

/// Levanta el listener administrativo real, en proceso, con la sesión dada registrada.
async fn admin_en_proceso(
    sesion: Option<SesionDeCanal>,
) -> (
    String,
    DirectorioTemporal,
    impl std::future::Future<Output = ()>,
) {
    let directorio = DirectorioTemporal::nuevo("restablecimiento-http");
    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let estado = Arc::new(EstadoDeAdmin::nuevo());
    let proveedor = ProveedorDeEmbeddingsDeCelula::Simulado(ProveedorDeEmbeddingsSimulado::nuevo());
    let servicio = Arc::new(ServicioDeEmbeddings::nuevo(proveedor, repositorio));
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    if let Some(sesion) = sesion {
        let _ = sesion.registrar(&registro);
    }
    let (direccion, futuro) = servir_admin(
        "127.0.0.1:0".parse().expect("dirección local válida"),
        1024 * 1024,
        estado,
        servicio,
        directorio.ruta().to_path_buf(),
        || false,
        registro,
        PlazosDeSesion::por_omision(),
    )
    .await
    .expect("vincular el listener administrativo del test");
    (direccion.to_string(), directorio, futuro)
}

/// POST crudo descargado al pool de bloqueo (el runtime del test es `current_thread`); devuelve
/// el código de estado y el cuerpo JSON ya parseado.
async fn post(direccion: &str, cuerpo: &str) -> (u16, Value) {
    let d = direccion.to_string();
    let c = cuerpo.to_string();
    let cruda = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/contacto/restablecer", &c)
    })
    .await
    .expect("la petición no debe entrar en pánico");
    let estado: u16 = cruda
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("respuesta sin código de estado: {cruda}"));
    let cuerpo_json = cruda
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_else(|| panic!("respuesta sin cuerpo: {cruda}"));
    (
        estado,
        serde_json::from_str(cuerpo_json.trim())
            .unwrap_or_else(|e| panic!("cuerpo no JSON ({e}): {cuerpo_json}")),
    )
}

// ---------------------------------------------------------------------------
// Enrutado
// ---------------------------------------------------------------------------

#[test]
fn enrutar_admin_mapea_solo_post_a_restablecer_contacto() {
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/contacto/restablecer"),
        RutaAdmin::RestablecerContacto
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/contacto/restablecer"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/contacto/restablecer/otro"),
        RutaAdmin::NoEncontrada
    );
}

// ---------------------------------------------------------------------------
// Validación del cuerpo: valor por omisión y 400 sin operación
// ---------------------------------------------------------------------------

#[tokio::test]
async fn incluir_baja_ausente_llega_como_false_y_true_llega_como_true() {
    let (sesion, recibidas, llamadas) = sesion_espia(aplicado_en_cero(false));
    let (direccion, _dir, futuro) = admin_en_proceso(Some(sesion)).await;
    tokio::spawn(futuro);

    let (estado, _) = post(&direccion, &json!({"contacto": CONTACTO}).to_string()).await;
    assert_eq!(estado, 200);
    let (estado, _) = post(
        &direccion,
        &json!({"contacto": CONTACTO, "incluir_baja": false}).to_string(),
    )
    .await;
    assert_eq!(estado, 200);
    let (estado, _) = post(
        &direccion,
        &json!({"contacto": CONTACTO, "incluir_baja": true}).to_string(),
    )
    .await;
    assert_eq!(estado, 200);

    assert_eq!(llamadas.load(Ordering::SeqCst), 3);
    let recibidas = recibidas.lock().unwrap().clone();
    assert_eq!(
        recibidas,
        vec![
            SolicitudDeRestablecimiento {
                contacto: CONTACTO.to_string(),
                incluir_baja: false
            },
            SolicitudDeRestablecimiento {
                contacto: CONTACTO.to_string(),
                incluir_baja: false
            },
            SolicitudDeRestablecimiento {
                contacto: CONTACTO.to_string(),
                incluir_baja: true
            },
        ]
    );
}

#[tokio::test]
async fn un_cuerpo_invalido_devuelve_400_y_no_emite_ninguna_orden() {
    let (sesion, _recibidas, llamadas) = sesion_espia(aplicado_en_cero(false));
    let (direccion, _dir, futuro) = admin_en_proceso(Some(sesion)).await;
    tokio::spawn(futuro);

    let invalidos: Vec<String> = vec![
        json!({"contacto": CONTACTO, "incluir_baja": "true"}).to_string(),
        json!({"contacto": CONTACTO, "incluir_baja": 1}).to_string(),
        json!({"contacto": CONTACTO, "incluir_baja": null}).to_string(),
        json!({"contacto": CONTACTO, "extra": 1}).to_string(),
        json!({"incluir_baja": true}).to_string(),
        json!({"contacto": null}).to_string(),
        json!({"contacto": ""}).to_string(),
        json!({"contacto": "CT-0123456789abcdef0123456789abcdef"}).to_string(),
        json!({"contacto": "ct-0123456789ABCDEF0123456789ABCDEF"}).to_string(),
        json!({"contacto": "ct-0123456789abcdef0123456789abcde"}).to_string(),
        json!({"contacto": "ct-0123456789abcdef0123456789abcdef0"}).to_string(),
        json!({"contacto": "ct-0123456789abcdef0123456789abcdéé"}).to_string(),
        "no-es-json".to_string(),
        String::new(),
        "[]".to_string(),
    ];
    for cuerpo in &invalidos {
        let (estado, respuesta) = post(&direccion, cuerpo).await;
        assert_eq!(estado, 400, "cuerpo {cuerpo:?} debía ser 400");
        assert_eq!(
            respuesta["resultado"], "fallido",
            "cuerpo {cuerpo:?}: {respuesta}"
        );
        assert!(
            respuesta["motivo"].as_str().is_some_and(|m| !m.is_empty()),
            "un 400 debe llevar motivo: {respuesta}"
        );
    }
    assert_eq!(
        llamadas.load(Ordering::SeqCst),
        0,
        "ningún 400 debe haber emitido una orden"
    );
}

#[tokio::test]
async fn la_composicion_http_devuelve_el_json_entero_del_desenlace() {
    let (sesion, _r, _l) = sesion_espia(DesenlaceDeRestablecimiento::Aplicado {
        contacto: CONTACTO.to_string(),
        incluir_baja: true,
        cortacircuitos: 1,
        presentacion_de_conversacion: 2,
        baja_de_contacto: 3,
    });
    let (direccion, _dir, futuro) = admin_en_proceso(Some(sesion)).await;
    tokio::spawn(futuro);

    let (estado, cuerpo) = post(
        &direccion,
        &json!({"contacto": CONTACTO, "incluir_baja": true}).to_string(),
    )
    .await;
    assert_eq!(estado, 200);
    assert_eq!(
        cuerpo,
        json!({
            "resultado": "aplicado", "contacto": CONTACTO, "existe": true, "incluir_baja": true,
            "cortacircuitos": 1, "presentacion_de_conversacion": 2, "baja_de_contacto": 3
        })
    );
}

#[tokio::test]
async fn sin_operacion_registrada_la_ruta_devuelve_502_fallido() {
    let (direccion, _dir, futuro) = admin_en_proceso(None).await;
    tokio::spawn(futuro);
    let (estado, cuerpo) = post(&direccion, &json!({"contacto": CONTACTO}).to_string()).await;
    assert_eq!(estado, 502);
    assert_eq!(cuerpo["resultado"], "fallido");
}

// ---------------------------------------------------------------------------
// Desenlaces por el servicio puro
// ---------------------------------------------------------------------------

fn solicitud(incluir_baja: bool) -> SolicitudDeRestablecimiento {
    SolicitudDeRestablecimiento {
        contacto: CONTACTO.to_string(),
        incluir_baja,
    }
}

fn registro_con(sesion: SesionDeCanal) -> RegistroDeSesion {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = sesion.registrar(&registro);
    registro
}

#[tokio::test]
async fn registro_sin_poblar_devuelve_502_fallido() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let (estado, cuerpo) =
        atender_restablecimiento_de_contacto(&registro, solicitud(false), Duration::from_secs(1))
            .await;
    assert_eq!(estado, StatusCode::BAD_GATEWAY);
    assert_eq!(cuerpo["resultado"], "fallido");
}

#[tokio::test]
async fn sin_sesion_devuelve_200_canal_sin_sesion() {
    let registro = registro_con(SesionDeCanal::SinSesion);
    let (estado, cuerpo) =
        atender_restablecimiento_de_contacto(&registro, solicitud(false), Duration::from_secs(1))
            .await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(cuerpo, json!({"resultado": "canal_sin_sesion"}));
}

#[tokio::test]
async fn una_operacion_que_nunca_resuelve_devuelve_200_fallido_por_plazo() {
    let operaciones = OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        pausar_envio: Box::new(|_| {
            Box::pin(async move { hexcell::admin::DesenlaceDePausa::Aplicado })
        }),
        emparejar: Box::new(|_, _| {
            Box::pin(async move {
                hexcell::admin::DesenlaceDeEmparejamiento::Fallido {
                    motivo: String::new(),
                }
            })
        }),
        estado: Box::new(|| Box::pin(async move { EstadoSesion::Activa })),
        restablecer_contacto: Box::new(|_, _| {
            Box::pin(async move { std::future::pending::<DesenlaceDeRestablecimiento>().await })
        }),
    };
    let registro = registro_con(SesionDeCanal::ConSesion(operaciones));
    let (estado, cuerpo) = atender_restablecimiento_de_contacto(
        &registro,
        solicitud(false),
        Duration::from_millis(100),
    )
    .await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "fallido");
    assert!(cuerpo["motivo"].as_str().is_some_and(|m| !m.is_empty()));
}

#[tokio::test]
async fn aplicado_con_contadores_en_cero_informa_existe_true() {
    let (sesion, _r, _l) = sesion_espia(aplicado_en_cero(false));
    let registro = registro_con(sesion);
    let (estado, cuerpo) =
        atender_restablecimiento_de_contacto(&registro, solicitud(false), Duration::from_secs(1))
            .await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(
        cuerpo,
        json!({
            "resultado": "aplicado", "contacto": CONTACTO, "existe": true, "incluir_baja": false,
            "cortacircuitos": 0, "presentacion_de_conversacion": 0, "baja_de_contacto": 0
        })
    );
}

#[tokio::test]
async fn contacto_desconocido_informa_existe_false_y_contadores_en_cero() {
    let (sesion, _r, _l) = sesion_espia(DesenlaceDeRestablecimiento::ContactoDesconocido {
        contacto: CONTACTO.to_string(),
        incluir_baja: true,
    });
    let registro = registro_con(sesion);
    let (estado, cuerpo) =
        atender_restablecimiento_de_contacto(&registro, solicitud(true), Duration::from_secs(1))
            .await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(
        cuerpo,
        json!({
            "resultado": "contacto_desconocido", "contacto": CONTACTO, "existe": false,
            "incluir_baja": true, "cortacircuitos": 0, "presentacion_de_conversacion": 0,
            "baja_de_contacto": 0
        })
    );
}

#[tokio::test]
async fn fallido_del_sidecar_se_vuelca_con_su_motivo() {
    let (sesion, _r, _l) = sesion_espia(DesenlaceDeRestablecimiento::Fallido {
        motivo: "almacén de identidad no disponible".to_string(),
    });
    let registro = registro_con(sesion);
    let (estado, cuerpo) =
        atender_restablecimiento_de_contacto(&registro, solicitud(false), Duration::from_secs(1))
            .await;
    assert_eq!(estado, StatusCode::OK);
    assert_eq!(
        cuerpo,
        json!({"resultado": "fallido", "motivo": "almacén de identidad no disponible"})
    );
}

// ---------------------------------------------------------------------------
// Traducción del acuse crudo
// ---------------------------------------------------------------------------

fn crudo(resultado: &str, existe: &str) -> AcuseDeRestablecimientoCrudo {
    AcuseDeRestablecimientoCrudo {
        contacto: CONTACTO.to_string(),
        incluir_baja: "no".to_string(),
        resultado: resultado.to_string(),
        existe: existe.to_string(),
        cortacircuitos: 0,
        presentacion_de_conversacion: 0,
        baja_de_contacto: 0,
        motivo: String::new(),
    }
}

#[test]
fn aplicado_con_existe_si_y_contadores_en_cero_es_aplicado() {
    // La existencia viene del discriminante, nunca de los contadores.
    assert_eq!(
        traducir_acuse_de_restablecimiento(&solicitud(false), &crudo("aplicado", "si")),
        aplicado_en_cero(false)
    );
}

#[test]
fn contacto_desconocido_con_existe_no_es_contacto_desconocido() {
    assert_eq!(
        traducir_acuse_de_restablecimiento(&solicitud(false), &crudo("contacto_desconocido", "no")),
        DesenlaceDeRestablecimiento::ContactoDesconocido {
            contacto: CONTACTO.to_string(),
            incluir_baja: false
        }
    );
}

#[test]
fn los_acuses_incoherentes_son_fallido() {
    let incoherente = DesenlaceDeRestablecimiento::Fallido {
        motivo: "acuse de restablecimiento incoherente".to_string(),
    };
    let mut casos: Vec<(&str, AcuseDeRestablecimientoCrudo)> = vec![
        ("aplicado con existe no", crudo("aplicado", "no")),
        (
            "contacto_desconocido con existe si",
            crudo("contacto_desconocido", "si"),
        ),
        ("existe fuera de si/no", crudo("aplicado", "true")),
        ("existe vacío", crudo("aplicado", "")),
        ("resultado desconocido", crudo("otro", "si")),
    ];
    let mut eco_contacto = crudo("aplicado", "si");
    eco_contacto.contacto = "ct-ffffffffffffffffffffffffffffffff".to_string();
    casos.push(("eco de contacto distinto", eco_contacto));
    let mut eco_baja = crudo("aplicado", "si");
    eco_baja.incluir_baja = "si".to_string();
    casos.push(("eco de incluir_baja distinto", eco_baja));
    let mut negativo = crudo("aplicado", "si");
    negativo.cortacircuitos = -1;
    casos.push(("contador negativo", negativo));
    let mut baja_sin_pedirla = crudo("aplicado", "si");
    baja_sin_pedirla.baja_de_contacto = 1;
    casos.push(("baja tocada sin haberla pedido", baja_sin_pedirla));

    for (nombre, acuse) in casos {
        assert_eq!(
            traducir_acuse_de_restablecimiento(&solicitud(false), &acuse),
            incoherente,
            "caso: {nombre}"
        );
    }
}

#[test]
fn fallido_conserva_el_motivo_del_sidecar() {
    let mut acuse = crudo("fallido", "no");
    acuse.motivo = "contacto inválido".to_string();
    assert_eq!(
        traducir_acuse_de_restablecimiento(&solicitud(false), &acuse),
        DesenlaceDeRestablecimiento::Fallido {
            motivo: "contacto inválido".to_string()
        }
    );
}

// ---------------------------------------------------------------------------
// Validador del identificador
// ---------------------------------------------------------------------------

#[test]
fn es_contacto_valido_acepta_solo_ct_mas_32_hexadecimales_en_minuscula() {
    assert!(es_contacto_valido(CONTACTO));
    let rechazados = [
        "",
        "ct-",
        "0123456789abcdef0123456789abcdef",
        "CT-0123456789abcdef0123456789abcdef",
        "ct-0123456789ABCDEF0123456789ABCDEF",
        "ct-0123456789abcdef0123456789abcde",
        "ct-0123456789abcdef0123456789abcdef0",
        "ct-0123456789abcdef0123456789abcdeg",
        // 35 bytes en total, pero con un carácter multibyte: debe rechazarse sin pánico.
        "ct-0123456789abcdef0123456789abcdé",
        "éct-0123456789abcdef0123456789abcd",
    ];
    for candidato in rechazados {
        assert!(
            !es_contacto_valido(candidato),
            "debía rechazar {candidato:?}"
        );
    }
}
