//! Pruebas HTTP de las rutas administrativas de marcas de época sospechosa
//! (`GET /admin/epocas/sospechosas` y `POST /admin/epocas/sospechosas/archivar`, HEX-093).
//!
//! Ejercitan el contrato HTTP congelado para el hijo "b" (la CLI de `hexcell-admin` consumirá
//! exactamente este cable): listado tolerante que nunca convierte una marca mala en 500, archivo
//! con renombrado más certificación (la marca nunca se borra), idempotencia, 404 explícito y 400
//! sin tocar archivos. Las marcas se escriben con la API pública de la capa de persistencia, o con
//! `std::fs::write` para las fixtures corruptas.

mod comun;

use std::path::Path;

use comun::{
    DirectorioTemporal, lanzar_binario_con_ruta_de_datos, peticion_http_cruda,
    peticion_http_post_cruda,
};
use hexcell::admin::{RutaAdmin, enrutar_admin};
use hexcell_storage::retencion::escribir_marca_de_epoca_sospechosa;
use hyper::Method;

/// Escribe una marca activa con la API pública de la capa de persistencia.
fn escribir_marca(ruta_datos: &Path, numero: i64, motivo: &str) {
    escribir_marca_de_epoca_sospechosa(ruta_datos, numero, motivo, "2026-09-30")
        .expect("escribir marca de época sospechosa");
}

/// Nombre del archivo de marca archivada del número dado.
fn nombre_de_marca_archivada(numero: i64) -> String {
    format!("knowledge_epoch_{numero}.sospechosa.archivada")
}

/// Nombre del archivo de marca activa del número dado.
fn nombre_de_marca_activa(numero: i64) -> String {
    format!("knowledge_epoch_{numero}.sospechosa")
}

/// Interpreta el cuerpo JSON de una respuesta HTTP cruda.
fn cuerpo_json(respuesta: &str) -> serde_json::Value {
    let cuerpo = respuesta
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_else(|| panic!("la respuesta no lleva cuerpo: {respuesta}"));
    serde_json::from_str(cuerpo).unwrap_or_else(|_| panic!("el cuerpo debe ser JSON: {cuerpo}"))
}

#[test]
fn get_sin_marcas_devuelve_lista_vacia() {
    let directorio = DirectorioTemporal::nuevo("epocas-get-vacio");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_cruda(&binario.direccion_admin, "/admin/epocas/sospechosas");
    assert!(
        respuesta.starts_with("HTTP/1.1 200"),
        "un directorio sin marcas responde 200: {respuesta}"
    );
    let cuerpo = cuerpo_json(&respuesta);
    assert_eq!(
        cuerpo["marcas"],
        serde_json::json!([]),
        "sin marcas el listado es un arreglo vacío: {cuerpo}"
    );
}

#[test]
fn get_lista_marca_vigente_con_certificacion_nula() {
    let directorio = DirectorioTemporal::nuevo("epocas-get-vigente");
    escribir_marca(directorio.ruta(), 7, "defecto de prueba");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_cruda(&binario.direccion_admin, "/admin/epocas/sospechosas");
    assert!(respuesta.starts_with("HTTP/1.1 200"), "{respuesta}");
    let cuerpo = cuerpo_json(&respuesta);
    let marcas = cuerpo["marcas"]
        .as_array()
        .unwrap_or_else(|| panic!("marcas debe ser un arreglo: {cuerpo}"));
    assert_eq!(marcas.len(), 1);
    let entrada = &marcas[0];
    assert_eq!(entrada["numero_de_epoca"], 7);
    assert_eq!(entrada["motivo"], "defecto de prueba");
    assert_eq!(entrada["fecha_absoluta"], "2026-09-30");
    assert_eq!(entrada["estado"], "vigente");
    assert!(
        entrada["certificacion"].is_null(),
        "una marca vigente no lleva certificación"
    );
}

