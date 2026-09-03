//! Tests de los pools duales: parámetros de SQLite (AC-1, AC-2), ausencia de identificadores de
//! transporte en el esquema (AC-4) y sonda de vitalidad (AC-10, parte de almacenamiento).

mod comun;

use comun::DirectorioTemporal;
use hexcell_storage::{
    BUSY_TIMEOUT, CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO, ErrorDeAlmacen, GestorDePools,
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_SESIONES, PoolDeConocimiento,
    SUFIJO_DE_ARCHIVO_WAL, Vitalidad,
};
use rusqlite::Connection;

/// `PRAGMA synchronous` devuelve el modo como entero; `NORMAL` es el 1.
const SYNCHRONOUS_NORMAL: i64 = 1;

/// Fragmentos de identificador de transporte que **ninguna** columna ni ningún texto de esquema
/// puede contener. La lista incluye sinónimos creativos a propósito: la regla de `adr-0010` es
/// semántica, y una comprobación que solo mirara el nombre exacto de una columna sería inútil en
/// cuanto alguien la renombrase.
const IDENTIFICADORES_DE_TRANSPORTE_PROHIBIDOS: [&str; 11] = [
    "wa_id",
    "waid",
    "jid",
    "remote_jid",
    "chat_id",
    "telefono",
    "phone",
    "msisdn",
    "e164",
    "numero_de_telefono",
    "whatsapp",
];

fn leer_pragma_entero(conexion: &Connection, pragma: &str) -> i64 {
    conexion
        .query_row(&format!("PRAGMA {pragma}"), [], |fila| fila.get(0))
        .unwrap_or_else(|error| panic!("leer PRAGMA {pragma}: {error}"))
}

fn leer_pragma_texto(conexion: &Connection, pragma: &str) -> String {
    conexion
        .query_row(&format!("PRAGMA {pragma}"), [], |fila| fila.get(0))
        .unwrap_or_else(|error| panic!("leer PRAGMA {pragma}: {error}"))
}

#[test]
fn las_dos_bases_se_abren_migradas_y_con_los_parametros_declarados() {
    let directorio = DirectorioTemporal::nuevo("pools-parametros");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los dos pools");

    assert!(
        directorio
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_SESIONES)
            .is_file()
    );
    assert!(
        directorio
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO)
            .is_file()
    );

    let busy_esperado = i64::try_from(BUSY_TIMEOUT.as_millis()).expect("el timeout cabe en i64");

    for etiqueta in ["escritura", "lectura"] {
        let comprobar = |conexion: &Connection| {
            assert_eq!(
                leer_pragma_texto(conexion, "journal_mode").to_lowercase(),
                "wal",
                "sessions.db debe estar en WAL desde la conexión de {etiqueta}"
            );
            assert_eq!(leer_pragma_entero(conexion, "busy_timeout"), busy_esperado);
            assert_eq!(
                leer_pragma_entero(conexion, "synchronous"),
                SYNCHRONOUS_NORMAL
            );
            assert_eq!(leer_pragma_entero(conexion, "foreign_keys"), 1);
            Ok(())
        };

        if etiqueta == "escritura" {
            gestor
                .sesiones()
                .con_escritura(comprobar)
                .expect("la conexión de escritura debe responder");
        } else {
            gestor
                .sesiones()
                .con_lectura(comprobar)
                .expect("la conexión de lectura debe responder");
        }
    }

    // El pool de conocimiento se recorre tantas veces como conexiones tiene para que el turno
    // rotatorio toque todas y no solo la primera.
    for _ in 0..CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO {
        gestor
            .conocimiento()
            .con_lectura(|conexion| {
                assert_eq!(
                    leer_pragma_texto(conexion, "journal_mode").to_lowercase(),
                    "wal"
                );
                assert_eq!(leer_pragma_entero(conexion, "busy_timeout"), busy_esperado);
                assert_eq!(
                    leer_pragma_entero(conexion, "synchronous"),
                    SYNCHRONOUS_NORMAL
                );
                Ok(())
            })
            .expect("cada conexión de conocimiento debe responder");
    }
}

#[test]
fn el_pool_de_conocimiento_es_de_solo_lectura_en_produccion() {
    let directorio = DirectorioTemporal::nuevo("pools-solo-lectura");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los dos pools");

    let resultado = gestor.conocimiento().con_lectura(|conexion| {
        let intento = conexion.execute(
            "INSERT INTO metadatos_de_conocimiento (clave, valor) VALUES ('x', 'y')",
            [],
        );
        assert!(
            intento.is_err(),
            "escribir en knowledge_live.db desde el pool de producción debe fallar"
        );
        Ok(())
    });
    assert!(resultado.is_ok());
}

