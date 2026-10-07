//! Tests de ejecución de `contacto restablecer` (HEX-091-b) contra el demonio Docker falso.
//!
//! Cada prueba pasa por `ejecutar_con_efectos`, el mismo despacho que usa el binario, y programa
//! la secuencia completa de la sonda hermana con `servir_guiones` (ver `comun`): inspección del
//! núcleo, inspección del núcleo y del sidecar para resolver red y puerto, y creación, arranque,
//! espera, lectura y borrado del contenedor de sonda. El cuerpo que la CLI envía a la ruta del
//! núcleo viaja dentro del `Cmd` del contenedor de sonda (`wget --post-data <cuerpo> <url>`), así
//! que las aserciones sobre el cuerpo leen la petición de creación que el demonio falso RECIBIÓ.

mod comun;

use hexcell_admin::argumentos::analizar;
use hexcell_admin::ciclo_de_vida::DatosDeSondeo;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::ejecutar_con_efectos;
use hexcell_admin::docker::{ClienteDocker, InventarioDocker};
use hexcell_admin::salida::Salida;

use comun::{
    AlmacenTemporal, Guion, PeticionRecibida, ServidorDockerFalso, exigir_silencio, recibir,
    servir_guiones,
};

const CONTACTO: &str = "ct-0123456789abcdef0123456789abcdef";
const RED: &str = "red-de-contacto-k8p2";
const PUERTO_ADMIN: &str = "5191";
const IMAGEN: &str = "sonda-de-contacto:7";

/// Peticiones de una ejecución completa: tres inspecciones y las cinco de la sonda.
const PETICIONES_DE_SONDA_COMPLETA: usize = 8;

fn fuga(texto: String) -> &'static [u8] {
    Box::leak(texto.into_bytes().into_boxed_slice())
}

fn con_cuerpo(estado: u16, razon: &'static str, cuerpo: &'static [u8]) -> Guion {
    Guion::ConCuerpo {
        estado,
        razon,
        cuerpo,
    }
}

fn inspeccion_del_nucleo(estado: &str) -> Guion {
    con_cuerpo(
        200,
        "OK",
        fuga(format!(
            r#"{{"State":{{"Status":"{estado}"}},"NetworkSettings":{{"Networks":{{"{RED}":{{"NetworkID":"n1"}}}}}},"Config":{{"Env":["HEXCELL_DIRECCION_ADMIN=0.0.0.0:{PUERTO_ADMIN}"]}},"Mounts":[{{"Type":"volume","Name":"vol-c","Destination":"/var/lib/hexcell"}}]}}"#
        )),
    )
}

fn logs(cuerpo: &[u8]) -> Guion {
    let mut trama = vec![1u8, 0, 0, 0];
    trama.extend_from_slice(&(cuerpo.len() as u32).to_be_bytes());
    trama.extend_from_slice(cuerpo);
    con_cuerpo(200, "OK", Box::leak(trama.into_boxed_slice()))
}

