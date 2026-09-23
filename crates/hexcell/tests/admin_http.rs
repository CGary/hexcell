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
    DirectorioTemporal, abrir_persistencia, lanzar_binario_con_ruta_de_datos,
    lanzar_binario_con_variables, peticion_http_cruda, peticion_http_post_cruda,
    peticion_http_post_cruda_con_cabeceras,
};
use hexcell::admin::{
    AccionDePausa, DesenlaceDeEmparejamiento, DesenlaceDePausa, EstadoDeAdmin, FaseDeIngesta,
    MOTIVO_DE_TERMINACION_ANORMAL, MetodoSolicitado, OperacionesDeSesion, PlazosDeSesion,
    RegistroDeSesion, RutaAdmin, SesionDeCanal, atender_cierre_de_sesion,
    atender_consulta_de_sesion, atender_emparejamiento, atender_pausa_de_envio, enrutar_admin,
    respuesta_de_fase, servir_admin, supervisar_ingesta,
};
use hexcell::configuracion::{Configuracion, ErrorDeConfiguracion, FuenteEnMemoria};
use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell::ingesta::{DesenlaceDeIngesta, ResumenDeIngesta};
use hexcell_core::canal::{CicloDeVidaSesion, Emparejamiento, EstadoSesion};
use http_body_util::BodyExt;
use hyper::Method;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
#[derive(Clone)]
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
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = SesionDeCanal::SinSesion.registrar(&registro);

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
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let doble = DobleCicloDeVida::nuevo(Ok(()));
    let _ = SesionDeCanal::con_sesion(doble).registrar(&registro);

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
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    // Motivo distintivo que no aparece en ningún otro lugar del código de producción.
    let motivo_fixture = "error-de-prueba-distintivo-abc123xyz";
    let doble = DobleCicloDeVida::nuevo(Err(motivo_fixture.to_string()));
    let _ = SesionDeCanal::con_sesion(doble).registrar(&registro);

    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::BAD_GATEWAY);
    assert_eq!(cuerpo["resultado"], "fallido");
    assert_eq!(cuerpo["motivo"], motivo_fixture);
}

#[tokio::test]
async fn cierre_de_sesion_con_sesion_que_nunca_responde_devuelve_504() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let doble = DobleCicloDeVida::que_nunca_responde();
    let _ = SesionDeCanal::con_sesion(doble).registrar(&registro);

    // Plazo corto inyectado por el test, nunca la constante de producción.
    let (estado, cuerpo) = atender_cierre_de_sesion(&registro, Duration::from_millis(50)).await;
    assert_eq!(estado, hyper::StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(cuerpo["resultado"], "ausente");
}

#[tokio::test]
async fn cierre_de_sesion_sin_registro_devuelve_502() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
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

// ---------------------------------------------------------------------------
// Pruebas unitarias de las tres rutas de sesión (HEX-085-a)
// ---------------------------------------------------------------------------

/// Construye un `SesionDeCanal::ConSesion` con operaciones espía bajo control del test.
///
/// `pausa_devuelve`, `emparejar_devuelve` y `estado_devuelve` fijan el resultado de cada
/// operación; `contador_de_pausa` y `contador_de_emparejamiento` cuentan las invocaciones (para
/// afirmar que un 400 no invoca la operación). Todas las operaciones devuelven en microsegundos
/// (sin dormir), deterministas para el test.
fn sesion_de_espia(
    pausa_devuelve: DesenlaceDePausa,
    emparejar_devuelve: DesenlaceDeEmparejamiento,
    estado_devuelve: EstadoSesion,
) -> (SesionDeCanal, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let contador_pausa = Arc::new(AtomicUsize::new(0));
    let contador_emparejamiento = Arc::new(AtomicUsize::new(0));

    let cp = Arc::clone(&contador_pausa);
    let ce = Arc::clone(&contador_emparejamiento);
    let pausa = pausa_devuelve;
    let empa = emparejar_devuelve;
    let estado = estado_devuelve;

    let operaciones = OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        pausar_envio: Box::new(move |_accion| {
            cp.fetch_add(1, Ordering::SeqCst);
            let r = pausa.clone();
            Box::pin(async move { r })
        }),
        emparejar: Box::new(move |_metodo, _plazo| {
            ce.fetch_add(1, Ordering::SeqCst);
            let r = empa.clone();
            Box::pin(async move { r })
        }),
        estado: Box::new(move || {
            let e = estado;
            Box::pin(async move { e })
        }),
    };

    (
        SesionDeCanal::ConSesion(operaciones),
        contador_pausa,
        contador_emparejamiento,
    )
}

