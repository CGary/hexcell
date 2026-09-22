//! Pruebas HTTP crudas del listener administrativo de la célula (`POST /admin/ingesta`, `GET /admin/ingesta` y `POST /admin/sesion/cierre`).
//!
//! Verifican la activación en segundo plano del flujo de ingesta de conocimiento en sombra,
//! la compuerta de exclusión mutua 409 Conflict, la consulta síncrona de fase, la limitación
//! de cuerpo 413 Payload Too Large por los dos caminos que el servidor distingue (longitud
//! declarada y cuerpo troceado), la independencia de sockets entre salud y administración,
//! el mapeo unitario de desenlaces, y el cierre de sesión del canal (HEX-082-a).

mod comun;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use comun::{
    DirectorioTemporal, lanzar_binario_con_ruta_de_datos, lanzar_binario_con_variables,
    peticion_http_cruda, peticion_http_post_cruda, peticion_http_post_cruda_con_cabeceras,
};
use hexcell::admin::{
    CierreDeSesion, EstadoDeAdmin, FaseDeIngesta, MOTIVO_DE_TERMINACION_ANORMAL,
    RegistroDeCierreDeSesion, RutaAdmin, atender_cierre_de_sesion, enrutar_admin,
    respuesta_de_fase, supervisar_ingesta,
};
use hexcell::configuracion::{Configuracion, ErrorDeConfiguracion, FuenteEnMemoria};
use hexcell::ingesta::{DesenlaceDeIngesta, ResumenDeIngesta};
use hexcell_core::canal::{CicloDeVidaSesion, Emparejamiento, EstadoSesion};
use http_body_util::BodyExt;
use hyper::Method;
use std::sync::Arc;

/// Servidor TCP que acepta conexiones y jamás contesta nada.
///
/// Existe para volver **determinista** la ventana en la que hay una ingesta `EnCurso`. Con el
/// proveedor de embeddings simulado la ingesta completa termina en microsegundos, así que un
/// segundo POST «inmediato» ganaría o perdería la carrera según la máquina, y afirmar el 409
/// sería afirmar la suerte. Apuntando el proveedor real a este pozo, la primera llamada de
/// incrustación —la sonda semántica, que `ejecutar_ingesta` emite antes que ningún lote— queda
/// bloqueada hasta agotar su propio tiempo de espera (8 s por omisión), de modo que la fase no
/// puede abandonar `EnCurso` mientras la prueba corre.
struct PozoDeEmbeddings {
    direccion: SocketAddr,
}

impl PozoDeEmbeddings {
    fn abrir() -> Self {
        let escucha =
            TcpListener::bind("127.0.0.1:0").expect("vincular el pozo de embeddings del test");
        let direccion = escucha
            .local_addr()
            .expect("leer la dirección del pozo de embeddings");

        // Las conexiones aceptadas se retienen vivas a propósito: cerrarlas devolvería un fin de
        // flujo que el cliente interpretaría como error de transporte, y la ingesta terminaría.
        std::thread::spawn(move || {
            let mut retenidas: Vec<TcpStream> = Vec::new();
            while let Ok((flujo, _)) = escucha.accept() {
                retenidas.push(flujo);
            }
        });

        Self { direccion }
    }

    fn url_base(&self) -> String {
        format!("http://{}", self.direccion)
    }
}

/// Extrae el valor del campo `desenlace` del resumen que devuelve el servidor administrativo.
fn desenlace_reportado(cuerpo: &str) -> String {
    let documento: serde_json::Value =
        serde_json::from_str(cuerpo).expect("el cuerpo de la respuesta debe ser JSON válido");
    documento["resumen"]["desenlace"]
        .as_str()
        .unwrap_or_else(|| panic!("la respuesta no reporta ningún desenlace: {cuerpo}"))
        .to_string()
}

