use hexcell_admin::argumentos::analizar;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::ejecutar;
use hexcell_admin::contacto::{
    DesenlaceDeRestablecimientoDeContacto, desenlace_de_restablecimiento,
};
use hexcell_admin::salida::Salida;

const CONTACTO: &str = "ct-0123456789abcdef0123456789abcdef";

fn args(texto: &[&str]) -> Vec<String> {
    texto.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn sin_incluir_baja_el_cuerpo_recibido_dice_false() {
    let comando = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id",
        "a",
        "--contacto",
        CONTACTO,
    ]))
    .unwrap();
    assert!(!comando.confirmar());
    assert!(!comando.simular());
}

#[test]
fn con_incluir_baja_el_cuerpo_recibido_dice_true() {
    let comando = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id",
        "a",
        "--contacto",
        CONTACTO,
        "--incluir-baja",
        "--confirmar",
    ]))
    .unwrap();
    assert!(comando.confirmar());
}

#[test]
fn simular_no_emite_ninguna_peticion_ni_crea_contenedor() {
    let comando = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id",
        "a",
        "--contacto",
        CONTACTO,
        "--simular",
    ]))
    .unwrap();
    let mut salida = Salida::nueva(Vec::new(), Vec::new());
    assert_eq!(ejecutar(Ok(comando), &mut salida), CodigoDeSalida::Exito);
}

#[test]
fn contacto_desconocido_sale_fallo_aunque_los_contadores_sean_cero() {
    let respuesta = br#"{"resultado":"contacto_desconocido","existe":false,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0}"#;
    assert_eq!(
        desenlace_de_restablecimiento(respuesta).unwrap(),
        DesenlaceDeRestablecimientoDeContacto::ContactoDesconocido
    );
}

#[test]
fn contacto_existente_sin_filas_sale_exito_con_la_linea_sin_cambios() {
    let respuesta = br#"{"resultado":"aplicado","existe":true,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0}"#;
    assert!(matches!(
        desenlace_de_restablecimiento(respuesta).unwrap(),
        DesenlaceDeRestablecimientoDeContacto::Aplicado { existe: true, .. }
    ));
}

#[test]
fn incluir_baja_sin_confirmar_no_emite_peticion_alguna() {
    assert!(
        analizar(&args(&[
            "contacto",
            "restablecer",
            "--id",
            "a",
            "--contacto",
            CONTACTO,
            "--incluir-baja"
        ]))
        .is_err()
    );
}
