//! Tests HTTP crudos de `/health/live` y `/health/ready` contra el binario real.
//!
//! Se lanza el binario con `HEXCELL_DIRECCION_SALUD=127.0.0.1:0` para que el sistema operativo
//! elija un puerto libre, se lee el puerto real que imprime en `stdout` y se habla HTTP/1.1 a mano
//! sobre un `std::net::TcpStream`: no se añade ningún crate cliente HTTP solo para probar, y
//! ningún test alcanza más red que loopback. Las ayudas viven en `comun/mod.rs`.
//!
//! Desde HEX-006, `/health/ready` ya no devuelve un cuerpo de esqueleto: responde la conjunción de
//! las dos vitalidades de los pools y del estado de sesión del canal.

mod comun;

use comun::{DirectorioTemporal, lanzar_binario_con_ruta_de_datos, peticion_http_cruda};
use hexcell_storage::NOMBRE_DE_ARCHIVO_DE_SESIONES;

#[test]
fn health_live_responde_200_sin_comprobar_nada() {
    let directorio = DirectorioTemporal::nuevo("salud-live");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_cruda(&binario.direccion, "/health/live");
    assert!(respuesta.starts_with("HTTP/1.1 200"));
}

#[test]
fn health_ready_responde_200_con_las_dos_bases_sanas() {
    let directorio = DirectorioTemporal::nuevo("salud-ready");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_cruda(&binario.direccion, "/health/ready");
    assert!(
        respuesta.starts_with("HTTP/1.1 200"),
        "con las dos bases abiertas y migradas la célula debe declararse lista: {respuesta}"
    );
    assert!(respuesta.contains("lista"));
}

#[test]
fn health_live_sigue_en_200_aunque_la_persistencia_se_haya_roto() {
    let directorio = DirectorioTemporal::nuevo("salud-live-degradada");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    // La vivacidad se comprueba antes de romper nada, para saber que el servidor ya responde.
    assert!(peticion_http_cruda(&binario.direccion, "/health/live").starts_with("HTTP/1.1 200"));

    std::fs::remove_file(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("borrar sessions.db del disco");

    // Vivacidad y preparación responden preguntas distintas: atar la primera a la persistencia
    // haría que un supervisor reiniciara en bucle una célula cuyo disco falla temporalmente.
    let vivacidad = peticion_http_cruda(&binario.direccion, "/health/live");
    assert!(
        vivacidad.starts_with("HTTP/1.1 200"),
        "la vivacidad no depende de los pools: {vivacidad}"
    );

    let preparacion = peticion_http_cruda(&binario.direccion, "/health/ready");
    assert!(
        preparacion.starts_with("HTTP/1.1 503"),
        "sin sessions.db la célula no puede atender un mensaje: {preparacion}"
    );
}

#[test]
fn una_ruta_desconocida_responde_404() {
    let directorio = DirectorioTemporal::nuevo("salud-404");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let respuesta = peticion_http_cruda(&binario.direccion, "/no-existe");
    assert!(respuesta.starts_with("HTTP/1.1 404"));
}
