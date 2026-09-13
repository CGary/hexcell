//! Pruebas del sumidero tipado de salida `Salida`.
//!
//! Viven en `tests/`, es decir, en un crate externo que solo ve la API pública de
//! `hexcell-admin`. Verifican la propiedad central del tipo: un mensaje legible para el operador
//! nunca llega al flujo de diagnóstico, y un mensaje de diagnóstico nunca llega al flujo legible
//! para el operador, en las dos direcciones. También verifican que una escritura fallida se
//! devuelve como valor de error — nunca como pánico — porque el perfil de publicación de este
//! workspace fija `panic = "abort"`.

use std::io::{self, Write};

use hexcell_admin::salida::Salida;

/// Escritor de prueba cuya escritura siempre falla, para ejercitar la ruta de error de
/// [`Salida`] sin depender de una tubería del sistema operativo realmente rota.
struct EscritorQueFalla;

impl Write for EscritorQueFalla {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("fallo simulado de escritura"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn una_linea_estandar_llega_solo_al_bufer_estandar() {
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        salida
            .linea("mensaje para el operador")
            .expect("la escritura sobre un Vec<u8> nunca falla");
    }
    assert_eq!(bufer_estandar, b"mensaje para el operador\n");
    assert!(
        bufer_diagnostico.is_empty(),
        "el mensaje estándar no debe llegar al bufer de diagnóstico"
    );
}

#[test]
fn un_diagnostico_llega_solo_al_bufer_de_diagnostico() {
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        salida
            .diagnostico("advertencia de diagnóstico")
            .expect("la escritura sobre un Vec<u8> nunca falla");
    }
    assert_eq!(bufer_diagnostico, "advertencia de diagnóstico\n".as_bytes());
    assert!(
        bufer_estandar.is_empty(),
        "el diagnóstico no debe llegar al bufer estándar"
    );
}

#[test]
fn los_mensajes_escritos_son_el_literal_exacto_en_espanol() {
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        salida
            .linea("célula reanudada correctamente")
            .expect("la escritura sobre un Vec<u8> nunca falla");
        salida
            .diagnostico("no se pudo alcanzar el daemon de Docker")
            .expect("la escritura sobre un Vec<u8> nunca falla");
    }
    assert_eq!(
        String::from_utf8(bufer_estandar).expect("la salida es UTF-8 válido"),
        "célula reanudada correctamente\n"
    );
    assert_eq!(
        String::from_utf8(bufer_diagnostico).expect("la salida es UTF-8 válido"),
        "no se pudo alcanzar el daemon de Docker\n"
    );
}

#[test]
fn una_escritura_estandar_fallida_se_devuelve_como_error_sin_entrar_en_panico() {
    let mut salida = Salida::nueva(EscritorQueFalla, Vec::new());
    let resultado = salida.linea("esto nunca llega a ningún lado");
    assert!(resultado.is_err(), "la escritura fallida debe ser un Err");
}

#[test]
fn una_escritura_de_diagnostico_fallida_se_devuelve_como_error_sin_entrar_en_panico() {
    let mut salida = Salida::nueva(Vec::new(), EscritorQueFalla);
    let resultado = salida.diagnostico("esto tampoco llega a ningún lado");
    assert!(resultado.is_err(), "la escritura fallida debe ser un Err");
}

#[test]
fn el_constructor_de_produccion_se_puede_invocar() {
    // No se escribe nada: solo se verifica que Salida::estandar() compila y produce el tipo
    // concreto Salida<Stdout, Stderr> sin tocar los descriptores reales del proceso de prueba.
    let _salida = Salida::estandar();
}
