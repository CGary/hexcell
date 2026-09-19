//! Pruebas externas del renderizado de configuración por célula (HEX-081).
//!
//! Crate externo que solo ve la API pública de `hexcell-admin`. Dos capas: las funciones puras
//! de `renderizado_configuracion` (analizar/combinar/serializar) se ejercitan directamente
//! sobre cadenas en memoria; el subcomando completo `config render` se ejercita a través de
//! `argumentos::analizar` + `comandos::ejecutar`, escribiendo únicamente en un directorio
//! temporal propio del proceso que se borra al final de cada prueba (nunca en el árbol del
//! repositorio ni en una ruta versionada).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hexcell_admin::argumentos::analizar;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::ejecutar;
use hexcell_admin::renderizado_configuracion::{
    ErrorDeRenderizado, analizar_env, combinar, serializar,
};
use hexcell_admin::salida::Salida;

static SECUENCIA: AtomicUsize = AtomicUsize::new(0);

/// Directorio temporal único para una prueba, borrado al salir de alcance.
struct DirectorioTemporal(PathBuf);

impl DirectorioTemporal {
    fn nuevo(etiqueta: &str) -> Self {
        let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
        let ruta = std::env::temp_dir().join(format!(
            "hexcell-admin-render-{etiqueta}-{}-{secuencia}",
            std::process::id()
        ));
        std::fs::create_dir_all(&ruta).expect("crear directorio temporal de la prueba");
        Self(ruta)
    }

    fn ruta(&self) -> &Path {
        &self.0
    }
}

impl Drop for DirectorioTemporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn escribir(dir: &DirectorioTemporal, nombre: &str, contenido: &str) -> PathBuf {
    let ruta = dir.ruta().join(nombre);
    let mut archivo = std::fs::File::create(&ruta).expect("crear archivo de la prueba");
    archivo
        .write_all(contenido.as_bytes())
        .expect("escribir archivo de la prueba");
    ruta
}

fn args(snippet: &[&str]) -> Vec<String> {
    snippet.iter().map(|s| (*s).to_string()).collect()
}

fn ejecutar_render(
    defecto: &Path,
    superposicion: &Path,
    salida_ruta: &Path,
) -> (CodigoDeSalida, String) {
    let argumentos = args(&[
        "config",
        "render",
        "--defecto",
        defecto.to_str().unwrap(),
        "--superposicion",
        superposicion.to_str().unwrap(),
        "--salida",
        salida_ruta.to_str().unwrap(),
    ]);
    let resultado = analizar(&argumentos);
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    let codigo = {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        ejecutar(resultado, &mut salida)
    };
    let diagnostico = String::from_utf8(bufer_diagnostico).expect("UTF-8 en el diagnóstico");
    (codigo, diagnostico)
}

// --- Funciones puras: analizar_env / combinar / serializar --------------------------------

#[test]
fn analizar_env_ignora_comentarios_y_lineas_en_blanco() {
    let texto = "# comentario\n\nHEXCELL_ID_CELULA=celula-x\n";
    let mapa = analizar_env(texto).expect("texto válido");
    assert_eq!(
        mapa.get("HEXCELL_ID_CELULA").map(String::as_str),
        Some("celula-x")
    );
    assert_eq!(mapa.len(), 1);
}

#[test]
fn analizar_env_rechaza_clave_desconocida() {
    let texto = "HEXCELL_CLAVE_INVENTADA=valor\n";
    match analizar_env(texto) {
        Err(ErrorDeRenderizado::ClaveInvalida { clave, .. }) => {
            assert_eq!(clave, "HEXCELL_CLAVE_INVENTADA");
        }
        otro => panic!("se esperaba ClaveInvalida, se obtuvo {otro:?}"),
    }
}

#[test]
fn combinar_hace_ganar_la_superposicion_por_clave() {
    let defecto =
        analizar_env("HEXCELL_ID_CELULA=defecto\nHEXCELL_RED_CELULA=red-defecto\n").unwrap();
    let superposicion = analizar_env("HEXCELL_ID_CELULA=superpuesto\n").unwrap();
    let combinado = combinar(defecto, superposicion).expect("combinación válida");
    assert_eq!(
        combinado.get("HEXCELL_ID_CELULA").map(String::as_str),
        Some("superpuesto")
    );
    assert_eq!(
        combinado.get("HEXCELL_RED_CELULA").map(String::as_str),
        Some("red-defecto")
    );
}