#[test]
fn ninguna_columna_ni_el_esquema_almacenado_nombran_un_identificador_de_transporte() {
    let directorio = DirectorioTemporal::nuevo("pools-identidad");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los dos pools");

    let revisar = |conexion: &Connection| {
        let mut sentencia = conexion
            .prepare("SELECT name, coalesce(sql, '') FROM sqlite_schema")
            .expect("preparar la lectura del esquema");
        let objetos: Vec<(String, String)> = sentencia
            .query_map([], |fila| Ok((fila.get(0)?, fila.get(1)?)))
            .expect("leer el esquema")
            .map(|fila| fila.expect("una fila del esquema"))
            .collect();

        assert!(!objetos.is_empty(), "el esquema no puede estar vacío");

        for (nombre, sql) in &objetos {
            // El texto completo del esquema, comentarios incluidos, no puede nombrarlos.
            let sql_en_minusculas = sql.to_lowercase();
            for prohibido in IDENTIFICADORES_DE_TRANSPORTE_PROHIBIDOS {
                assert!(
                    !sql_en_minusculas.contains(prohibido),
                    "el objeto {nombre} nombra un identificador de transporte: {prohibido}"
                );
            }
        }

        // Y además se recorre columna a columna, que es lo que de verdad se persiste.
        let tablas: Vec<String> = objetos
            .iter()
            .filter(|(nombre, sql)| !nombre.starts_with("sqlite_") && !sql.is_empty())
            .map(|(nombre, _)| nombre.clone())
            .collect();

        for tabla in tablas {
            let mut columnas = conexion
                .prepare(&format!("PRAGMA table_info({tabla})"))
                .expect("preparar pragma_table_info");
            let nombres: Vec<String> = columnas
                .query_map([], |fila| fila.get::<_, String>(1))
                .expect("leer las columnas")
                .map(|fila| fila.expect("una columna"))
                .collect();
            for nombre in nombres {
                let nombre_en_minusculas = nombre.to_lowercase();
                for prohibido in IDENTIFICADORES_DE_TRANSPORTE_PROHIBIDOS {
                    assert!(
                        !nombre_en_minusculas.contains(prohibido),
                        "la columna {tabla}.{nombre} nombra un identificador de transporte"
                    );
                }
            }
        }
        Ok(())
    };

    gestor
        .sesiones()
        .con_lectura(revisar)
        .expect("revisar sessions.db");
    gestor
        .conocimiento()
        .con_lectura(revisar)
        .expect("revisar knowledge_live.db");
}

#[test]
fn la_vitalidad_es_sana_al_abrir_y_cae_cuando_el_archivo_desaparece_del_disco() {
    let directorio = DirectorioTemporal::nuevo("pools-vitalidad");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los dos pools");

    assert_eq!(gestor.sesiones().vitalidad(), Vitalidad::Sana);
    assert_eq!(gestor.conocimiento().vitalidad(), Vitalidad::Sana);

    // En Linux, borrar el archivo no invalida el descriptor ya abierto: una sonda que solo
    // consultara seguiría respondiendo que todo va bien. La sonda comprueba también la ruta.
    std::fs::remove_file(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES))
        .expect("borrar sessions.db del disco");

    match gestor.sesiones().vitalidad() {
        Vitalidad::Sana => panic!("la sonda debe detectar que sessions.db ya no está en disco"),
        Vitalidad::Caida { componente, motivo } => {
            assert_eq!(componente, NOMBRE_DE_ARCHIVO_DE_SESIONES);
            assert!(!motivo.is_empty(), "la caída debe explicarse");
        }
    }

    // El otro pool es independiente: su archivo sigue en su sitio.
    assert_eq!(gestor.conocimiento().vitalidad(), Vitalidad::Sana);

    std::fs::remove_file(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO))
        .expect("borrar knowledge_live.db del disco");
    match gestor.conocimiento().vitalidad() {
        Vitalidad::Sana => panic!("la sonda debe detectar que knowledge_live.db ya no está"),
        Vitalidad::Caida { componente, .. } => {
            assert_eq!(componente, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
        }
    }
}