#[test]
fn get_con_marca_ilegible_responde_200_con_entrada_ilegible() {
    let directorio = DirectorioTemporal::nuevo("epocas-get-ilegible");
    escribir_marca(directorio.ruta(), 2, "sana");
    // Marca con número discrepante: nombre 1, contenido 99.
    std::fs::write(
        directorio.ruta().join("knowledge_epoch_1.sospechosa"),
        "numero_de_epoca: 99\nmotivo: corrupto\nfecha_absoluta: 2026-09-30\n",
    )
    .expect("escribir marca discrepante");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_cruda(&binario.direccion_admin, "/admin/epocas/sospechosas");
    assert!(
        respuesta.starts_with("HTTP/1.1 200"),
        "una marca ilegible nunca convierte el listado en 500: {respuesta}"
    );
    let cuerpo = cuerpo_json(&respuesta);
    let marcas = cuerpo["marcas"]
        .as_array()
        .unwrap_or_else(|| panic!("marcas debe ser un arreglo: {cuerpo}"));
    assert_eq!(
        marcas.len(),
        2,
        "la marca sana y la ilegible se listan ambas"
    );

    let sana = marcas
        .iter()
        .find(|e| e["numero_de_epoca"] == 2)
        .expect("la marca sana se lista");
    assert_eq!(sana["estado"], "vigente");

    let ilegible = marcas
        .iter()
        .find(|e| e["estado"] == "ilegible")
        .expect("la marca discrepante aparece como ilegible");
    assert_eq!(ilegible["numero_de_epoca"], 1, "el número viene del nombre");
    assert_eq!(ilegible["error"], "NumeroDeMarcaDiscrepante");
}

#[test]
fn post_archivar_valido_responde_archivada_y_get_la_muestra_archivada() {
    let directorio = DirectorioTemporal::nuevo("epocas-archivar-valido");
    escribir_marca(directorio.ruta(), 3, "defecto de prueba");
    let ruta_activa = directorio.ruta().join(nombre_de_marca_activa(3));
    let ruta_archivada = directorio.ruta().join(nombre_de_marca_archivada(3));
    let original = std::fs::read(&ruta_activa).expect("leer marca activa");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let cuerpo_post = r#"{"numero_de_epoca":3,"certifico":"operador de turno","motivo":"época defectuosa archivada"}"#;
    let respuesta = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/epocas/sospechosas/archivar",
        cuerpo_post,
    );
    assert!(respuesta.starts_with("HTTP/1.1 200"), "{respuesta}");
    let cuerpo = cuerpo_json(&respuesta);
    assert_eq!(cuerpo["resultado"], "archivada");
    assert_eq!(cuerpo["numero_de_epoca"], 3);
    assert_eq!(cuerpo["certificacion"]["certifico"], "operador de turno");
    assert_eq!(
        cuerpo["certificacion"]["motivo"],
        "época defectuosa archivada"
    );
    assert!(
        !cuerpo["certificacion"]["fecha_absoluta"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    // La marca activa quedó renombrada; la archivada conserva el contenido original y anexa la
    // certificación.
    assert!(
        !ruta_activa.exists(),
        "la marca activa debe quedar renombrada"
    );
    assert!(ruta_archivada.exists(), "la marca archivada debe existir");
    let contenido = std::fs::read_to_string(&ruta_archivada).expect("leer marca archivada");
    assert!(contenido.starts_with(std::str::from_utf8(&original).expect("original UTF-8")));
    assert!(contenido.contains("certificacion_certifico: operador de turno"));
    assert!(contenido.contains("certificacion_motivo: época defectuosa archivada"));
    assert!(contenido.contains("certificacion_fecha_absoluta: 20"));

    // Un GET posterior muestra la marca como archivada con su certificación.
    let respuesta_get = peticion_http_cruda(&binario.direccion_admin, "/admin/epocas/sospechosas");
    let cuerpo_get = cuerpo_json(&respuesta_get);
    let marcas = cuerpo_get["marcas"].as_array().expect("marcas arreglo");
    assert_eq!(marcas.len(), 1);
    assert_eq!(marcas[0]["numero_de_epoca"], 3);
    assert_eq!(marcas[0]["estado"], "archivada");
    assert_eq!(marcas[0]["certificacion"]["certifico"], "operador de turno");
    assert_eq!(
        marcas[0]["certificacion"]["motivo"],
        "época defectuosa archivada"
    );
}

#[test]
fn post_archivar_repetido_responde_sin_cambios_y_archivo_identico() {
    let directorio = DirectorioTemporal::nuevo("epocas-archivar-repetido");
    escribir_marca(directorio.ruta(), 3, "defecto de prueba");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let cuerpo_post = r#"{"numero_de_epoca":3,"certifico":"operador","motivo":"archivar"}"#;
    let primera = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/epocas/sospechosas/archivar",
        cuerpo_post,
    );
    assert!(primera.starts_with("HTTP/1.1 200"), "{primera}");
    assert_eq!(cuerpo_json(&primera)["resultado"], "archivada");

    let ruta_archivada = directorio.ruta().join(nombre_de_marca_archivada(3));
    let bytes_tras_el_primer_archivo = std::fs::read(&ruta_archivada).expect("leer archivada");

    let segunda = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/epocas/sospechosas/archivar",
        cuerpo_post,
    );
    assert!(segunda.starts_with("HTTP/1.1 200"), "{segunda}");
    assert_eq!(cuerpo_json(&segunda)["resultado"], "sin_cambios");
    assert_eq!(
        std::fs::read(&ruta_archivada).expect("releer archivada"),
        bytes_tras_el_primer_archivo,
        "el archivo archivado queda byte a byte idéntico tras el segundo POST"
    );
}