/// Levanta el servidor administrativo real, en proceso, con la `sesion` dada ya registrada.
///
/// A diferencia de `lanzar_binario_con_ruta_de_datos` (que lanza el binario completo como
/// subproceso y siempre registra `SinSesion` en el canal simulado), esta ayuda deja inyectar un
/// `SesionDeCanal::ConSesion` espía **dentro** del mismo proceso de test: es la única forma de
/// observar, desde fuera de `admin.rs`, que una petición HTTP 400 nunca llegó a invocar la
/// operación real. El futuro devuelto se debe `tokio::spawn`-ear por quien llama; el directorio
/// temporal se debe mantener vivo mientras el servidor esté en pie.
async fn admin_en_proceso_con_sesion(
    sesion: SesionDeCanal,
) -> (
    String,
    DirectorioTemporal,
    impl std::future::Future<Output = ()>,
) {
    let directorio = DirectorioTemporal::nuevo("admin-espia-en-proceso");
    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let estado = Arc::new(EstadoDeAdmin::nuevo());
    let proveedor = ProveedorDeEmbeddingsDeCelula::Simulado(ProveedorDeEmbeddingsSimulado::nuevo());
    let servicio = Arc::new(ServicioDeEmbeddings::nuevo(proveedor, repositorio));

    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = sesion.registrar(&registro);

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
    .expect("vincular el listener administrativo en proceso del test");

    (direccion.to_string(), directorio, futuro)
}

#[tokio::test]
async fn pausa_de_envio_sin_sesion_devuelve_200_canal_sin_sesion() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = SesionDeCanal::SinSesion.registrar(&registro);

    for accion in [AccionDePausa::Pausar, AccionDePausa::Reanudar] {
        let (estado, cuerpo) =
            atender_pausa_de_envio(&registro, accion, Duration::from_secs(1)).await;
        assert_eq!(estado, hyper::StatusCode::OK);
        assert_eq!(cuerpo["resultado"], "canal_sin_sesion");
    }
}

#[tokio::test]
async fn pausa_de_envio_con_sesion_aplicado_devuelve_200_aplicado_con_accion() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let (sesion, contador, _) = sesion_de_espia(
        DesenlaceDePausa::Aplicado,
        DesenlaceDeEmparejamiento::Fallido {
            motivo: String::new(),
        },
        EstadoSesion::Activa,
    );
    let _ = sesion.registrar(&registro);

    let (estado, cuerpo) =
        atender_pausa_de_envio(&registro, AccionDePausa::Pausar, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "aplicado");
    assert_eq!(cuerpo["accion"], "pausar");
    assert_eq!(contador.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pausa_de_envio_con_sesion_fallido_devuelve_200_fallido_con_motivo_real() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let motivo_distintivo = "motivo-distintivo-pausa-7q".to_string();
    let (sesion, contador, _) = sesion_de_espia(
        DesenlaceDePausa::Fallido {
            motivo: motivo_distintivo.clone(),
        },
        DesenlaceDeEmparejamiento::Fallido {
            motivo: String::new(),
        },
        EstadoSesion::Activa,
    );
    let _ = sesion.registrar(&registro);

    let (estado, cuerpo) =
        atender_pausa_de_envio(&registro, AccionDePausa::Pausar, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "fallido");
    assert_eq!(cuerpo["accion"], "pausar");
    assert_eq!(cuerpo["motivo"], motivo_distintivo);
    assert_eq!(contador.load(Ordering::SeqCst), 1);
}