#[test]
fn el_punto_de_control_deja_el_wal_de_sesiones_en_cero_bytes_con_los_pools_abiertos() {
    // El test se hace falsificable manteniendo los pools ABIERTOS: comprobado el 2026-07-30,
    // SQLite consolida y borra por sí solo el archivo `-wal` cuando la última conexión a una base
    // se cierra, así que comprobar el `-wal` después de que el proceso termine pasaría
    // exactamente igual con o sin este punto de control explícito — una aserción que no puede
    // fallar es decoración, no una prueba.
    let directorio = DirectorioTemporal::nuevo("pools-checkpoint");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los dos pools");

    // Escribe suficientes filas para que el WAL de sessions.db crezca por encima de cero bytes
    // antes del punto de control.
    gestor
        .sesiones()
        .con_escritura(|conexion| {
            for indice in 0..200i64 {
                conexion
                    .execute(
                        "INSERT INTO estado_del_motor (clave, valor) VALUES (?1, ?2)",
                        rusqlite::params![format!("clave-{indice}"), indice],
                    )
                    .expect("insertar una fila de prueba");
            }
            Ok(())
        })
        .expect("escribir las filas de prueba");

    let ruta_wal = directorio.ruta().join(format!(
        "{}{SUFIJO_DE_ARCHIVO_WAL}",
        NOMBRE_DE_ARCHIVO_DE_SESIONES
    ));
    let tamano_antes = std::fs::metadata(&ruta_wal)
        .map(|metadatos| metadatos.len())
        .unwrap_or(0);
    assert!(
        tamano_antes > 0,
        "el WAL debe haber crecido por encima de cero bytes antes del punto de control"
    );

    let resumen = gestor.punto_de_control_de_wal();
    assert!(
        !resumen.ocupado,
        "el punto de control no debe estar ocupado en este test"
    );
    assert_eq!(
        resumen.tamano_wal_de_sesiones_bytes, 0,
        "el punto de control debe consolidar el WAL a cero bytes"
    );

    let tamano_despues = std::fs::metadata(&ruta_wal)
        .map(|metadatos| metadatos.len())
        .unwrap_or(0);
    assert_eq!(
        tamano_despues, 0,
        "el archivo -wal debe quedar en cero bytes con los pools todavía abiertos"
    );

    // Los datos siguen siendo legibles: el punto de control consolida, no destruye.
    let cuenta: i64 = gestor
        .sesiones()
        .con_lectura(|conexion| {
            conexion
                .query_row("SELECT count(*) FROM estado_del_motor", [], |fila| {
                    fila.get(0)
                })
                .map_err(|causa| ErrorDeAlmacen::Sqlite {
                    operacion: "contar filas de prueba",
                    causa,
                })
        })
        .expect("las filas deben seguir siendo legibles tras el punto de control");
    assert_eq!(cuenta, 200);
}

#[test]
fn abrir_sobre_una_ruta_que_no_es_un_directorio_falla_sin_panico() {
    let directorio = DirectorioTemporal::nuevo("pools-ruta-invalida");
    let archivo = directorio.ruta().join("no-soy-un-directorio");
    std::fs::write(&archivo, b"contenido").expect("crear el archivo de prueba");

    let resultado = GestorDePools::abrir(&archivo);
    assert!(
        resultado.is_err(),
        "una ruta que no es directorio debe fallar"
    );
}

#[test]
fn verificar_guarda_3_enlace_vivo_colgante_en_abrir_falla_sin_crear_base_vacia() {
    let directorio = DirectorioTemporal::nuevo("pools-enlace-colgante");
    let ruta_live = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let destino_inexistente = directorio.ruta().join("knowledge_epoch_desaparecida.db");

    // Crear un enlace simbólico colgante apuntando a un archivo que no existe en disco
    std::os::unix::fs::symlink(&destino_inexistente, &ruta_live)
        .expect("crear enlace simbólico colgante");

    // 1. GestorDePools::abrir (lectura y escritura) debe abortar con EnlaceVivoColgante antes de abrir la base
    let resultado_abrir = GestorDePools::abrir(directorio.ruta());
    match resultado_abrir {
        Err(ErrorDeAlmacen::EnlaceVivoColgante { ruta, destino }) => {
            assert_eq!(ruta, ruta_live);
            assert_eq!(destino, destino_inexistente);
        }
        Err(error) => panic!("se esperaba Err(EnlaceVivoColgante), se obtuvo: {error:?}"),
        Ok(_) => panic!("se esperaba Err(EnlaceVivoColgante), se obtuvo Ok"),
    }

    // Aserción negativa fundamental: el archivo destino no debe haber sido creado como base vacía de 40960 bytes
    assert!(
        !destino_inexistente.exists(),
        "el destino inexistente jamás debe ser creado por el camino de apertura"
    );

    // 2. Contrapartida: la apertura en solo lectura sobre el mismo enlace colgante falla limpiamente sin crear nada
    let resultado_solo_lectura = hexcell_storage::PoolDeConocimiento::abrir_sobre(&ruta_live);
    assert!(
        resultado_solo_lectura.is_err(),
        "la apertura en solo lectura debe fallar sobre un enlace colgante"
    );
    assert!(
        !destino_inexistente.exists(),
        "la apertura en solo lectura no debe crear ningún archivo en el destino"
    );
}