#[test]
fn post_archivar_inexistente_responde_404_con_discriminante() {
    let directorio = DirectorioTemporal::nuevo("epocas-archivar-inexistente");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let cuerpo_post = r#"{"numero_de_epoca":9,"certifico":"operador","motivo":"archivar"}"#;
    let respuesta = peticion_http_post_cruda(
        &binario.direccion_admin,
        "/admin/epocas/sospechosas/archivar",
        cuerpo_post,
    );
    assert!(
        respuesta.starts_with("HTTP/1.1 404"),
        "archivar una marca inexistente responde 404: {respuesta}"
    );
    let cuerpo = cuerpo_json(&respuesta);
    assert_eq!(cuerpo["resultado"], "marca_inexistente");
    assert_eq!(cuerpo["numero_de_epoca"], 9);
    assert!(
        !directorio
            .ruta()
            .join(nombre_de_marca_archivada(9))
            .exists(),
        "no se crea ningún archivo al archivar una marca inexistente"
    );
}

#[test]
fn post_archivar_sin_certifico_o_vacio_responde_400_sin_tocar_la_marca() {
    let directorio = DirectorioTemporal::nuevo("epocas-archivar-400");
    escribir_marca(directorio.ruta(), 3, "defecto de prueba");
    let ruta_activa = directorio.ruta().join(nombre_de_marca_activa(3));
    let original = std::fs::read(&ruta_activa).expect("leer marca activa");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let casos_400 = [
        // Sin certifico: debe llegar a la compuerta de la capa de persistencia vía serde default.
        r#"{"numero_de_epoca":3,"motivo":"archivar"}"#,
        // Certifico vacío.
        r#"{"numero_de_epoca":3,"certifico":"","motivo":"archivar"}"#,
        // Motivo vacío.
        r#"{"numero_de_epoca":3,"certifico":"operador","motivo":""}"#,
        // Cuerpo no JSON.
        "no-es-json",
        // Campo desconocido (deny_unknown_fields).
        r#"{"numero_de_epoca":3,"certifico":"a","motivo":"b","extra":1}"#,
    ];
    for caso in casos_400 {
        let respuesta = peticion_http_post_cruda(
            &binario.direccion_admin,
            "/admin/epocas/sospechosas/archivar",
            caso,
        );
        assert!(
            respuesta.starts_with("HTTP/1.1 400"),
            "el cuerpo {caso} debe rechazarse con 400: {respuesta}"
        );
        assert_eq!(
            std::fs::read(&ruta_activa).expect("marca activa intacta"),
            original,
            "la marca activa queda byte a byte idéntica tras el 400 de {caso}"
        );
    }
    assert!(
        !directorio
            .ruta()
            .join(nombre_de_marca_archivada(3))
            .exists(),
        "ningún 400 deja un archivo archivado"
    );
}

#[test]
fn enrutar_admin_reconoce_las_rutas_de_epocas() {
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/epocas/sospechosas"),
        RutaAdmin::ListarMarcasDeEpoca
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/epocas/sospechosas/archivar"),
        RutaAdmin::ArchivarMarcaDeEpoca
    );
    // Literales exactos: ni método cruzado, ni barra final, ni parámetros de ruta.
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/epocas/sospechosas/archivar"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/epocas/sospechosas"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::GET, "/admin/epocas/sospechosas/"),
        RutaAdmin::NoEncontrada
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/epocas/sospechosas/archivar/3"),
        RutaAdmin::NoEncontrada
    );
    // Las rutas existentes siguen mapeando igual.
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/ingesta"),
        RutaAdmin::DispararIngesta
    );
    assert_eq!(
        enrutar_admin(&Method::POST, "/admin/contacto/restablecer"),
        RutaAdmin::RestablecerContacto
    );
}