/// D2: el plazo agotado de la pausa de envío responde 200 fallido, nunca 5xx — el 5xx queda
/// reservado a un fallo del propio listener (registro vacío), no a que el sidecar tarde.
#[tokio::test]
async fn pausa_de_envio_que_nunca_resuelve_con_plazo_corto_devuelve_200_fallido() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let sesion = SesionDeCanal::ConSesion(OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        // Pausa que nunca resuelve.
        pausar_envio: Box::new(|_accion| {
            Box::pin(async move { std::future::pending::<DesenlaceDePausa>().await })
        }),
        emparejar: Box::new(|_metodo, _plazo| {
            Box::pin(async move {
                DesenlaceDeEmparejamiento::Fallido {
                    motivo: String::new(),
                }
            })
        }),
        estado: Box::new(|| Box::pin(async move { EstadoSesion::Activa })),
    });
    let _ = sesion.registrar(&registro);

    // Plazo corto de prueba (50 ms), nunca la constante de producción (30 s).
    let inicio = std::time::Instant::now();
    let (estado, cuerpo) =
        atender_pausa_de_envio(&registro, AccionDePausa::Pausar, Duration::from_millis(50)).await;
    let transcurrido = inicio.elapsed();
    assert_eq!(
        estado,
        hyper::StatusCode::OK,
        "un plazo agotado no es un fallo del listener; debe responder 200: {cuerpo}"
    );
    assert_eq!(cuerpo["resultado"], "fallido");
    assert_eq!(cuerpo["accion"], "pausar");
    assert!(!cuerpo["motivo"].as_str().unwrap_or_default().is_empty());
    assert!(
        transcurrido < Duration::from_secs(1),
        "la prueba debe completarse en menos de un segundo, tomó {transcurrido:?}"
    );
}

#[tokio::test]
async fn pausa_de_envio_invalida_o_sin_json_devuelve_400_sin_invocar_operacion() {
    let (sesion, contador, _) = sesion_de_espia(
        DesenlaceDePausa::Aplicado,
        DesenlaceDeEmparejamiento::Fallido {
            motivo: String::new(),
        },
        EstadoSesion::Activa,
    );
    let (direccion, _directorio, futuro) = admin_en_proceso_con_sesion(sesion).await;
    tokio::spawn(futuro);

    // La petición cruda bloquea el hilo del sistema operativo que la ejecuta: en un runtime
    // `current_thread` (el único que este crate habilita) hay que descargarla en el pool de
    // bloqueo para que el servidor espía, corriendo como tarea aparte del mismo runtime, pueda
    // seguir avanzando mientras el test espera la respuesta.
    let d = direccion.clone();
    let resp = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/envio/pausa", r#"{"accion":"detener"}"#)
    })
    .await
    .expect("la petición 400 (acción inválida) no debe entrar en pánico");
    assert!(
        resp.starts_with("HTTP/1.1 400"),
        "acción inválida debe responder 400: {resp}"
    );

    let d = direccion.clone();
    let resp = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/envio/pausa", r#"{"otro":"valor"}"#)
    })
    .await
    .expect("la petición 400 (accion ausente) no debe entrar en pánico");
    assert!(
        resp.starts_with("HTTP/1.1 400"),
        "accion ausente debe responder 400: {resp}"
    );

    let d = direccion.clone();
    let resp = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/envio/pausa", "no-es-json")
    })
    .await
    .expect("la petición 400 (cuerpo no JSON) no debe entrar en pánico");
    assert!(
        resp.starts_with("HTTP/1.1 400"),
        "cuerpo no JSON debe responder 400: {resp}"
    );

    assert_eq!(
        contador.load(Ordering::SeqCst),
        0,
        "ninguno de los tres 400 debe haber invocado la operación de pausa espía"
    );
}

#[tokio::test]
async fn emparejamiento_sin_sesion_devuelve_200_canal_sin_sesion() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = SesionDeCanal::SinSesion.registrar(&registro);

    let (estado, cuerpo) = atender_emparejamiento(
        &registro,
        MetodoSolicitado::CodigoDeVinculacion,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "canal_sin_sesion");
}

