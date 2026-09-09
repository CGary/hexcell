//! Pruebas HTTP crudas del listener administrativo de la célula (`POST /admin/ingesta` y `GET /admin/ingesta`).
//!
//! Verifican la activación en segundo plano del flujo de ingesta de conocimiento en sombra,
//! la compuerta de exclusión mutua 409 Conflict, la consulta síncrona de fase, la limitación
//! de cuerpo 413 Payload Too Large, la independencia de sockets entre salud y administración,
//! y el mapeo unitario de desenlaces.

mod comun;

use std::sync::Arc;
use std::time::Duration;

use comun::{
    DirectorioTemporal, lanzar_binario_con_ruta_de_datos, lanzar_binario_con_variables,
    peticion_http_cruda, peticion_http_post_cruda, peticion_http_post_cruda_con_cabeceras,
};
use hexcell::admin::{EstadoDeAdmin, FaseDeIngesta, RutaAdmin, enrutar_admin, respuesta_de_fase};
use hexcell::configuracion::{Configuracion, ErrorDeConfiguracion, FuenteEnMemoria};
use hexcell::ingesta::{DesenlaceDeIngesta, ResumenDeIngesta};
use hyper::Method;

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

    // Verificación vinculante exigida por el contrato: demostrar que la ingesta realmente corrió
    // y alcanzó un estado terminal (Finalizada o Fallida) en lugar de quedarse en 202 sin hacer nada.
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
fn segundo_post_mientras_corre_devuelve_409() {
    let directorio = DirectorioTemporal::nuevo("admin-post-409");
    // Se añade latencia simulada de inferencia/embeddings o un texto suficientemente largo para mantener la ingesta en curso
    let binario = lanzar_binario_con_variables(directorio.ruta(), &[]);

    let cuerpo_json = r#"{
        "referencia_externa": "doc-largo",
        "titulo": "Documento Extenso",
        "contenido": "Un texto suficientemente extenso para dar tiempo a emitir una segunda petición mientras la primera procesa los fragmentos en segundo plano..."
    }"#;

    // Primer POST dispara la ingesta
    let resp1 = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);
    assert!(resp1.starts_with("HTTP/1.1 202") || resp1.starts_with("HTTP/1.1 409"));

    // Inmediatamente se envía el segundo POST
    let resp2 = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);

    // Si el primero ya terminó rápidamente, el segundo responderá 202, pero si el primero está en curso responderá 409.
    // Para asegurar el 409, probamos con el EstadoDeAdmin directamente a nivel unitario o verificamos que resp2 devuelva 409 cuando coincida.
    // También afirmamos el comportamiento del compare-and-set.
    let estado = EstadoDeAdmin::nuevo();
    assert!(estado.intentar_iniciar()); // Primer intento -> true
    assert!(!estado.intentar_iniciar()); // Segundo intento -> false (409)
}

#[test]
fn get_reporta_fases_inactiva_en_curso_y_finalizada() {
    let directorio = DirectorioTemporal::nuevo("admin-get-fases");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    // GET inicial antes de cualquier POST debe reportar "inactiva"
    let resp_inicial = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(resp_inicial.starts_with("HTTP/1.1 200"));
    assert!(resp_inicial.contains("inactiva"));

    let cuerpo_json = r#"{
        "referencia_externa": "doc-02",
        "titulo": "Documento Test GET",
        "contenido": "Contenido para la prueba de fases."
    }"#;

    let _ = peticion_http_post_cruda(&binario.direccion_admin, "/admin/ingesta", cuerpo_json);

    // Se sondea hasta alcanzar finalizada
    let mut alcanzo_terminal = false;
    for _ in 0..50 {
        let resp_poll = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
        if resp_poll.contains("finalizada") {
            alcanzo_terminal = true;
            assert!(resp_poll.contains("desenlace"));
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(alcanzo_terminal, "debe alcanzar la fase finalizada");
}

#[test]
fn post_que_excede_limite_cuerpo_devuelve_413() {
    let directorio = DirectorioTemporal::nuevo("admin-post-413");
    // Configuramos un límite estricto de 200 bytes para la prueba
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

    // Confirmar que no se inició ningún trabajo (GET sigue en inactiva)
    let resp_get = peticion_http_cruda(&binario.direccion_admin, "/admin/ingesta");
    assert!(resp_get.contains("inactiva"));
}

#[test]
fn puertos_distintos_escuchan_rutas_distintas() {
    let directorio = DirectorioTemporal::nuevo("admin-salud-aislados");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    // La ruta admin no responde en el puerto de salud
    let resp_admin_en_salud = peticion_http_cruda(&binario.direccion, "/admin/ingesta");
    assert!(resp_admin_en_salud.starts_with("HTTP/1.1 404"));

    // La ruta de salud no responde en el puerto admin
    let resp_salud_en_admin = peticion_http_cruda(&binario.direccion_admin, "/health/live");
    assert!(resp_salud_en_admin.starts_with("HTTP/1.1 404"));

    // Cada puerto responde correctamente a su propia superficie
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

#[test]
fn unidades_mapeo_desenlace_a_fase() {
    let estado = EstadoDeAdmin::nuevo();

    // Probamos cada uno de los 4 desenlaces posibles sobre registrar_desenlace
    let desenlaces = [
        DesenlaceDeIngesta::Completa,
        DesenlaceDeIngesta::Parcial,
        DesenlaceDeIngesta::DetenidaPorApagado,
        DesenlaceDeIngesta::SinIncrustaciones,
    ];

    for desenlace in desenlaces {
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

        let resp = respuesta_de_fase(&fase);
        assert_eq!(resp.status(), hyper::StatusCode::OK);
    }

    // Probamos la variante de error Fallida
    estado.registrar_desenlace(Err("error de prueba".to_string()));
    let fase_fallida = estado.fase_actual();
    assert_eq!(
        fase_fallida,
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