#[test]
fn post_valido_responde_202_e_inicia_ingesta_demostrable() {
    let directorio = DirectorioTemporal::nuevo("admin-post-valido");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let cuerpo_json = r#"{
        "referencia_externa": "doc-01",
        "titulo": "Catálogo de Pruebas",
        "contenido": "Este es un contenido de prueba para validar la ingesta administrativa."
    }"#;

    let respuesta_post =
        peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);
    assert!(
        respuesta_post.starts_with("HTTP/1.1 202"),
        "un POST válido debe responder 202 Accepted: {respuesta_post}"
    );
    assert!(
        respuesta_post.contains("en_curso"),
        "la respuesta inicial debe indicar fase en_curso: {respuesta_post}"
    );

    // Afirmar solo el 202 y la fase inicial pasaría en verde aunque nunca se lanzara la tarea de
    // fondo: hay que observar un efecto que solo la ejecución real produce, y el único visible
    // desde fuera del proceso es que la fase abandone `EnCurso`.
    let mut completado = false;
    for _ in 0..50 {
        let respuesta_get = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
        if respuesta_get.contains("finalizada") || respuesta_get.contains("fallida") {
            completado = true;
            assert!(
                respuesta_get.contains("finalizada"),
                "la ingesta en segundo plano debe culminar exitosamente: {respuesta_get}"
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(
        completado,
        "el trabajo de ingesta en segundo plano debe finalizar dentro del tiempo de espera"
    );
}

#[test]
fn segundo_post_mientras_corre_devuelve_409_sobre_http() {
    let directorio = DirectorioTemporal::nuevo("admin-post-409");
    let pozo = PozoDeEmbeddings::abrir();
    let url_base = pozo.url_base();
    let binario = lanzar_binario_con_variables(
        directorio.ruta(),
        &[
            ("HEXCELL_EMBEDDINGS_URL_BASE", &url_base),
            ("HEXCELL_EMBEDDINGS_API_KEY", "clave-de-prueba"),
            ("HEXCELL_EMBEDDINGS_MODELO", "modelo-de-prueba"),
        ],
    );

    let cuerpo_json = r#"{
        "referencia_externa": "doc-largo",
        "titulo": "Documento Extenso",
        "contenido": "Un texto cualquiera: lo que mantiene la ingesta en curso no es su tamaño sino el proveedor de embeddings que nunca contesta."
    }"#;

    let primera = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);
    assert!(
        primera.starts_with("HTTP/1.1 202"),
        "el primer POST sobre una célula recién arrancada debe aceptarse: {primera}"
    );

    let segunda = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);
    assert!(
        segunda.starts_with("HTTP/1.1 409"),
        "un segundo POST con la ingesta en curso debe rechazarse con 409 Conflict sobre HTTP: {segunda}"
    );

    let consulta = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(
        consulta.contains("en_curso"),
        "el rechazo debe convivir con una fase que sigue en curso: {consulta}"
    );
}

#[test]
fn compare_and_set_de_la_fase_solo_admite_un_trabajo() {
    let estado = EstadoDeAdmin::nuevo();
    assert!(estado.intentar_iniciar());
    assert!(!estado.intentar_iniciar());
}