#[tokio::test]
async fn emparejamiento_con_codigo_devuelve_200_codigo_con_valores() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let (operaciones, metodo_recibido) = operaciones_espia_codigo();
    let sesion = SesionDeCanal::ConSesion(operaciones);
    let _ = sesion.registrar(&registro);

    let (estado, cuerpo) = atender_emparejamiento(
        &registro,
        MetodoSolicitado::CodigoDeVinculacion,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "codigo");
    assert_eq!(cuerpo["metodo"], "codigo_de_vinculacion");
    assert_eq!(cuerpo["valor"], "ABCD-EFGH");
    assert_eq!(cuerpo["expira_en_ms"], 1234567);
    assert_eq!(
        *metodo_recibido
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        Some(MetodoSolicitado::CodigoDeVinculacion),
        "la operación espía debe recibir exactamente el método pedido en la petición"
    );
}

/// Además de la operación espía, devuelve el método que la operación `emparejar` recibió: la
/// ruta debe pasar el método pedido tal cual, sin traducirlo ni perderlo en el camino.
fn operaciones_espia_codigo() -> (OperacionesDeSesion, Arc<Mutex<Option<MetodoSolicitado>>>) {
    let metodo_recibido = Arc::new(Mutex::new(None));
    let mr = Arc::clone(&metodo_recibido);
    let operaciones = OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        pausar_envio: Box::new(|_| Box::pin(async move { DesenlaceDePausa::Aplicado })),
        emparejar: Box::new(move |metodo, _plazo| {
            *mr.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(metodo);
            Box::pin(async move {
                DesenlaceDeEmparejamiento::Codigo {
                    metodo: "codigo_de_vinculacion".to_string(),
                    valor: "ABCD-EFGH".to_string(),
                    expira_en_ms: 1234567,
                }
            })
        }),
        estado: Box::new(|| Box::pin(async move { EstadoSesion::Activa })),
    };
    (operaciones, metodo_recibido)
}

#[tokio::test]
async fn emparejamiento_con_fallido_sin_conexion_devuelve_200_fallido() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let sesion = SesionDeCanal::ConSesion(OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        pausar_envio: Box::new(|_| Box::pin(async move { DesenlaceDePausa::Aplicado })),
        emparejar: Box::new(|_metodo, _plazo| {
            Box::pin(async move {
                DesenlaceDeEmparejamiento::Fallido {
                    motivo: "sin_conexion".to_string(),
                }
            })
        }),
        estado: Box::new(|| Box::pin(async move { EstadoSesion::Activa })),
    });
    let _ = sesion.registrar(&registro);

    let (estado, cuerpo) =
        atender_emparejamiento(&registro, MetodoSolicitado::Qr, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "fallido");
    assert_eq!(cuerpo["motivo"], "sin_conexion");
}

#[tokio::test]
async fn emparejamiento_invalido_o_sin_json_devuelve_400_sin_invocar_operacion() {
    let (sesion, _, contador) = sesion_de_espia(
        DesenlaceDePausa::Aplicado,
        DesenlaceDeEmparejamiento::Fallido {
            motivo: String::new(),
        },
        EstadoSesion::Activa,
    );
    let (direccion, _directorio, futuro) = admin_en_proceso_con_sesion(sesion).await;
    tokio::spawn(futuro);

    // Igual que en la prueba de pausa: la petición cruda es bloqueante y este runtime es
    // `current_thread`, así que se descarga en el pool de bloqueo.
    let d = direccion.clone();
    let resp = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/sesion/emparejamiento", r#"{"metodo":"sms"}"#)
    })
    .await
    .expect("la petición 400 (método inválido) no debe entrar en pánico");
    assert!(
        resp.starts_with("HTTP/1.1 400"),
        "método inválido debe responder 400: {resp}"
    );

    let d = direccion.clone();
    let resp = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/sesion/emparejamiento", r#"{"otro":"valor"}"#)
    })
    .await
    .expect("la petición 400 (metodo ausente) no debe entrar en pánico");
    assert!(
        resp.starts_with("HTTP/1.1 400"),
        "metodo ausente debe responder 400: {resp}"
    );

    let d = direccion.clone();
    let resp = tokio::task::spawn_blocking(move || {
        peticion_http_post_cruda(&d, "/admin/sesion/emparejamiento", "no-es-json")
    })
    .await
    .expect("la petición 400 (cuerpo no JSON) no debe entrar en pánico");
    assert!(
        resp.starts_with("HTTP/1.1 400"),
        "cuerpo no JSON debe responder 400: {resp}"
    );

    assert_eq!(
        contador.load(Ordering::SeqCst),
        0,
        "ninguno de los tres 400 debe haber invocado la operación de emparejamiento espía"
    );
}