#[test]
fn verificar_ac7_anchura_de_pool_parametrizable() {
    let directorio = DirectorioTemporal::nuevo("pools-anchura-ac7");
    let gestor_defecto = GestorDePools::abrir(directorio.ruta()).expect("abrir gestor por defecto");

    // La anchura por omisión es 2
    assert_eq!(gestor_defecto.anchura_de_lecturas_de_conocimiento(), 2);
    assert_eq!(gestor_defecto.conocimiento().anchura_de_lecturas(), 2);

    let ruta_conocimiento = directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);

    // abrir_sobre conserva la anchura 2 por omisión
    let pool_defecto = PoolDeConocimiento::abrir_sobre(&ruta_conocimiento).expect("abrir_sobre");
    assert_eq!(pool_defecto.anchura_de_lecturas(), 2);

    // abrir_sobre_con_anchura(ruta, 5) construye un pool de anchura 5
    let pool_ancho =
        PoolDeConocimiento::abrir_sobre_con_anchura(&ruta_conocimiento, 5).expect("abrir 5");
    assert_eq!(pool_ancho.anchura_de_lecturas(), 5);

    // abrir_sobre_con_anchura(ruta, 0) falla de inmediato con PoolDeConocimientoVacio
    let res_cero = PoolDeConocimiento::abrir_sobre_con_anchura(&ruta_conocimiento, 0);
    assert!(matches!(
        res_cero,
        Err(ErrorDeAlmacen::PoolDeConocimientoVacio)
    ));

    // GestorDePools::abrir_con_anchura_de_conocimiento(ruta, 5) configura la anchura 5
    let dir2 = DirectorioTemporal::nuevo("pools-anchura-gestor-ac7");
    let gestor_ancho = GestorDePools::abrir_con_anchura_de_conocimiento(dir2.ruta(), 5)
        .expect("abrir gestor ancho");
    assert_eq!(gestor_ancho.anchura_de_lecturas_de_conocimiento(), 5);
    assert_eq!(gestor_ancho.conocimiento().anchura_de_lecturas(), 5);
}

#[test]
fn verificar_ac7_anchura_configurada_sobrevive_a_promocion_y_reversion() {
    use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
    use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
    use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
    use hexcell_storage::promocion::promover_epoca;
    use hexcell_storage::reversion::revertir_a_epoca;

    let dir = DirectorioTemporal::nuevo("pools-anchura-conmutacion-ac7");
    let gestor = GestorDePools::abrir_con_anchura_de_conocimiento(dir.ruta(), 5)
        .expect("abrir gestor con 5");
    assert_eq!(gestor.conocimiento().anchura_de_lecturas(), 5);

    // Crear base de staging válida
    let ruta_staging = dir.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).unwrap();
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).unwrap();
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = 4 WHERE id = 1",
            [],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO documentos VALUES (1, 'ref1', 't', 'c', 1000)",
            [],
        )
        .unwrap();
    let bytes = vec![1.0f32; 4]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    conexion
        .execute("INSERT INTO fragmentos VALUES (1, 1, 0, 't')", [])
        .unwrap();
    conexion
        .execute(
            "INSERT INTO vectores_de_fragmento VALUES (1, ?1)",
            rusqlite::params![bytes],
        )
        .unwrap();
    conexion
        .execute(
            "INSERT INTO sonda_semantica VALUES (1, 'c', ?1, 0.5, 1000)",
            rusqlite::params![bytes],
        )
        .unwrap();
    drop(conexion);

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 1,
        solapamiento: 0,
    };
    promover_epoca(&gestor, dir.ruta(), &config, 10_000).expect("promover época");

    // Tras la promoción, el pool recién instalado debe conservar la anchura 5
    assert_eq!(gestor.conocimiento().anchura_de_lecturas(), 5);

    // Revertir a la época 1
    revertir_a_epoca(&gestor, dir.ruta(), &config, 1).expect("revertir a época 1");

    // Tras la reversión, el pool recién instalado también conserva la anchura 5
    assert_eq!(gestor.conocimiento().anchura_de_lecturas(), 5);
}
