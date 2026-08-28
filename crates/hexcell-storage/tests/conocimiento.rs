//! Pruebas de integración del constructor de conocimiento en sombra.
//!
//! Estos tests verifican el ciclo de vida síncrono del archivo de persistencia en sombra,
//! incluyendo el borrado incondicional antes del inicio de una ingesta, la aplicación
//! de restricciones de integridad referencial y cascada de SQLite, y la serialización de vectores.
//!
//! Diseñado el 28 de agosto de 2026 para robustecer la capa de persistencia.

mod comun;

use comun::DirectorioTemporal;
use hexcell_storage::pools::SUFIJO_DE_ARCHIVO_WAL;
use hexcell_storage::{
    ConstructorDeConocimientoEnSombra, DocumentoDeIngesta,
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA, SUFIJO_DE_ARCHIVO_SHM,
    VERSION_DE_ESQUEMA_DE_CONOCIMIENTO,
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