#[tokio::test]
async fn emparejamiento_que_nunca_resuelve_con_plazo_corto_devuelve_200_fallido() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let sesion = SesionDeCanal::ConSesion(OperacionesDeSesion {
        cerrar: Box::new(|| Box::pin(async move { Ok(()) })),
        pausar_envio: Box::new(|_| Box::pin(async move { DesenlaceDePausa::Aplicado })),
        // Emparejamiento que nunca resuelve.
        emparejar: Box::new(|_metodo, _plazo| {
            Box::pin(async move { std::future::pending::<DesenlaceDeEmparejamiento>().await })
        }),
        estado: Box::new(|| Box::pin(async move { EstadoSesion::Activa })),
    });
    let _ = sesion.registrar(&registro);

    // Plazo corto de prueba (50 ms), nunca la constante de producción (30 s).
    let inicio = std::time::Instant::now();
    let (estado, cuerpo) =
        atender_emparejamiento(&registro, MetodoSolicitado::Qr, Duration::from_millis(50)).await;
    let transcurrido = inicio.elapsed();
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["resultado"], "fallido");
    assert!(!cuerpo["motivo"].as_str().unwrap_or_default().is_empty());
    assert!(
        transcurrido < Duration::from_secs(1),
        "la prueba debe completarse en menos de un segundo, tomó {transcurrido:?}"
    );
}

#[tokio::test]
async fn consulta_sesion_sin_sesion_devuelve_200_canal_sin_sesion() {
    let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
    let _ = SesionDeCanal::SinSesion.registrar(&registro);

    let (estado, cuerpo) = atender_consulta_de_sesion(&registro, Duration::from_secs(1)).await;
    assert_eq!(estado, hyper::StatusCode::OK);
    assert_eq!(cuerpo["estado"], "canal_sin_sesion");
}

#[tokio::test]
async fn consulta_sesion_con_sesion_devuelve_cuatro_estados_distintos() {
    for (esperado, estado) in [
        ("activa", EstadoSesion::Activa),
        ("reconectando", EstadoSesion::Reconectando),
        ("desvinculada", EstadoSesion::Desvinculada),
        ("pausada", EstadoSesion::Pausada),
    ] {
        let registro: RegistroDeSesion = Arc::new(std::sync::OnceLock::new());
        let (sesion, _, _) = sesion_de_espia(
            DesenlaceDePausa::Aplicado,
            DesenlaceDeEmparejamiento::Fallido {
                motivo: String::new(),
            },
            estado,
        );
        let _ = sesion.registrar(&registro);

        let (estado_http, cuerpo) =
            atender_consulta_de_sesion(&registro, Duration::from_secs(1)).await;
        assert_eq!(estado_http, hyper::StatusCode::OK);
        assert_eq!(
            cuerpo["estado"], esperado,
            "el estado {estado:?} debe reportarse como «{esperado}»"
        );
    }
}

#[test]
fn enrutar_admin_mapea_tres_nuevas_rutas_y_las_demas_siguen_igual() {
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/envio/pausa"),
        RutaAdmin::PausarEnvio
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/sesion/emparejamiento"),
        RutaAdmin::IniciarEmparejamiento
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/sesion"),
        RutaAdmin::ConsultarSesion
    );
    // Rutas que NO deben mapear:
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/envio/pausa"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/sesion/emparejamiento"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/sesion"),
        RutaAdmin::NoEncontrada
    );
    // Las rutas existentes de ingesta y cierre no cambian.
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/ingesta"),
        RutaAdmin::DispararIngesta
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/ingesta"),
        RutaAdmin::ConsultarEstado
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/sesion/cierre"),
        RutaAdmin::CerrarSesion
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/sesion/cierre"),
        RutaAdmin::NoEncontrada
    );
}

