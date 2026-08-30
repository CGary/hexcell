//! Pruebas de integración del constructor de conocimiento en sombra.
//!
//! Estos tests verifican el ciclo de vida síncrono del archivo de persistencia en sombra,
//! incluyendo el borrado incondicional antes del inicio de una ingesta, la aplicación
//! de restricciones de integridad referencial y cascada de SQLite, y la serialización de vectores.
//!
//! Diseñado el 28 de agosto de 2026 para robustecer la capa de persistencia.

mod comun;

use comun::DirectorioTemporal;
use hexcell_core::embeddings::VectorDeEmbedding;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::pools::SUFIJO_DE_ARCHIVO_WAL;
use hexcell_storage::validacion::{
    MotivoDeRechazo, SondaResuelta, VeredictoDeIntegridad, validar_integridad_del_indice,
};
use hexcell_storage::{
    ConstructorDeConocimientoEnSombra, DocumentoDeIngesta, ErrorDeAlmacen,
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA, SUFIJO_DE_ARCHIVO_SHM,
    VERSION_DE_ESQUEMA_DE_CONOCIMIENTO, aplicar_migraciones_de_conocimiento, leer_sonda_semantica,
};
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

#[test]
fn verificar_reconstruccion_limpia_y_borrado_de_residuos() {
    let temporal = DirectorioTemporal::nuevo("reconstruccion-conocimiento");
    let ruta_datos = temporal.ruta();
    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);

    let mut ruta_wal_os = ruta_base.as_os_str().to_owned();
    ruta_wal_os.push(SUFIJO_DE_ARCHIVO_WAL);
    let ruta_wal = PathBuf::from(ruta_wal_os);

    let mut ruta_shm_os = ruta_base.as_os_str().to_owned();
    ruta_shm_os.push(SUFIJO_DE_ARCHIVO_SHM);
    let ruta_shm = PathBuf::from(ruta_shm_os);

    // Se simula un residuo de una ejecución fallida previa escribiendo archivos basura.
    fs::write(&ruta_base, b"datos obsoletos").unwrap();
    fs::write(&ruta_wal, b"wal obsoleto").unwrap();
    fs::write(&ruta_shm, b"shm obsoleto").unwrap();

    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-1".to_string(),
        titulo: "Documento 1".to_string(),
        contenido: "Texto de prueba".to_string(),
        actualizado_ms: 1724800000,
    };

    // Al crear un nuevo constructor, se debe limpiar todo vestigio anterior.
    let constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();

    // La base existe y ya NO es la basura previa: se comprueba por contenido, porque un
    // `exists()` seguiría siendo cierto sobre el archivo obsoleto sin reconstruir.
    assert!(ruta_base.exists());
    let cabecera = fs::read(&ruta_base).unwrap();
    assert_ne!(
        &cabecera[..],
        b"datos obsoletos",
        "La base debe haberse reconstruido, no conservarse tal cual"
    );
    assert!(
        cabecera.starts_with(b"SQLite format 3\0"),
        "La base reconstruida debe ser un archivo SQLite valido"
    );

    // Los residuos de la corrida anterior no pueden sobrevivir. No se afirma que los archivos
    // auxiliares no existan —SQLite crea los suyos mientras la conexion esta viva, y afirmarlo
    // aqui seria falso—, sino que su CONTENIDO ya no es el heredado.
    if ruta_wal.exists() {
        assert_ne!(
            &fs::read(&ruta_wal).unwrap()[..],
            b"wal obsoleto",
            "El WAL heredado debe haberse borrado, no reutilizado"
        );
    }
    if ruta_shm.exists() {
        assert_ne!(
            &fs::read(&ruta_shm).unwrap()[..],
            b"shm obsoleto",
            "El SHM heredado debe haberse borrado, no reutilizado"
        );
    }

    // Finalizamos para liberar conexiones y poder leer el archivo.
    constructor.finalizar().unwrap();

    // Tras un cierre limpio no puede quedar ningun auxiliar huerfano: ese es justamente el
    // fallo que la etapa A-5 existe para evitar, porque un -wal suelto corrompe al siguiente
    // lector que abra la base.
    assert!(
        !ruta_wal.exists(),
        "Tras finalizar no debe quedar un archivo -wal huerfano"
    );
    assert!(
        !ruta_shm.exists(),
        "Tras finalizar no debe quedar un archivo -shm huerfano"
    );
}

