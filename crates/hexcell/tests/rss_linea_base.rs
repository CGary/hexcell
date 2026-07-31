//! Línea base de memoria residente (RSS) del proceso `hexcell` en reposo, para NFR-01 (HEX-009).
//!
//! No es un test de regresión: es un procedimiento reproducible que arranca el binario real
//! contra el adaptador simulado (sin evento de arranque inyectado, motor ocioso), espera a que
//! termine de arrancar (línea `salud_vinculada`, ya usada por el resto de `tests/comun/mod.rs`),
//! deja que las páginas del asignador se asienten con una pausa corta, y lee `VmRSS` de
//! `/proc/<pid>/status`. Por eso queda `#[ignore]`: depende del host y del instante en que se
//! mide, así que no debe correr en el `cargo test --workspace` por defecto (AC-1), pero sí se
//! ejecuta a mano con `cargo test --workspace -- --ignored rss_linea_base --nocapture` para leer
//! el valor impreso y transcribirlo a `docs/STATUS.md` (AC-4).
//!
//! Solo funciona en Linux: `/proc/<pid>/status` no existe en otros sistemas operativos, y este
//! árbol ya asume Linux como destino de despliegue y de CI (ver CLAUDE.md/README).

mod comun;

use std::fs;
use std::time::Duration;

use comun::{DirectorioTemporal, lanzar_binario_con_ruta_de_datos};

#[test]
#[ignore]
fn mide_rss_linea_base_en_reposo() {
    let directorio = DirectorioTemporal::nuevo("rss-linea-base");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    // `salud_vinculada` ya demuestra que el arranque terminó; esta pausa adicional es solo para
    // dejar que las páginas del asignador se asienten antes de leer VmRSS (RISK-3 del blueprint).
    std::thread::sleep(Duration::from_millis(500));

    let ruta_status = format!("/proc/{}/status", binario.pid());
    let contenido = fs::read_to_string(&ruta_status)
        .unwrap_or_else(|error| panic!("no se pudo leer {ruta_status}: {error}"));

    let linea_vm_rss = contenido
        .lines()
        .find(|linea| linea.starts_with("VmRSS:"))
        .unwrap_or_else(|| panic!("no se encontró la línea VmRSS en {ruta_status}: {contenido}"));

    let kb: u64 = linea_vm_rss
        .split_whitespace()
        .nth(1)
        .and_then(|valor| valor.parse().ok())
        .unwrap_or_else(|| panic!("no se pudo interpretar la línea VmRSS: {linea_vm_rss}"));

    // Conversión kB -> MB por división entera (truncamiento hacia cero): una cifra conservadora,
    // nunca inflada, tal como registra RISK-5 del blueprint.
    let mb = kb / 1024;

    assert!(
        mb > 0,
        "el VmRSS medido debe ser un entero positivo en MB, se obtuvo {kb} kB"
    );

    println!("linea_base_rss_hexcell_en_reposo_mb={mb}");
}
