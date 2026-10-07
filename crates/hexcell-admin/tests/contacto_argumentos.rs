use hexcell_admin::argumentos::{Comando, ErrorDeArgumentos, analizar};

fn args(texto: &[&str]) -> Vec<String> {
    texto.iter().map(|s| (*s).to_string()).collect()
}

const CONTACTO: &str = "ct-0123456789abcdef0123456789abcdef";

#[test]
fn incluir_baja_sin_confirmar_es_uso_incorrecto_y_nombra_confirmar() {
    let error = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id",
        "a",
        "--contacto",
        CONTACTO,
        "--incluir-baja",
    ]))
    .expect_err("debe exigir confirmación");
    assert!(matches!(error, ErrorDeArgumentos::ContactoInvalido { .. }));
    assert!(error.to_string().contains("--confirmar"));
}

#[test]
fn confirmar_sin_incluir_baja_es_uso_incorrecto() {
    let error = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id",
        "a",
        "--contacto",
        CONTACTO,
        "--confirmar",
    ]))
    .expect_err("confirmar aislado no se admite");
    assert!(error.to_string().contains("--incluir-baja"));
}

#[test]
fn restablecer_por_omision_no_pide_confirmar() {
    let comando = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id=a",
        &format!("--contacto={CONTACTO}"),
    ]))
    .expect("la operación por defecto es válida");
    match comando {
        Comando::Contacto(i) => {
            assert_eq!(i.id(), "a");
            assert_eq!(i.contacto(), CONTACTO);
            assert!(!i.incluir_baja());
            assert!(!i.confirmar());
        }
        _ => panic!("se esperaba contacto"),
    }
}

#[test]
fn simular_con_incluir_baja_no_pide_confirmar() {
    let comando = analizar(&args(&[
        "contacto",
        "restablecer",
        "--id",
        "a",
        "--contacto",
        CONTACTO,
        "--incluir-baja",
        "--simular",
    ]))
    .expect("simular no exige confirmar");
    assert!(comando.simular());
    assert!(!comando.confirmar());
}

#[test]
fn contacto_valida_forma_y_rechaza_entrada_multibyte() {
    for invalido in [
        "ct-0123456789ABCDEF0123456789abcdef",
        "ct-123",
        "xx-0123456789abcdef0123456789abcdef",
        "ct-0123456789abcdef0123456789abcdeé",
    ] {
        assert!(
            analizar(&args(&[
                "contacto",
                "restablecer",
                "--id",
                "a",
                "--contacto",
                invalido
            ]))
            .is_err()
        );
    }
}