// ---------------------------------------------------------------------------
// Pruebas de integración con el binario real (HEX-085-a)
// ---------------------------------------------------------------------------

#[test]
fn binario_simulado_tres_rutas_sesion_devuelven_canal_sin_sesion() {
    // A través del binario real en el canal simulado: todas las rutas de sesión devuelven
    // canal_sin_sesion, probando que están conectadas en servir_admin y que la rama simulado
    // registra SinSesion.
    let directorio = DirectorioTemporal::nuevo("admin-sesion-simulado");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let resp_pausa = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/envio/pausa",
        r#"{"accion":"pausar"}"#,
    );
    assert!(
        resp_pausa.starts_with("HTTP/1.1 200"),
        "pausa en simulado debe responder 200: {resp_pausa}"
    );
    assert!(
        resp_pausa.contains("canal_sin_sesion"),
        "pausa en simulado debe devolver canal_sin_sesion: {resp_pausa}"
    );

    let resp_emp = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/sesion/emparejamiento",
        r#"{"metodo":"codigo_de_vinculacion"}"#,
    );
    assert!(
        resp_emp.starts_with("HTTP/1.1 200"),
        "emparejamiento en simulado debe responder 200: {resp_emp}"
    );
    assert!(
        resp_emp.contains("canal_sin_sesion"),
        "emparejamiento en simulado debe devolver canal_sin_sesion: {resp_emp}"
    );

    let resp_estado = peticion_http_cruda(&binario.direccion_admin, "/admin/sesion");
    assert!(
        resp_estado.starts_with("HTTP/1.1 200"),
        "consulta en simulado debe responder 200: {resp_estado}"
    );
    assert!(
        resp_estado.contains("canal_sin_sesion"),
        "consulta en simulado debe devolver canal_sin_sesion: {resp_estado}"
    );
}

#[test]
fn binario_whatsmeow_sin_sidecar_pausa_fallida_y_estado_reconectando() {
    // A través del binario real en canal whatsmeow sin sidecar escuchando en el socket: la pausa
    // falla con motivo sin_conexion y el estado de sesión es reconectando. Es la única prueba que
    // cubre el mapeo main.rs ErrorCanalWhatsmeow::SinConexion -> "sin_conexion".
    let dir_socket = DirectorioTemporal::nuevo("admin-whatsmeow-sin-sidecar");
    let ruta_socket = dir_socket.ruta().join("sidecar.sock");
    let dir_datos = DirectorioTemporal::nuevo("admin-whatsmeow-sin-sidecar-datos");

    let mut binario = lanzar_binario_con_variables(
        dir_datos.ruta(),
        &[
            ("HEXCELL_CANAL", "whatsmeow"),
            ("HEXCELL_SOCKET_IPC", &ruta_socket.to_string_lossy()),
        ],
    );

    // Dar tiempo al binario a arrancar e intentar conectar (fallará, pero el estado queda definido).
    std::thread::sleep(Duration::from_millis(500));

    let resp_pausa = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/envio/pausa",
        r#"{"accion":"pausar"}"#,
    );
    assert!(
        resp_pausa.starts_with("HTTP/1.1 200"),
        "pausa sin sidecar debe responder 200: {resp_pausa}"
    );
    assert!(
        resp_pausa.contains("\"resultado\":\"fallido\""),
        "pausa sin sidecar debe ser fallido: {resp_pausa}"
    );
    assert!(
        resp_pausa.contains("\"motivo\":\"sin_conexion\""),
        "pausa sin sidecar debe devolver sin_conexion: {resp_pausa}"
    );

    let resp_estado = peticion_http_cruda(&binario.direccion_admin, "/admin/sesion");
    assert!(
        resp_estado.starts_with("HTTP/1.1 200"),
        "consulta sin sidecar debe responder 200: {resp_estado}"
    );
    assert!(
        resp_estado.contains("\"estado\":\"reconectando\""),
        "consulta sin sidecar debe devolver reconectando: {resp_estado}"
    );

    binario.enviar_sigterm();
    let _ = binario.esperar_salida(Duration::from_secs(5));
}