#[test]
fn serializar_produce_una_linea_key_igual_valor_por_clave() {
    let mapa = analizar_env("HEXCELL_ID_CELULA=celula-x\n").unwrap();
    let texto = serializar(&mapa);
    assert_eq!(texto, "HEXCELL_ID_CELULA=celula-x\n");
}

// --- Subcomando completo: AC-1, AC-2, AC-3 -------------------------------------------------

#[test]
fn ac1_la_superposicion_prevalece_sobre_el_defecto_en_la_clave_compartida() {
    let dir = DirectorioTemporal::nuevo("ac1");
    let defecto = escribir(
        &dir,
        "defecto.env",
        "HEXCELL_ID_CELULA=celula-defecto\nHEXCELL_RED_CELULA=red-defecto\n",
    );
    let superposicion = escribir(&dir, "superposicion.env", "HEXCELL_ID_CELULA=celula-01\n");
    let salida_ruta = dir.ruta().join("render.env");

    let (codigo, diagnostico) = ejecutar_render(&defecto, &superposicion, &salida_ruta);

    assert_eq!(codigo, CodigoDeSalida::Exito, "diagnóstico: {diagnostico}");
    let contenido = std::fs::read_to_string(&salida_ruta).expect("la salida debe existir");
    assert!(contenido.contains("HEXCELL_ID_CELULA=celula-01\n"));
    assert!(contenido.contains("HEXCELL_RED_CELULA=red-defecto\n"));
}

#[test]
fn ac2_clave_desconocida_en_la_superposicion_aborta_sin_escribir_salida() {
    let dir = DirectorioTemporal::nuevo("ac2");
    let defecto = escribir(&dir, "defecto.env", "HEXCELL_ID_CELULA=celula-defecto\n");
    let superposicion = escribir(
        &dir,
        "superposicion.env",
        "HEXCELL_ID_CELULA=celula-01\nHEXCELL_CLAVE_INVENTADA=valor\n",
    );
    let salida_ruta = dir.ruta().join("render.env");

    let (codigo, diagnostico) = ejecutar_render(&defecto, &superposicion, &salida_ruta);

    assert_eq!(codigo, CodigoDeSalida::Fallo);
    assert!(
        diagnostico.contains("HEXCELL_CLAVE_INVENTADA"),
        "diagnóstico: {diagnostico}"
    );
    assert!(
        !salida_ruta.exists(),
        "no debe crearse el archivo de salida"
    );
}

#[test]
fn ac2_no_sobrescribe_una_salida_preexistente_ante_clave_desconocida() {
    let dir = DirectorioTemporal::nuevo("ac2-preexistente");
    let defecto = escribir(&dir, "defecto.env", "HEXCELL_ID_CELULA=celula-defecto\n");
    let superposicion = escribir(&dir, "superposicion.env", "HEXCELL_CLAVE_INVENTADA=valor\n");
    let salida_ruta = escribir(&dir, "render.env", "CONTENIDO_PREVIO_INTACTO\n");

    let (codigo, _diagnostico) = ejecutar_render(&defecto, &superposicion, &salida_ruta);

    assert_eq!(codigo, CodigoDeSalida::Fallo);
    let contenido =
        std::fs::read_to_string(&salida_ruta).expect("el archivo previo debe seguir existiendo");
    assert_eq!(contenido, "CONTENIDO_PREVIO_INTACTO\n");
}

#[test]
fn ac3_valor_invalido_para_clave_conocida_aborta_sin_escribir_salida() {
    let dir = DirectorioTemporal::nuevo("ac3");
    let defecto = escribir(&dir, "defecto.env", "HEXCELL_ID_CELULA=celula-defecto\n");
    let superposicion = escribir(
        &dir,
        "superposicion.env",
        "HEXCELL_ID_CELULA=celula-01\nHEXCELL_TELEFONO_CELULA=\n",
    );
    let salida_ruta = dir.ruta().join("render.env");

    let (codigo, diagnostico) = ejecutar_render(&defecto, &superposicion, &salida_ruta);

    assert_eq!(codigo, CodigoDeSalida::Fallo);
    assert!(
        diagnostico.contains("HEXCELL_TELEFONO_CELULA"),
        "diagnóstico: {diagnostico}"
    );
    assert!(
        !salida_ruta.exists(),
        "no debe crearse el archivo de salida"
    );
}
