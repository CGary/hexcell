use std::fs;

#[test]
fn la_documentacion_de_contacto_conserva_los_contratos_clave() {
    let raiz = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let readme = fs::read_to_string(raiz.join("README.md")).unwrap();
    let runbook = fs::read_to_string(raiz.join("docs/runbook-operacion.md")).unwrap();
    let estado = fs::read_to_string(raiz.join("docs/STATUS.md")).unwrap();
    assert!(readme.contains("### 10. Restablecer un contacto"));
    assert!(runbook.contains("identidad.baja_de_contacto_revivida"));
    assert!(runbook.contains("hexcell-admin contacto restablecer --incluir-baja --confirmar"));
    assert!(estado.contains("Definido el 2026-09-30 con HEX-091"));
}