#[test]
fn get_reporta_fases_inactiva_en_curso_y_finalizada() {
    let directorio = DirectorioTemporal::nuevo("admin-get-fases");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let resp_inicial = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(resp_inicial.starts_with("HTTP/1.1 200"));
    assert!(resp_inicial.contains("inactiva"));

    let cuerpo_json = r#"{
        "referencia_externa": "doc-02",
        "titulo": "Documento de Prueba para la Consulta",
        "contenido": "Contenido para la prueba de fases."
    }"#;

    let _ = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);

    let mut alcanzo_terminal = false;
    for _ in 0..50 {
        let respuesta_sondeo = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
        if respuesta_sondeo.contains("finalizada") {
            alcanzo_terminal = true;
            assert!(respuesta_sondeo.contains("desenlace"));
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(alcanzo_terminal, "debe alcanzar la fase finalizada");
}

#[test]
fn post_que_excede_limite_cuerpo_devuelve_413() {
    let directorio = DirectorioTemporal::nuevo("admin-post-413");
    let binario = lanzar_binario_con_variables(
        directorio.ruta(),
        &[("HEXCELL_LIMITE_DE_CUERPO_ADMIN_BYTES", "200")],
    );

    let contenido_grande = "x".repeat(300);
    let cuerpo_json = format!(
        r#"{{"referencia_externa":"doc-grande","titulo":"Gran Doc","contenido":"{contenido_grande}"}}"#
    );

    let resp = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", &cuerpo_json);
    assert!(
        resp.starts_with("HTTP/1.1 413"),
        "un cuerpo mayor al límite debe ser rechazado con 413 Payload Too Large: {resp}"
    );

    let resp_get = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(resp_get.contains("inactiva"));
}

#[test]
fn post_troceado_que_excede_limite_cuerpo_devuelve_413() {
    let directorio = DirectorioTemporal::nuevo("admin-post-413-troceado");
    let binario = lanzar_binario_con_variables(
        directorio.ruta(),
        &[("HEXCELL_LIMITE_DE_CUERPO_ADMIN_BYTES", "200")],
    );

    let contenido_grande = "x".repeat(300);
    let cuerpo_json = format!(
        r#"{{"referencia_externa":"doc-troceado","titulo":"Gran Doc","contenido":"{contenido_grande}"}}"#
    );

    // Sin `Content-Length` el servidor no puede decidir por adelantado y solo le queda acotar el
    // cuerpo mientras lo lee: este es el único camino que ejercita esa segunda mitad de la guarda.
    let cuerpo_troceado = format!("{:x}\r\n{cuerpo_json}\r\n0\r\n\r\n", cuerpo_json.len());

    let resp = peticion_http_post_cruda_con_cabeceras(
        &binario.direccion_admin,
        "/admin/ingesta",
        &cuerpo_troceado,
        &[
            ("Content-Type", "application/json"),
            ("Transfer-Encoding", "chunked"),
        ],
    );
    assert!(
        resp.starts_with("HTTP/1.1 413"),
        "un cuerpo troceado mayor al límite debe ser rechazado con 413 Payload Too Large: {resp}"
    );

    let resp_get = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(resp_get.contains("inactiva"));
}

#[test]
fn puertos_distintos_escuchan_rutas_distintas() {
    let directorio = DirectorioTemporal::nuevo("admin-salud-aislados");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let resp_admin_en_salud = peticion_http_cruda(&binario.direccion, "/admin/ingesta");
    assert!(resp_admin_en_salud.starts_with("HTTP/1.1 404"));

    let resp_salud_en_admin = peticion_http_cruda(&binario.direccion_admin, "/health/live");
    assert!(resp_salud_en_admin.starts_with("HTTP/1.1 404"));

    let resp_salud_ok = peticion_http_cruda(&binario.direccion, "/health/live");
    assert!(resp_salud_ok.starts_with("HTTP/1.1 200"));

    let resp_admin_ok = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(resp_admin_ok.starts_with("HTTP/1.1 200"));
}

#[test]
fn configuracion_direccion_admin_invalida_rechaza_arranque() {
    let directorio_temporal = std::env::temp_dir().join(format!(
        "hexcell-test-config-admin-dir-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directorio_temporal).unwrap();

    let fuente = FuenteEnMemoria::vacia()
        .con("HEXCELL_ID_CELULA", "piloto-01")
        .con("HEXCELL_RUTA_DATOS", directorio_temporal.to_string_lossy())
        .con("HEXCELL_DIRECCION_ADMIN", "socket-invalido");

    let err = Configuracion::desde_fuente(&fuente).expect_err("debe fallar con socket inválido");
    match err {
        ErrorDeConfiguracion::ValorInvalido { nombre, valor, .. } => {
            assert_eq!(nombre, "HEXCELL_DIRECCION_ADMIN");
            assert_eq!(valor, "socket-invalido");
        }
        otro => panic!("se esperaba ValorInvalido, se obtuvo {otro:?}"),
    }

    let _ = std::fs::remove_dir_all(&directorio_temporal);
}

#[tokio::test]
async fn cada_desenlace_se_reporta_con_una_fase_propia_y_distinta() {
    let estado = EstadoDeAdmin::nuevo();

    let esperados = [
        (DesenlaceDeIngesta::Completa, "completa"),
        (DesenlaceDeIngesta::Parcial, "parcial"),
        (
            DesenlaceDeIngesta::DetenidaPorApagado,
            "detenida_por_apagado",
        ),
        (DesenlaceDeIngesta::SinIncrustaciones, "sin_incrustaciones"),
    ];

    let mut reportados = Vec::new();
    for (desenlace, esperado) in esperados {
        let resumen = ResumenDeIngesta {
            fragmentos_solicitados: 10,
            fragmentos_escritos: 10,
            lotes_emitidos: 1,
            dimension_observada: Some(4),
            dimension_de_la_sonda: Some(4),
            desenlace,
        };

        estado.registrar_desenlace(Ok(resumen.clone()));
        let fase = estado.fase_actual();
        assert_eq!(
            fase,
            FaseDeIngesta::Finalizada {
                resumen: resumen.clone()
            }
        );

        let respuesta = respuesta_de_fase(&fase);
        assert_eq!(respuesta.status(), hyper::StatusCode::OK);

        let bytes = respuesta
            .into_body()
            .collect()
            .await
            .expect("el cuerpo de la respuesta administrativa siempre está completo en memoria")
            .to_bytes();
        let cuerpo = String::from_utf8(bytes.to_vec()).expect("el cuerpo debe ser UTF-8");
        let reportado = desenlace_reportado(&cuerpo);

        assert_eq!(
            reportado, esperado,
            "el desenlace {desenlace:?} debe reportarse como «{esperado}»"
        );
        reportados.push(reportado);
    }

    // Comprobar solo los valores esperados uno a uno dejaría pasar una tabla que los repitiera si
    // alguien cambiara a la vez la expectativa: la propiedad que el criterio pide es que ningún
    // par de desenlaces comparta fase reportada, y eso hay que afirmarlo sobre el conjunto.
    for (i, uno) in reportados.iter().enumerate() {
        for otro in reportados.iter().skip(i + 1) {
            assert_ne!(
                uno, otro,
                "dos desenlaces distintos no pueden reportar la misma fase: {reportados:?}"
            );
        }
    }

    estado.registrar_desenlace(Err("error de prueba".to_string()));
    assert_eq!(
        estado.fase_actual(),
        FaseDeIngesta::Fallida {
            motivo: "error de prueba".to_string()
        }
    );
}

#[test]
fn unidades_enrutar_admin_puro() {
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/ingesta"),
        RutaAdmin::DispararIngesta
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/ingesta"),
        RutaAdmin::ConsultarEstado
    );
    assert_eq!(
        enrutar_admin(&Method::PUT, "/admin/ingesta"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/desconocido"),
        RutaAdmin::NoEncontrada
    );
}

/// El vigilante convierte la muerte anormal de la tarea en una fase terminal, no en un cierre eterno.
///
/// Ejercita el mecanismo de supervisión, no un pánico real dentro de `ejecutar_ingesta`: se le
/// entrega un `JoinHandle` de una tarea que entra en pánico, que es exactamente lo que `tokio`
/// entregaría si la ingesta reventara. Lo que se afirma es la consecuencia que le importa al
/// operador: la fase deja de ser `EnCurso` y el siguiente POST vuelve a ser admitido en lugar de
/// chocar para siempre contra el 409.
/// El perfil de release fija `panic = "abort"`, donde un pánico mata el proceso y esta rama no se
/// alcanza; la prueba corre bajo `panic = "unwind"`, que es el perfil en que la guarda existe.
#[tokio::test]
async fn tarea_en_panico_deja_fase_terminal_y_readmite_un_nuevo_trabajo() {
    let estado = std::sync::Arc::new(EstadoDeAdmin::nuevo());
    assert!(estado.intentar_iniciar(), "el primer trabajo debe arrancar");
    assert_eq!(estado.fase_actual(), FaseDeIngesta::EnCurso);

    let tarea = tokio::task::spawn(async { panic!("pánico simulado dentro de la ingesta") });
    supervisar_ingesta(std::sync::Arc::clone(&estado), tarea).await;

    let fase = estado.fase_actual();
    match &fase {
        FaseDeIngesta::Fallida { motivo } => assert!(
            motivo.contains(MOTIVO_DE_TERMINACION_ANORMAL),
            "el motivo debe nombrar la terminación anormal, no parecer un error de ingesta: {motivo}"
        ),
        otra => panic!("la fase debe ser terminal tras un pánico, y fue {otra:?}"),
    }

    assert!(
        estado.intentar_iniciar(),
        "un POST posterior debe ser admitido, no responder 409 para siempre"
    );
}

// ---------------------------------------------------------------------------
// Pruebas de cierre de sesión (HEX-082-a)
// ---------------------------------------------------------------------------

/// Error de prueba para el doble de `CicloDeVidaSesion`.
#[derive(Debug)]
struct ErrorDePrueba(String);

impl std::fmt::Display for ErrorDePrueba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ErrorDePrueba {}

/// Doble de `CicloDeVidaSesion` para las pruebas de cierre de sesión.
///
/// Permite inyectar el resultado de `cerrar_sesion` y el estado de sesión, sin depender de
/// ningún adaptador real.
struct DobleCicloDeVida {
    resultado: Arc<tokio::sync::Mutex<Option<Result<(), ErrorDePrueba>>>>,
    estado: EstadoSesion,
}

impl DobleCicloDeVida {
    fn nuevo(resultado: Result<(), String>) -> Self {
        Self {
            resultado: Arc::new(tokio::sync::Mutex::new(Some(
                resultado.map_err(ErrorDePrueba),
            ))),
            estado: EstadoSesion::Activa,
        }
    }

    fn que_nunca_responde() -> Self {
        Self {
            resultado: Arc::new(tokio::sync::Mutex::new(None)),
            estado: EstadoSesion::Activa,
        }
    }
}

impl CicloDeVidaSesion for DobleCicloDeVida {
    type Error = ErrorDePrueba;

    async fn iniciar_emparejamiento(&self) -> Result<Emparejamiento, Self::Error> {
        Err(ErrorDePrueba("no implementado en el doble".to_string()))
    }

    async fn cerrar_sesion(&self) -> Result<(), Self::Error> {
        let resultado = {
            let mut guard = self.resultado.lock().await;
            guard.take()
        };
        match resultado {
            Some(resultado) => resultado,
            None => {
                // Nunca resuelve: se queda colgado para siempre.
                std::future::pending::<Result<(), ErrorDePrueba>>().await
            }
        }
    }

    fn estado_sesion(&self) -> EstadoSesion {
        self.estado
    }
}

#[test]
fn enrutar_admin_post_sesion_cierre_es_cerrar_sesion() {
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/sesion/cierre"),
        RutaAdmin::CerrarSesion
    );
    // GET sobre la misma ruta no existe: solo POST.
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/sesion/cierre"),
        RutaAdmin::NoEncontrada
    );
    // PUT tampoco.
    assert_eq!(
        enrutar_admin(&Method::PUT, "/admin/sesion/cierre"),
        RutaAdmin::NoEncontrada
    );
    // Las rutas existentes siguen mapeando igual.
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/ingesta"),
        RutaAdmin::DispararIngesta
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/ingesta"),
        RutaAdmin::ConsultarEstado
    );
}

