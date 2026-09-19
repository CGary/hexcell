//! Pruebas externas del esquema cerrado de configuración `esquema_configuracion::validar_clave`.
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`: el esquema es una función
//! pura sobre `(clave, valor)`, así que las pruebas lo ejercitan sin tocar el sistema de
//! archivos ni el subcomando de la CLI.

use hexcell_admin::esquema_configuracion::{ESQUEMA_PERMITIDO, validar_clave};

#[test]
fn clave_desconocida_es_rechazada() {
    assert!(validar_clave("HEXCELL_CLAVE_INVENTADA", "cualquier-valor").is_err());
}

#[test]
fn secretos_nunca_estan_en_el_esquema_permitido() {
    for clave in [
        "HEXCELL_INFERENCIA_API_KEY",
        "HEXCELL_EMBEDDINGS_API_KEY",
        "HEXCELL_TELEGRAM_BOT_TOKEN",
    ] {
        assert!(
            !ESQUEMA_PERMITIDO.contains(&clave),
            "«{clave}» es un secreto y no debe figurar en el esquema permitido"
        );
        assert!(
            validar_clave(clave, "valor-cualquiera").is_err(),
            "«{clave}» debe rechazarse como clave desconocida"
        );
    }
}

#[test]
fn identificadores_operativos_no_secretos_estan_permitidos() {
    assert!(ESQUEMA_PERMITIDO.contains(&"HEXCELL_TELEGRAM_CHAT_ID"));
    assert!(ESQUEMA_PERMITIDO.contains(&"HEXCELL_TELEFONO_CELULA"));
    assert!(validar_clave("HEXCELL_TELEGRAM_CHAT_ID", "chat-ejemplo").is_ok());
    assert!(validar_clave("HEXCELL_TELEFONO_CELULA", "5491100000000").is_ok());
}

#[test]
fn valor_vacio_es_rechazado_para_clave_conocida() {
    assert!(validar_clave("HEXCELL_TELEFONO_CELULA", "").is_err());
}

#[test]
fn zona_horaria_valida_e_invalida() {
    assert!(validar_clave("HEXCELL_VENTANA_ZONA", "America/Argentina/Buenos_Aires").is_ok());
    assert!(validar_clave("HEXCELL_VENTANA_ZONA", "no-es-una-zona").is_err());
    assert!(validar_clave("HEXCELL_VENTANA_ZONA", "").is_err());
}

#[test]
fn limite_de_memoria_valido_e_invalido() {
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_MEMORIA", "48m").is_ok());
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_MEMORIA", "0m").is_err());
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_MEMORIA", "48x").is_err());
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_MEMORIA", "abc").is_err());
}

#[test]
fn limite_de_cpus_decimal_positivo() {
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_CPUS", "0.5").is_ok());
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_CPUS", "0").is_err());
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_CPUS", "-1").is_err());
}

#[test]
fn entero_positivo_para_nofile_y_temporizadores() {
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_NOFILE", "1024").is_ok());
    assert!(validar_clave("HEXCELL_NUCLEO_LIMITE_NOFILE", "0").is_err());
    assert!(validar_clave("HEXCELL_TELEGRAM_TIMEOUT_MS", "5000").is_ok());
    assert!(validar_clave("HEXCELL_TELEGRAM_TIMEOUT_MS", "-5").is_err());
}

#[test]
fn ratio_de_acuses_debe_estar_entre_cero_y_uno() {
    assert!(validar_clave("HEXCELL_ALERTAS_LIMITE_CAIDA_RATIO_ACUSES", "0.5").is_ok());
    assert!(validar_clave("HEXCELL_ALERTAS_LIMITE_CAIDA_RATIO_ACUSES", "1.5").is_err());
    assert!(validar_clave("HEXCELL_ALERTAS_LIMITE_CAIDA_RATIO_ACUSES", "-0.1").is_err());
}

#[test]
fn clave_generica_rechaza_espacios() {
    assert!(validar_clave("HEXCELL_ID_CELULA", "celula-ejemplo").is_ok());
    assert!(validar_clave("HEXCELL_ID_CELULA", "celula con espacios").is_err());
}