#[test]
fn verificar_pragmas_e_integridad_referencial_y_cascada() {
    let temporal = DirectorioTemporal::nuevo("pragmas-conocimiento");
    let ruta_datos = temporal.ruta();
    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-2".to_string(),
        titulo: "Documento 2".to_string(),
        contenido: "Texto a trocear".to_string(),
        actualizado_ms: 1724800000,
    };

    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();

    // Escribimos algunos fragmentos.
    let lote = vec![
        (0, "Frase uno".to_string(), vec![0.1f32, 0.2f32, 0.3f32]),
        (1, "Frase dos".to_string(), vec![0.4f32, 0.5f32, 0.6f32]),
    ];
    constructor.escribir_lote_de_fragmentos(&lote).unwrap();
    constructor.finalizar().unwrap();

    // Se abre una conexión propia del test, tal como hace migraciones.rs, en vez de alcanzar
    // el campo privado del constructor: la frontera de encapsulación no se rompe para inspeccionar.
    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion_de_inspeccion =
        Connection::open(&ruta_base).expect("abrir la base en sombra ya construida");

    // Se comprueba de forma explícita que la conexión tenga activos los pragmas obligatorios.
    // PRAGMA foreign_keys = 1 asegura restricciones de integridad referencial activas.
    // PRAGMA user_version asegura que estamos en el esquema v2 de conocimiento.
    let foreign_keys: i64 = conexion_de_inspeccion
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    let user_version: i64 = conexion_de_inspeccion
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();

    assert_eq!(foreign_keys, 1, "La pragma foreign_keys debe estar activa");
    assert_eq!(
        user_version, VERSION_DE_ESQUEMA_DE_CONOCIMIENTO,
        "La base debe estar migrada a la versión correcta"
    );

    // Se verifica que existan las filas correspondientes en las tablas de fragmentos y vectores.
    let cant_fragmentos: i64 = conexion_de_inspeccion
        .query_row("SELECT COUNT(*) FROM fragmentos", [], |r| r.get(0))
        .unwrap();
    let cant_vectores: i64 = conexion_de_inspeccion
        .query_row("SELECT COUNT(*) FROM vectores_de_fragmento", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(cant_fragmentos, 2);
    assert_eq!(cant_vectores, 2);

    // Se comprueba que el borrado en cascada funcione al eliminar el documento original.
    conexion_de_inspeccion
        .execute("DELETE FROM documentos", [])
        .unwrap();

    let cant_fragmentos_post: i64 = conexion_de_inspeccion
        .query_row("SELECT COUNT(*) FROM fragmentos", [], |r| r.get(0))
        .unwrap();
    let cant_vectores_post: i64 = conexion_de_inspeccion
        .query_row("SELECT COUNT(*) FROM vectores_de_fragmento", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        cant_fragmentos_post, 0,
        "Los fragmentos debieron borrarse en cascada"
    );
    assert_eq!(
        cant_vectores_post, 0,
        "Los vectores debieron borrarse en cascada"
    );
}

#[test]
fn verificar_ida_y_vuelta_de_vectores_en_little_endian() {
    let temporal = DirectorioTemporal::nuevo("endian-conocimiento");
    let ruta_datos = temporal.ruta();
    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-3".to_string(),
        titulo: "Documento 3".to_string(),
        contenido: "Contenido para embeddings".to_string(),
        actualizado_ms: 1724800000,
    };

    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();

    // Se definen f32s específicos para verificar que los bits no sufran alteraciones al serializarse.
    let vector_original = vec![1.5f32, -2.75f32, 3.125f32, 0.0f32];
    let lote = vec![(0, "Fragmento único".to_string(), vector_original.clone())];
    constructor.escribir_lote_de_fragmentos(&lote).unwrap();
    constructor.finalizar().unwrap();

    // Se abre una conexión propia del test, tal como hace migraciones.rs, en vez de alcanzar
    // el campo privado del constructor: la frontera de encapsulación no se rompe para inspeccionar.
    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion_de_inspeccion =
        Connection::open(&ruta_base).expect("abrir la base en sombra ya construida");

    // Se lee el BLOB de vectores crudo para comprobar que tenga exactamente 4 bytes por cada f32.
    let blob_bytes: Vec<u8> = conexion_de_inspeccion
        .query_row(
            "SELECT vector FROM vectores_de_fragmento LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(blob_bytes.len(), vector_original.len() * 4);

    // Reconstruimos los f32 del BLOB asumiendo little-endian y verificamos igualdad exacta.
    let mut valores_leídos = Vec::new();
    for octeto_de_cuatro in blob_bytes.chunks_exact(4) {
        let mut arr = [0u8; 4];
        arr.copy_from_slice(octeto_de_cuatro);
        valores_leídos.push(f32::from_le_bytes(arr));
    }

    assert_eq!(valores_leídos, vector_original);
}

