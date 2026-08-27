//! Tests de fragmentación de contenido para el motor de conocimiento.
//!
//! Este archivo cubre los criterios de aceptación AC-1 a AC-6 especificados
//! en la especificación 00-spec.yaml y el plano 01-blueprint.yaml.
//!
//! Los mensajes de aserción están en español, siguiendo la convención del
//! repositorio.

use hexcell_core::fragmentacion::{ConfiguracionDeFragmentacion, ErrorDeFragmentacion, fragmentar};

#[test]
fn ac_1_texto_corto_devuelve_un_solo_fragmento() {
    // Texto ASCII más corto que el tamaño de fragmento.
    let texto = "hello";
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 10,
        solapamiento: 2,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert_eq!(resultado, vec!["hello"]);

    // Texto no ASCII con la misma cantidad de caracteres.
    let texto_no_ascii = "áéíóú"; // 5 caracteres
    let resultado_no_ascii = fragmentar(texto_no_ascii, &config).expect("fragmentación válida");
    assert_eq!(resultado_no_ascii, vec!["áéíóú"]);
}

#[test]
fn ac_2_texto_vacio_devuelve_cero_fragmentos() {
    let texto = "";
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 2,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert!(resultado.is_empty());
}

#[test]
fn ac_3_texto_largo_produce_solapamiento_consistente() {
    // Texto largo: 30 caracteres, tamaño de fragmento 10, solapamiento 3.
    let texto = "012345678901234567890123456789"; // 30 caracteres
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 10,
        solapamiento: 3,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    // Esperamos 4 fragmentos: [0-10], [7-17], [14-24], [21-30]
    assert_eq!(resultado.len(), 4);
    assert_eq!(resultado[0], "0123456789");
    assert_eq!(resultado[1], "7890123456");
    assert_eq!(resultado[2], "4567890123");
    assert_eq!(resultado[3], "123456789"); // último fragmento (irregular)

    // Verificar solapamiento: cada fragmento después del primero debe comenzar
    // con los últimos `solapamiento` caracteres del fragmento anterior.
    assert_eq!(&resultado[1][..3], &resultado[0][7..10]); // "789" == "789"
    assert_eq!(&resultado[2][..3], &resultado[1][7..10]); // "456" == "456"
    assert_eq!(&resultado[3][..3], &resultado[2][7..10]); // "789" == "789"
}

#[test]
fn ac_3_resta_irregular_todavia_solapada() {
    // Caso donde el último fragmento es más corto que el solapamiento.
    // Texto de 12 caracteres, tamaño 10, solapamiento 8.
    // Fragmentos esperados: [0-10], [2-12] (porque 10 - 8 = 2 de avance)
    let texto = "012345678901"; // 12 caracteres
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 10,
        solapamiento: 8,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert_eq!(resultado.len(), 2);
    assert_eq!(resultado[0], "0123456789");
    assert_eq!(resultado[1], "2345678901"); // comienza en el índice 2

    // Verificar solapamiento: el segundo fragmento debe comenzar con
    // los últimos 8 caracteres del primero.
    // Los últimos 8 del primero: "23456789"
    // El comienzo del segundo: "23456789" (de hecho, el segundo es "2345678901")
    assert_eq!(&resultado[1][..8], &resultado[0][2..10]);
}

#[test]
fn ac_4_texto_listeado_limite_puede_caer_dentro_de_linea() {
    // Texto de viñetas de una letra cada una: "- A\n", "- B\n", ..., "- J"
    // (la última viñeta no lleva salto de línea final). Cada viñeta ocupa
    // 4 caracteres, salvo la última que ocupa 3.
    let texto = "- A\n- B\n- C\n- D\n- E\n- F\n- G\n- H\n- I\n- J";
    // Con tamaño 8 y solapamiento 3 el paso de avance es 8 - 3 = 5, que no es
    // múltiplo de 4 (el largo de cada viñeta). Por construcción, ningún límite
    // de fragmento cae alineado con un salto de línea: el segundo fragmento
    // debe partir dentro de la viñeta "- B", no al comienzo de una viñeta.
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 8,
        solapamiento: 3,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert_eq!(resultado.len(), 8);
    // Primer fragmento: las dos primeras viñetas completas, límite alineado.
    assert_eq!(resultado[0], "- A\n- B\n");
    // Segundo fragmento: comienza en el índice 5, en medio de la viñeta "- B"
    // (en el espacio que separa el guion de la letra, antes de "B"). Este es
    // el límite mid-línea que AC-4 exige documentar y probar explícitamente:
    // la ventana de caracteres no espera a un salto de línea para cortar.
    assert_eq!(resultado[1], " B\n- C\n-");
}