#[tokio::test]
async fn cierre_de_sesion_sin_sesion_devuelve_200_con_motivo() {
    let registro: RegistroDeCierreDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = CierreDeSesion::SinSesion.registrar(&registro);

    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "completado");
    // Literal fijado a propósito (no importa la constante de producción): esta guarda debe
    // ponerse roja si alguien cambia el valor de MOTIVO_CANAL_SIN_SESION, no seguir verde
    // porque ambos lados se movieron juntos.
    assert_eq!(cuerpo["motivo"], "canal_sin_sesion");
}

#[tokio::test]
async fn cierre_de_sesion_con_sesion_ok_devuelve_200_sin_motivo() {
    let registro: RegistroDeCierreDeSesion = Arc::new(std::sync::OnceLock::new());
    let doble = DobleCicloDeVida::nuevo(Ok(()));
    let _ = CierreDeSesion::con_sesion(doble).registrar(&registro);

    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "completado");
    // El campo `motivo` NO debe estar presente en el 200 de ConSesion-Ok.
    assert!(
        cuerpo.get("motivo").is_none(),
        "el 200 de ConSesion-Ok no debe llevar campo motivo: {cuerpo}"
    );
}

#[tokio::test]
async fn cierre_de_sesion_con_sesion_error_devuelve_502_con_motivo_real() {
    let registro: RegistroDeCierreDeSesion = Arc::new(std::sync::OnceLock::new());
    // Motivo distintivo que no aparece en ningún otro lugar del código de producción.
    let motivo_fixture = "error-de-prueba-distintivo-abc123xyz";
    let doble = DobleCicloDeVida::nuevo(Err(motivo_fixture.to_string()));
    let _ = CierreDeSesion::con_sesion(doble).registrar(&registro);

    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::BAD_GATEWAY);
    assert_eq!(cuerpo["resultado"], "fallido");
    assert_eq!(cuerpo["motivo"], motivo_fixture);
}