#[test]
fn verificar_registro_de_sonda_semantica_y_redondeo_de_bytes() {
    let temporal = DirectorioTemporal::nuevo("sonda-registro");
    let ruta_datos = temporal.ruta();
    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-sonda".to_string(),
        titulo: "Documento Sonda".to_string(),
        contenido: "Texto con contenido de prueba".to_string(),
        actualizado_ms: 1724800000,
    };

    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();

    let texto_sonda = "¿Cuál es el resumen del texto?";
    let vector_sonda = vec![0.25f32, -0.75f32, 1.5f32, 0.0f32];
    let umbral = 0.82f32;
    let marca_ms = 1724800500i64;

    constructor
        .registrar_sonda_semantica(texto_sonda, &vector_sonda, umbral, marca_ms)
        .unwrap();

    let lote = vec![(
        0,
        "Unico fragmento".to_string(),
        vec![0.1f32, 0.2f32, 0.3f32, 0.4f32],
    )];
    constructor.escribir_lote_de_fragmentos(&lote).unwrap();
    constructor.finalizar().unwrap();

    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_base).expect("abrir la base en sombra construida");

    let (id, texto_recuperado, vector_bytes, umbral_recuperado, marca_recuperada): (
        i64,
        String,
        Vec<u8>,
        f64,
        i64,
    ) = conexion
        .query_row(
            "SELECT id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms FROM sonda_semantica WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();

    assert_eq!(id, 1);
    assert_eq!(texto_recuperado, texto_sonda);
    assert_eq!(umbral_recuperado as f32, umbral);
    assert_eq!(marca_recuperada, marca_ms);

    let vector_esperado_bytes = VectorDeEmbedding::nuevo(vector_sonda.clone()).a_bytes_le();
    assert_eq!(vector_bytes, vector_esperado_bytes);

    let vector_decodificado = VectorDeEmbedding::desde_bytes_le(&vector_bytes).unwrap();
    assert_eq!(vector_decodificado.valores(), &vector_sonda[..]);
}

#[test]
fn verificar_descarte_de_sonda_semantica_cuando_no_hay_incrustaciones() {
    let temporal = DirectorioTemporal::nuevo("sonda-descarte-cero");
    let ruta_datos = temporal.ruta();
    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-descarte".to_string(),
        titulo: "Documento Descarte".to_string(),
        contenido: "Texto sin vectores resueltos".to_string(),
        actualizado_ms: 1724800000,
    };

    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();

    constructor
        .registrar_sonda_semantica("Sonda solitaria", &[0.1, 0.2, 0.3, 0.4], 0.75, 1724800600)
        .unwrap();

    // Finalizar sin haber escrito ningún fragmento con vector
    constructor.finalizar().unwrap();

    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_base).expect("abrir la base en sombra construida");

    let filas_sonda: i64 = conexion
        .query_row("SELECT COUNT(*) FROM sonda_semantica", [], |r| r.get(0))
        .unwrap();
    let filas_epoca: i64 = conexion
        .query_row("SELECT COUNT(*) FROM metadatos_de_epoca", [], |r| r.get(0))
        .unwrap();

    assert_eq!(
        filas_sonda, 0,
        "La sonda semántica debe descartarse si no hubo fragmentos con vector"
    );
    assert_eq!(
        filas_epoca, 0,
        "Los metadatos de época deben descartarse si no hubo fragmentos con vector"
    );
}

#[test]
fn verificar_lectura_de_sonda_semantica_existente_y_ausente() {
    let temporal = DirectorioTemporal::nuevo("sonda-lectura");
    let ruta_datos = temporal.ruta();

    // Caso 1: Archivo con sonda semántica persistida
    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-leible".to_string(),
        titulo: "Documento Leíble".to_string(),
        contenido: "Texto completo".to_string(),
        actualizado_ms: 1724800000,
    };
    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();
    let vector_sonda = vec![0.1f32, 0.2f32, 0.3f32, 0.4f32];
    constructor
        .registrar_sonda_semantica("Consulta de validación", &vector_sonda, 0.9, 1724800700)
        .unwrap();
    let lote = vec![(0, "Frase".to_string(), vec![0.5f32, 0.6f32, 0.7f32, 0.8f32])];
    constructor.escribir_lote_de_fragmentos(&lote).unwrap();
    constructor.finalizar().unwrap();

    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let sonda_leida = leer_sonda_semantica(&ruta_base).unwrap();
    assert!(sonda_leida.is_some());
    let sonda = sonda_leida.unwrap();
    assert_eq!(sonda.vector, vector_sonda);
    assert_eq!(sonda.umbral_de_aceptacion, 0.9f32);

    // Caso 2: Archivo recién migrado en versión 3 sin fila de sonda
    let temp_vacio = DirectorioTemporal::nuevo("sonda-lectura-vacia");
    let ruta_vacia = temp_vacio.ruta().join("knowledge_live.db");
    let conexion_vacia = Connection::open(&ruta_vacia).unwrap();
    aplicar_migraciones_de_conocimiento(&conexion_vacia).unwrap();
    drop(conexion_vacia);

    let sonda_vacia = leer_sonda_semantica(&ruta_vacia).unwrap();
    assert!(
        sonda_vacia.is_none(),
        "Una base sin fila en sonda_semantica debe devolver Ok(None)"
    );

    // Caso 3: Archivo inexistente debe devolver Err
    let ruta_inexistente = temp_vacio.ruta().join("archivo_que_no_existe.db");
    let resultado_error = leer_sonda_semantica(&ruta_inexistente);
    assert!(
        resultado_error.is_err(),
        "Abrir un archivo inexistente debe fallar"
    );
}