/// Guiones de una ejecución con núcleo en marcha cuya sonda devuelve `respuesta` por stdout.
fn guiones_de_sonda(respuesta: &[u8], codigo_wget: i64) -> Vec<Guion> {
    vec![
        inspeccion_del_nucleo("running"),
        inspeccion_del_nucleo("running"),
        con_cuerpo(200, "OK", br#"{"State":{"Status":"running"}}"#),
        con_cuerpo(201, "Created", br#"{"Id":"sonda-c1","Warnings":[]}"#),
        Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        },
        con_cuerpo(
            200,
            "OK",
            fuga(format!(r#"{{"StatusCode":{codigo_wget}}}"#)),
        ),
        logs(respuesta),
        Guion::SinCuerpo {
            estado: 204,
            razon: "No Content",
        },
    ]
}

struct Ejecucion {
    codigo: CodigoDeSalida,
    estandar: String,
    diagnostico: String,
    peticiones: Vec<PeticionRecibida>,
    receptor: std::sync::mpsc::Receiver<PeticionRecibida>,
    almacen: AlmacenTemporal,
}

/// Ejecuta `argumentos` por `ejecutar_con_efectos` contra los `guiones` y recoge las
/// `esperadas` peticiones que el demonio falso tuvo que atender.
fn ejecutar(argumentos: &[&str], guiones: Vec<Guion>, esperadas: usize) -> Ejecucion {
    let servidor = ServidorDockerFalso::nuevo("contacto");
    let ruta = servidor.ruta();
    let receptor = servir_guiones(servidor, guiones);
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta, std::time::Duration::from_secs(10));
    let almacen = AlmacenTemporal::nuevo("contacto");
    let argumentos: Vec<String> = argumentos.iter().map(|s| s.to_string()).collect();
    let mut estandar = Vec::new();
    let mut diagnostico = Vec::new();
    let codigo = {
        let mut salida = Salida::nueva(&mut estandar, &mut diagnostico);
        ejecutar_con_efectos(
            analizar(&argumentos),
            &mut salida,
            &cliente,
            &inventario,
            &almacen.texto(),
            1_700_000_000_000,
            DatosDeSondeo {
                imagen: IMAGEN.to_string(),
                limite_segundos: 45,
            },
        )
    };
    let peticiones = (0..esperadas).map(|_| recibir(&receptor)).collect();
    Ejecucion {
        codigo,
        estandar: String::from_utf8(estandar).unwrap(),
        diagnostico: String::from_utf8(diagnostico).unwrap(),
        peticiones,
        receptor,
        almacen,
    }
}

fn restablecer(extra: &[&str]) -> Vec<&'static str> {
    let mut argumentos = vec![
        "contacto",
        "restablecer",
        "--id",
        "celula-c",
        "--contacto",
        CONTACTO,
    ];
    for e in extra {
        argumentos.push(Box::leak(e.to_string().into_boxed_str()));
    }
    argumentos
}

/// Cuerpo JSON y URL que la sonda recibió en su `Cmd`, leídos de la petición de creación.
fn cuerpo_y_url_enviados(peticiones: &[PeticionRecibida]) -> (serde_json::Value, String) {
    let creacion = peticiones
        .iter()
        .find(|p| p.metodo == "POST" && p.objetivo.starts_with("/containers/create"))
        .expect("la sonda debía crearse");
    let valor: serde_json::Value = serde_json::from_slice(&creacion.cuerpo).unwrap();
    assert_eq!(valor["Image"], IMAGEN);
    assert_eq!(valor["HostConfig"]["NetworkMode"], RED);
    let cmd: Vec<String> = valor["Cmd"]
        .as_array()
        .expect("Cmd de la sonda")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let posicion = cmd
        .iter()
        .position(|a| a == "--post-data")
        .expect("la sonda debía hacer POST");
    let cuerpo = serde_json::from_str(&cmd[posicion + 1]).expect("cuerpo JSON");
    (cuerpo, cmd.last().unwrap().clone())
}

const APLICADO_CON_FILAS: &[u8] = br#"{"resultado":"aplicado","contacto":"ct-0123456789abcdef0123456789abcdef","existe":true,"incluir_baja":false,"cortacircuitos":2,"presentacion_de_conversacion":1,"baja_de_contacto":0}"#;
const APLICADO_CON_BAJA: &[u8] = br#"{"resultado":"aplicado","contacto":"ct-0123456789abcdef0123456789abcdef","existe":true,"incluir_baja":true,"cortacircuitos":2,"presentacion_de_conversacion":1,"baja_de_contacto":1}"#;

#[test]
fn sin_incluir_baja_el_cuerpo_recibido_dice_false() {
    let e = ejecutar(
        &restablecer(&[]),
        guiones_de_sonda(APLICADO_CON_FILAS, 0),
        PETICIONES_DE_SONDA_COMPLETA,
    );
    let (cuerpo, url) = cuerpo_y_url_enviados(&e.peticiones);
    assert_eq!(
        cuerpo,
        serde_json::json!({"contacto": CONTACTO, "incluir_baja": false})
    );
    assert_eq!(
        url,
        format!("http://celula-c-nucleo:{PUERTO_ADMIN}/admin/contacto/restablecer")
    );
    assert_eq!(e.codigo, CodigoDeSalida::Exito);
    assert_eq!(
        e.estandar,
        "cortacircuitos: 2\npresentacion_de_conversacion: 1\nbaja_de_contacto: no tocada\n"
    );
    assert!(e.diagnostico.is_empty(), "{:?}", e.diagnostico);
}

#[test]
fn con_incluir_baja_el_cuerpo_recibido_dice_true() {
    let e = ejecutar(
        &restablecer(&["--incluir-baja", "--confirmar"]),
        guiones_de_sonda(APLICADO_CON_BAJA, 0),
        PETICIONES_DE_SONDA_COMPLETA,
    );
    let (cuerpo, _) = cuerpo_y_url_enviados(&e.peticiones);
    assert_eq!(
        cuerpo,
        serde_json::json!({"contacto": CONTACTO, "incluir_baja": true})
    );
    assert_eq!(e.codigo, CodigoDeSalida::Exito);
    assert_eq!(
        e.estandar,
        "cortacircuitos: 2\npresentacion_de_conversacion: 1\nbaja_de_contacto: 1\n"
    );
    assert!(
        e.diagnostico.starts_with("ADVERTENCIA:"),
        "la advertencia de la baja debía ir al diagnóstico: {:?}",
        e.diagnostico
    );
}

#[test]
fn simular_no_emite_ninguna_peticion_ni_crea_contenedor() {
    let e = ejecutar(
        &restablecer(&["--simular", "--incluir-baja"]),
        vec![inspeccion_del_nucleo("running")],
        0,
    );
    assert_eq!(e.codigo, CodigoDeSalida::Exito);
    assert!(
        e.estandar.starts_with("simulación: contacto restablecer")
            && e.estandar.contains("baja_de_contacto: incluida"),
        "{:?}",
        e.estandar
    );
    exigir_silencio(&e.receptor);
    assert!(e.almacen.bytes().is_none(), "no se escribe ningún almacén");
}

#[test]
fn contacto_desconocido_sale_fallo_aunque_los_contadores_sean_cero() {
    let e = ejecutar(
        &restablecer(&[]),
        guiones_de_sonda(
            br#"{"resultado":"contacto_desconocido","contacto":"ct-0123456789abcdef0123456789abcdef","existe":false,"incluir_baja":false,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0}"#,
            0,
        ),
        PETICIONES_DE_SONDA_COMPLETA,
    );
    assert_eq!(e.codigo, CodigoDeSalida::Fallo);
    assert!(
        e.estandar.is_empty(),
        "sin línea de éxito: {:?}",
        e.estandar
    );
    assert!(e.diagnostico.contains("contacto_desconocido"));
    assert!(!e.diagnostico.contains("sin cambios"));
}

#[test]
fn aplicado_con_existe_false_tambien_sale_fallo() {
    let e = ejecutar(
        &restablecer(&[]),
        guiones_de_sonda(
            br#"{"resultado":"aplicado","existe":false,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0}"#,
            0,
        ),
        PETICIONES_DE_SONDA_COMPLETA,
    );
    assert_eq!(e.codigo, CodigoDeSalida::Fallo);
    assert!(e.estandar.is_empty(), "{:?}", e.estandar);
}

#[test]
fn contacto_existente_sin_filas_sale_exito_con_la_linea_sin_cambios() {
    let e = ejecutar(
        &restablecer(&[]),
        guiones_de_sonda(
            br#"{"resultado":"aplicado","contacto":"ct-0123456789abcdef0123456789abcdef","existe":true,"incluir_baja":false,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0}"#,
            0,
        ),
        PETICIONES_DE_SONDA_COMPLETA,
    );
    assert_eq!(e.codigo, CodigoDeSalida::Exito);
    assert_eq!(
        e.estandar,
        "cortacircuitos: 0\npresentacion_de_conversacion: 0\nbaja_de_contacto: no tocada\n"
    );
    assert_eq!(
        e.diagnostico,
        "sin cambios: el contacto no tenía filas que borrar\n"
    );
}

#[test]
fn incluir_baja_sin_confirmar_no_emite_peticion_alguna() {
    let e = ejecutar(
        &restablecer(&["--incluir-baja"]),
        vec![inspeccion_del_nucleo("running")],
        0,
    );
    assert_eq!(e.codigo, CodigoDeSalida::UsoIncorrecto);
    assert!(e.diagnostico.contains("--confirmar"), "{:?}", e.diagnostico);
    exigir_silencio(&e.receptor);
}

#[test]
fn celula_desconocida_sale_fallo_sin_crear_sonda() {
    let e = ejecutar(
        &restablecer(&[]),
        vec![
            Guion::SinCuerpo {
                estado: 404,
                razon: "Not Found",
            },
            inspeccion_del_nucleo("running"),
        ],
        1,
    );
    assert_eq!(e.peticiones[0].objetivo, "/containers/celula-c-nucleo/json");
    assert_eq!(e.codigo, CodigoDeSalida::Fallo);
    assert!(e.estandar.is_empty());
    assert!(e.diagnostico.contains("célula no encontrada"));
    exigir_silencio(&e.receptor);
}

#[test]
fn nucleo_detenido_sale_fallo_sin_crear_sonda() {
    let e = ejecutar(
        &restablecer(&[]),
        vec![
            inspeccion_del_nucleo("exited"),
            inspeccion_del_nucleo("running"),
        ],
        1,
    );
    assert_eq!(e.codigo, CodigoDeSalida::Fallo);
    assert!(e.estandar.is_empty());
    assert!(e.diagnostico.contains("no está en ejecución"));
    exigir_silencio(&e.receptor);
}

/// Respuestas de la ruta (o de la sonda) que deben terminar en Fallo sin línea de éxito.
fn exigir_fallo_por_respuesta(respuesta: &'static [u8], codigo_wget: i64, esperado: &str) {
    let e = ejecutar(
        &restablecer(&[]),
        guiones_de_sonda(respuesta, codigo_wget),
        PETICIONES_DE_SONDA_COMPLETA,
    );
    assert_eq!(e.codigo, CodigoDeSalida::Fallo, "respuesta {respuesta:?}");
    assert!(
        e.estandar.is_empty(),
        "sin línea de éxito: {:?}",
        e.estandar
    );
    assert!(
        e.diagnostico.contains(esperado),
        "diagnóstico {:?} debía contener {esperado:?}",
        e.diagnostico
    );
    assert_eq!(
        e.peticiones[7].metodo, "DELETE",
        "la sonda se elimina siempre"
    );
}

#[test]
fn canal_sin_sesion_sale_fallo() {
    exigir_fallo_por_respuesta(
        br#"{"resultado":"canal_sin_sesion"}"#,
        0,
        "canal_sin_sesion",
    );
}

#[test]
fn fallido_sale_fallo_con_el_motivo() {
    exigir_fallo_por_respuesta(
        br#"{"resultado":"fallido","motivo":"sin_conexion"}"#,
        0,
        "sin_conexion",
    );
}

#[test]
fn cuerpo_ilegible_sale_fallo() {
    exigir_fallo_por_respuesta(b"<html>no</html>", 0, "JSON");
}

#[test]
fn sonda_fallida_o_agotada_sale_fallo() {
    // wget agotó su `-T` o la ruta no respondió 2xx: código distinto de 0 y stdout vacío.
    exigir_fallo_por_respuesta(b"", 4, "JSON");
}