#[tokio::test]
async fn cierre_de_sesion_con_sesion_que_nunca_responde_devuelve_504() {
    let registro: RegistroDeCierreDeSesion = Arc::new(std::sync::OnceLock::new());
    let doble = DobleCicloDeVida::que_nunca_responde();
    let _ = CierreDeSesion::con_sesion(doble).registrar(&registro);

    // Plazo corto inyectado por el test, nunca la constante de producción.
    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_millis(50)).await;
    assert_eq!(estado, hyper::StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(cuerpo["resultado"], "ausente");
}

#[tokio::test]
async fn cierre_de_sesion_sin_registro_devuelve_502() {
    let registro: RegistroDeCierreDeSesion = Arc::new(std::sync::OnceLock::new());
    // No se registra nada: el OnceLock queda vacío.

    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::BAD_GATEWAY);
    assert_eq!(cuerpo["resultado"], "fallido");
}

#[test]
fn cierre_de_sesion_canal_simulado_responde_200_con_motivo() {
    // Prueba end-to-end sobre el binario real con el canal simulado.
    let directorio = DirectorioTemporal::nuevo("admin-cierre-simulado");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_post_cruda(&binario.direccion_admin, "/admin/sesion/cierre", "");
    assert!(
        respuesta.starts_with("HTTP/1.1 200"),
        "el canal simulado debe responder 200 al cierre de sesión: {respuesta}"
    );
    assert!(
        respuesta.contains("canal_sin_sesion"),
        "la respuesta debe llevar el motivo canal_sin_sesion: {respuesta}"
    );
}
