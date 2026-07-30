//! Tests del respaldo en caliente (AC-1, AC-2): las copias, su integridad, sus destinos y su
//! comportamiento bajo un escritor concurrente.
//!
//! Cubre las DOS bases que `GestorDePools::respaldar_en` alcanza (`sessions.db` y
//! `knowledge_live.db`); la tercera base alcanzable desde esta etapa, el almacén de identidad del
//! adaptador, tiene su propia batería en `tests/almacen_de_identidad.rs`, porque vive en su propio
//! tipo con su propio método `respaldar_en`.

mod comun;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use comun::DirectorioTemporal;
use hexcell_storage::{
    ErrorDeAlmacen, GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_SESIONES,
};
use rusqlite::Connection;

#[test]
fn el_respaldo_produce_las_dos_copias_de_pools_intactas_y_verificadas() {
    let directorio = DirectorioTemporal::nuevo("respaldo-copias");
    let destino = DirectorioTemporal::nuevo("respaldo-copias-destino");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los pools");

    let resumen = gestor
        .respaldar_en(destino.ruta())
        .expect("el respaldo de los dos pools no debe fallar");
    assert_eq!(resumen.copias.len(), 2);

    for nombre in [
        NOMBRE_DE_ARCHIVO_DE_SESIONES,
        NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO,
    ] {
        let ruta = destino.ruta().join(nombre);
        assert!(ruta.is_file(), "la copia de {nombre} debe existir");
        assert!(
            std::fs::metadata(&ruta)
                .expect("leer metadatos de la copia")
                .len()
                > 0,
            "la copia de {nombre} no puede estar vacía"
        );

        let conexion =
            Connection::open_with_flags(&ruta, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap_or_else(|error| {
                    panic!("la copia de {nombre} debe abrir como SQLite: {error}")
                });
        let integridad: String = conexion
            .query_row("PRAGMA integrity_check", [], |fila| fila.get(0))
            .expect("ejecutar integrity_check sobre la copia");
        assert_eq!(integridad, "ok");
    }
}

#[test]
fn cada_copia_conserva_su_version_de_esquema() {
    let directorio = DirectorioTemporal::nuevo("respaldo-version");
    let destino = DirectorioTemporal::nuevo("respaldo-version-destino");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los pools");

    let resumen = gestor
        .respaldar_en(destino.ruta())
        .expect("el respaldo no debe fallar");

    for copia in &resumen.copias {
        let conexion =
            Connection::open_with_flags(&copia.ruta, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("abrir la copia");
        let version: i64 = conexion
            .query_row("PRAGMA user_version", [], |fila| fila.get(0))
            .expect("leer user_version de la copia");
        assert_eq!(
            version, 1,
            "la copia de {} debe conservar la versión de esquema de su origen",
            copia.nombre_logico
        );
    }
}

#[test]
fn un_destino_que_no_existe_falla_con_su_propio_error_sin_dejar_nada_a_medias() {
    let directorio = DirectorioTemporal::nuevo("respaldo-destino-inexistente");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los pools");

    let destino_inexistente = directorio.ruta().join("no-existe").join("tampoco");
    let resultado = gestor.respaldar_en(&destino_inexistente);
    assert!(matches!(
        resultado,
        Err(ErrorDeAlmacen::DirectorioDeRespaldoInaccesible { .. })
    ));
}

#[test]
fn un_destino_ya_ocupado_falla_con_su_propio_error_y_no_toca_al_segundo() {
    let directorio = DirectorioTemporal::nuevo("respaldo-destino-ocupado");
    let destino = DirectorioTemporal::nuevo("respaldo-destino-ocupado-destino");
    let gestor = GestorDePools::abrir(directorio.ruta()).expect("abrir los pools");

    // Ocupa de antemano el destino de la PRIMERA copia que intentará el gestor.
    std::fs::write(
        destino.ruta().join(NOMBRE_DE_ARCHIVO_DE_SESIONES),
        b"ocupado",
    )
    .expect("escribir el archivo que ocupa el destino");

    let resultado = gestor.respaldar_en(destino.ruta());
    assert!(matches!(
        resultado,
        Err(ErrorDeAlmacen::DestinoDeRespaldoOcupado { .. })
    ));

    // La ronda entera falla antes de tocar el segundo destino: no queda ninguna copia parcial.
    assert!(
        !destino
            .ruta()
            .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO)
            .exists(),
        "un destino ocupado no debe dejar ninguna otra copia a medias"
    );
}

#[test]
fn el_respaldo_no_produce_sqlite_busy_con_un_escritor_concurrente_activo() {
    let directorio = DirectorioTemporal::nuevo("respaldo-concurrencia");
    let destino = DirectorioTemporal::nuevo("respaldo-concurrencia-destino");
    let gestor = Arc::new(GestorDePools::abrir(directorio.ruta()).expect("abrir los pools"));

    let detener = Arc::new(AtomicBool::new(false));

    let gestor_escritor = Arc::clone(&gestor);
    let detener_escritor = Arc::clone(&detener);
    let hilo_escritor = std::thread::spawn(move || {
        let mut indice = 0i64;
        while !detener_escritor.load(Ordering::Relaxed) {
            let resultado = gestor_escritor.sesiones().con_escritura(|conexion| {
                conexion
                    .execute(
                        "INSERT INTO estado_del_motor (clave, valor) \
                         VALUES (?1, ?2) \
                         ON CONFLICT(clave) DO UPDATE SET valor = excluded.valor",
                        rusqlite::params![format!("clave-concurrencia-{indice}"), indice],
                    )
                    .map_err(|causa| ErrorDeAlmacen::Sqlite {
                        operacion: "escribir en el hilo de escritura concurrente",
                        causa,
                    })
            });
            assert!(
                !es_sqlite_busy(&resultado),
                "el escritor concurrente no debe ver SQLITE_BUSY: {resultado:?}"
            );
            indice += 1;
        }
    });

    // Deja que el escritor arranque antes de disparar el respaldo.
    std::thread::sleep(Duration::from_millis(20));

    let resultado_del_respaldo = gestor.respaldar_en(destino.ruta());

    detener.store(true, Ordering::Relaxed);
    hilo_escritor
        .join()
        .expect("el hilo escritor no debe entrar en pánico");

    assert!(
        !es_sqlite_busy(&resultado_del_respaldo),
        "el respaldo no debe devolver SQLITE_BUSY: {resultado_del_respaldo:?}"
    );
    resultado_del_respaldo.expect("el respaldo debe completarse con el escritor activo");

    // La base de origen sigue sana tras el respaldo concurrente.
    assert_eq!(
        gestor.sesiones().vitalidad(),
        hexcell_storage::Vitalidad::Sana
    );
}

/// Comprueba, sin asumir el tipo exacto de error, que un resultado no lleva el código
/// `SQLITE_BUSY` de la biblioteca subyacente.
fn es_sqlite_busy<T>(resultado: &Result<T, ErrorDeAlmacen>) -> bool {
    match resultado {
        Err(ErrorDeAlmacen::Sqlite { causa, .. }) => {
            matches!(
                causa.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            )
        }
        _ => false,
    }
}