#[test]
fn ac_5_solapamiento_igual_o_mayor_que_tamano_devuelve_error() {
    let texto = "no vacío";
    // Caso solapamiento == tamaño
    let config_igual = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 5,
    };
    let resultado_igual = fragmentar(texto, &config_igual);
    assert!(resultado_igual.is_err());
    if let Err(ErrorDeFragmentacion::SolapamientoNoMenorQueTamano {
        tamano_de_fragmento,
        solapamiento,
    }) = resultado_igual
    {
        assert_eq!(tamano_de_fragmento, 5);
        assert_eq!(solapamiento, 5);
    } else {
        panic!("Error inesperado: {:?}", resultado_igual);
    }

    // Caso solapamiento > tamaño
    let config_mayor = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 5,
        solapamiento: 6,
    };
    let resultado_mayor = fragmentar(texto, &config_mayor);
    assert!(resultado_mayor.is_err());
    if let Err(ErrorDeFragmentacion::SolapamientoNoMenorQueTamano {
        tamano_de_fragmento,
        solapamiento,
    }) = resultado_mayor
    {
        assert_eq!(tamano_de_fragmento, 5);
        assert_eq!(solapamiento, 6);
    } else {
        panic!("Error inesperado: {:?}", resultado_mayor);
    }
}

#[test]
fn ac_6_limites_no_parten_caracteres_unicode() {
    // Texto con acentos, eñe y emoji (multi-byte en UTF-8).
    let texto = "áéíóúñ🚀"; // 7 caracteres: á, é, í, ó, ú, ñ, 🚀
    // Elegimos un tamaño de fragmento que, si se contara por bytes, partiría un carácter.
    // Pero nosotros trabajamos en caracteres, así que usamos 3 caracteres.
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 3,
        solapamiento: 1,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    // Verificar que cada fragmento es válido UTF-8 (al ser String, lo es).
    // Y que la concatenación de los fragmentos sin solapamiento da el original.
    // Reconstruir: tomar el primer fragmento completo, luego cada siguiente
    // fragmento desde el índice `solapamiento` (1) en adelante.
    let mut reconstruido = String::new();
    if !resultado.is_empty() {
        reconstruido.push_str(&resultado[0]);
        for fragmento in &resultado[1..] {
            let mut chars = fragmento.chars();
            for _ in 0..config.solapamiento {
                chars.next();
            }
            reconstruido.extend(chars);
        }
    }
    assert_eq!(reconstruido, texto);
}

#[test]
fn ac_1_longitud_exacta_al_tamano_produce_un_solo_fragmento() {
    // Sitio clásico de error por uno: cuando el texto mide EXACTAMENTE
    // `tamano_de_fragmento`, `fin` alcanza `len` en la primera iteración
    // y el bucle debe terminar sin avanzar `inicio` una segunda vez.
    let texto = "0123456789"; // 10 caracteres
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 10,
        solapamiento: 2,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert_eq!(resultado.len(), 1);
    assert_eq!(resultado[0], texto);
}

#[test]
fn ac_1_longitud_tamano_mas_uno_produce_dos_fragmentos() {
    // Un carácter más que `tamano_de_fragmento` es el primer caso donde el
    // bucle SÍ debe volver a iterar: el primer fragmento no alcanza `len`,
    // así que `inicio` avanza en (tamano_de_fragmento - solapamiento) y se
    // produce un segundo fragmento con el resto solapado.
    let texto = "01234567890"; // 11 caracteres
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 10,
        solapamiento: 2,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert_eq!(resultado.len(), 2);
    assert_eq!(resultado[0], "0123456789");
    // inicio avanza a 10 - 2 = 8; el segundo fragmento son los caracteres [8..11].
    assert_eq!(resultado[1], "890");
}

#[test]
fn ac_5_tamano_uno_con_solapamiento_cero_es_la_configuracion_minima_valida() {
    // tamano_de_fragmento = 1 y solapamiento = 0 es el par válido más
    // pequeño posible (solapamiento < tamano_de_fragmento se cumple en el
    // límite). Cada carácter debe convertirse en su propio fragmento, sin
    // solapamiento entre fragmentos consecutivos.
    let texto = "abc";
    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 1,
        solapamiento: 0,
    };
    let resultado = fragmentar(texto, &config).expect("fragmentación válida");
    assert_eq!(resultado, vec!["a", "b", "c"]);
}