#[test]
fn verificar_error_al_leer_sonda_semantica_con_blob_corrupto() {
    let temporal = DirectorioTemporal::nuevo("sonda-corrupta");
    let ruta_db = temporal.ruta().join("base_corrupta.db");
    let conexion = Connection::open(&ruta_db).unwrap();

    // Crear tabla sin la restricción CHECK de múltiplo de 4 para forzar la presencia de un BLOB dañado
    conexion
        .execute_batch(
            "CREATE TABLE sonda_semantica (
                id INTEGER PRIMARY KEY,
                texto_de_la_sonda TEXT NOT NULL,
                vector BLOB NOT NULL,
                umbral_de_aceptacion REAL NOT NULL,
                registrada_ms INTEGER NOT NULL
            );
            INSERT INTO sonda_semantica (id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms)
            VALUES (1, 'Texto', X'0102030405', 0.8, 1000);",
        )
        .unwrap();
    drop(conexion);

    let resultado = leer_sonda_semantica(&ruta_db);
    assert!(resultado.is_err());
    match resultado {
        Err(ErrorDeAlmacen::SondaSemanticaIlegible { ruta, motivo }) => {
            assert_eq!(ruta, ruta_db);
            assert!(!motivo.is_empty());
        }
        other => panic!("Se esperaba SondaSemanticaIlegible pero se obtuvo: {other:?}"),
    }
}

#[test]
fn verificar_entrega_de_sonda_leida_al_validador_de_integridad() {
    let temporal = DirectorioTemporal::nuevo("sonda-validador-handoff");
    let ruta_datos = temporal.ruta();
    let doc = DocumentoDeIngesta {
        referencia_externa: "https://ejemplo.com/doc-val".to_string(),
        titulo: "Documento Val".to_string(),
        contenido: "Contenido para validar".to_string(),
        actualizado_ms: 1724800000,
    };

    let mut constructor = ConstructorDeConocimientoEnSombra::crear(ruta_datos, &doc).unwrap();

    // Sonda con vector de dimensión 4 y similitud alta esperada
    let vector_sonda = vec![1.0f32, 0.0f32, 0.0f32, 0.0f32];
    constructor
        .registrar_sonda_semantica("Consulta", &vector_sonda, 0.5, 1724800800)
        .unwrap();

    let lote = vec![
        (
            0,
            "Contenido para".to_string(),
            vec![1.0f32, 0.0f32, 0.0f32, 0.0f32],
        ),
        (
            1,
            " validar".to_string(),
            vec![0.9f32, 0.1f32, 0.0f32, 0.0f32],
        ),
    ];
    constructor.escribir_lote_de_fragmentos(&lote).unwrap();
    constructor.finalizar().unwrap();

    let ruta_base = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let sonda_opcional = leer_sonda_semantica(&ruta_base).unwrap();
    assert!(sonda_opcional.is_some());
    let sonda = sonda_opcional.expect("sonda presente en la base de datos");

    let config = ConfiguracionDeFragmentacion {
        tamano_de_fragmento: 14,
        solapamiento: 0,
    };

    // Validación exitosa con la sonda leída directamente del archivo
    let veredicto = validar_integridad_del_indice(&ruta_base, &config, &sonda).unwrap();
    assert!(
        matches!(veredicto, VeredictoDeIntegridad::Aprobado { .. }),
        "El índice debe aprobarse con la sonda leída del archivo"
    );

    // Caso de sonda con dimensión discrepante (dimensión 8 vs dimensión de época 4)
    let sonda_discrepante = SondaResuelta {
        vector: vec![0.1f32; 8],
        umbral_de_aceptacion: 0.5,
    };
    let veredicto_discrepante =
        validar_integridad_del_indice(&ruta_base, &config, &sonda_discrepante).unwrap();
    assert!(
        matches!(
            veredicto_discrepante,
            VeredictoDeIntegridad::Rechazado { ref motivos }
                if motivos.contains(&MotivoDeRechazo::DimensionDeLaSondaDiscrepante {
                    dimension_sonda: 8,
                    dimension_epoca: 4,
                })
        ),
        "Una sonda con dimensión diferente a la época debe ser rechazada por el validador"
    );
}
