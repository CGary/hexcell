//! Pruebas externas del almacén del plano de control.
//!
//! Cada prueba reserva su propia base temporal —que su `Drop` borra— y verifica esquema,
//! migración y operaciones. Dos reglas las hacen capaces de ponerse rojas: el esquema se aserta
//! COLUMNA a columna con `PRAGMA table_info`, no por el nombre de las tablas; y ninguna ruta de
//! accesorio es un literal fijo como `/tmp/algo-12345`, que puede existir de verdad en la máquina
//! que corre la prueba y hacerla pasar por el motivo equivocado.

mod comun;

use hexcell_admin::almacen_plano_de_control::{
    AlmacenDelPlanoDeControl, ErrorDeAlmacenDePlano, MOTIVO_DE_ALTA_IMPLICITA,
    MOTIVO_DE_EMPAREJAMIENTO_CONFIRMADO, VERSION_DE_ESQUEMA_DEL_PLANO, estado_desde_etiqueta,
    etiqueta_persistida,
};
use hexcell_admin::estado_de_celula::EstadoDeCelula;

use comun::{AlmacenTemporal, ruta_directorio_inexistente};

/// Conexión de sólo lectura propia de la prueba: el almacén no presta la suya, para que la
/// conexión de escritura no se filtre a la API pública con la excusa de las pruebas.
fn lector(almacen: &AlmacenTemporal) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        almacen.ruta(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("abrir la base temporal en sólo lectura")
}

fn tablas(conexion: &rusqlite::Connection) -> Vec<String> {
    let mut s = conexion
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let v = s
        .query_map([], |f| f.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    v
}

/// Columnas de `tabla` como `«nombre TIPO nn=N def=X pk=N»`, una cadena por columna.
fn columnas(conexion: &rusqlite::Connection, tabla: &str) -> Vec<String> {
    let mut s = conexion
        .prepare(&format!("PRAGMA table_info({tabla})"))
        .unwrap();
    let v = s
        .query_map([], |f| {
            Ok(format!(
                "{} {} nn={} def={} pk={}",
                f.get::<_, String>(1)?,
                f.get::<_, String>(2)?,
                f.get::<_, i64>(3)?,
                f.get::<_, Option<String>>(4)?.unwrap_or_default(),
                f.get::<_, i64>(5)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    v
}

/// Filas de `transiciones` como `«id de>a motivo ms»`.
fn transiciones(conexion: &rusqlite::Connection) -> Vec<String> {
    let mut s = conexion
        .prepare("SELECT id_celula, de, a, motivo, registrado_ms FROM transiciones ORDER BY id")
        .unwrap();
    let v = s
        .query_map([], |f| {
            Ok(format!(
                "{} {}>{} {} {}",
                f.get::<_, String>(0)?,
                f.get::<_, String>(1)?,
                f.get::<_, String>(2)?,
                f.get::<_, String>(3)?,
                f.get::<_, i64>(4)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    v
}

/// AC-1: abrir contra un archivo vacío crea las tres tablas y fija la versión de esquema.
#[test]
fn abrir_contra_archivo_vacio_crea_las_tres_tablas_con_la_version() {
    let temporal = AlmacenTemporal::nuevo("abrir-vacio");
    drop(AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("abrir el almacén"));
    let conexion = lector(&temporal);
    assert_eq!(
        tablas(&conexion),
        ["celulas", "sustituciones", "transiciones"]
    );
    let version: i64 = conexion
        .query_row("PRAGMA user_version", [], |f| f.get(0))
        .unwrap();
    assert_eq!(version, VERSION_DE_ESQUEMA_DEL_PLANO);
}

/// AC-1: el esquema se aserta COLUMNA a columna, no por el nombre de las tablas.
///
/// Las tres listas se escriben enteras aquí, sin leer ninguna constante de producción: añadir una
/// columna, renombrarla, cambiarle el tipo, quitarle el `NOT NULL` o cambiar el valor por omisión
/// de `motivo` pone roja exactamente una de estas tres aserciones.
#[test]
fn las_tres_tablas_declaran_exactamente_sus_columnas() {
    let temporal = AlmacenTemporal::nuevo("columnas");
    drop(AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("abrir el almacén"));
    let conexion = lector(&temporal);
    assert_eq!(
        columnas(&conexion, "celulas"),
        [
            "id TEXT nn=0 def= pk=1",
            "estado TEXT nn=1 def= pk=0",
            "motivo TEXT nn=1 def='' pk=0",
            "actualizado_ms INTEGER nn=1 def= pk=0",
        ]
    );
    assert_eq!(
        columnas(&conexion, "transiciones"),
        [
            "id INTEGER nn=0 def= pk=1",
            "id_celula TEXT nn=1 def= pk=0",
            "de TEXT nn=1 def= pk=0",
            "a TEXT nn=1 def= pk=0",
            "motivo TEXT nn=1 def= pk=0",
            "registrado_ms INTEGER nn=1 def= pk=0",
        ]
    );
    assert_eq!(
        columnas(&conexion, "sustituciones"),
        [
            "id INTEGER nn=0 def= pk=1",
            "id_celula TEXT nn=1 def= pk=0",
            "motivo TEXT nn=1 def= pk=0",
            "registrado_ms INTEGER nn=1 def= pk=0",
        ]
    );
}

/// AC-1: reabrir el mismo archivo es una operación nula que devuelve Ok.
#[test]
fn reabrir_el_mismo_archivo_es_no_op() {
    let temporal = AlmacenTemporal::nuevo("reabrir");
    let _a = AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("primera apertura");
    let _b = AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("segunda apertura");
}

/// AC-1: bajo un directorio ausente el rechazo es la VARIANTE tipada, su `Display` nombra el
/// directorio y el directorio sigue ausente.
///
/// Las tres aserciones son necesarias. Un `is_err()` a secas es una guarda vacua: sin la
/// comprobación del directorio padre, SQLite falla igualmente al crear el archivo y la prueba
/// seguiría verde sobre código sin la guarda.
#[test]
fn abrir_bajo_directorio_inexistente_devuelve_error_tipado_y_no_lo_crea() {
    let directorio = ruta_directorio_inexistente("abrir-sin-directorio");
    assert!(!directorio.exists(), "el accesorio empieza sin directorio");
    let ruta = directorio.join("plano_de_control.db");

    let error = AlmacenDelPlanoDeControl::abrir(&ruta)
        .err()
        .expect("abrir bajo un directorio ausente tiene que fallar");

    assert!(
        matches!(&error, ErrorDeAlmacenDePlano::DirectorioInaccesible { directorio: d } if d == &directorio),
        "se esperaba DirectorioInaccesible con el directorio ausente: {error:?}"
    );
    assert!(
        error
            .to_string()
            .contains(&directorio.to_string_lossy().into_owned()),
        "el diagnóstico nombra el directorio: {error}"
    );
    assert!(!directorio.exists(), "hexcell-admin no crea el directorio");
    assert!(!ruta.exists(), "ni el archivo");
}

/// Sólo lectura: un archivo ausente se rechaza como `ArchivoAusente` y NO se crea. Es la mitad de
/// la garantía de que `cell status` y `cell list` no escriben: con `abrir`, este caso dejaría una
/// base recién migrada detrás de una consulta.
#[test]
fn abrir_solo_lectura_sobre_un_archivo_ausente_no_lo_crea() {
    let temporal = AlmacenTemporal::nuevo("solo-lectura-ausente");
    let error = AlmacenDelPlanoDeControl::abrir_solo_lectura(temporal.ruta())
        .err()
        .expect("un almacén ausente no se abre en sólo lectura");
    assert!(
        matches!(&error, ErrorDeAlmacenDePlano::ArchivoAusente { ruta } if ruta == temporal.ruta()),
        "se esperaba ArchivoAusente: {error:?}"
    );
    assert!(
        error
            .to_string()
            .contains(&temporal.ruta().to_string_lossy().into_owned()),
        "el diagnóstico nombra la ruta ausente: {error}"
    );
    assert!(temporal.bytes().is_none(), "no se crea el archivo");
}

/// Sólo lectura: el MOTOR rechaza la escritura, no una convención. Es lo que convierte «nunca
/// escriben» en una propiedad del descriptor y no de una revisión de código.
#[test]
fn una_escritura_sobre_el_almacen_de_solo_lectura_falla() {
    let temporal = AlmacenTemporal::nuevo("solo-lectura-escritura");
    drop(AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("crear el almacén"));

    let lector =
        AlmacenDelPlanoDeControl::abrir_solo_lectura(temporal.ruta()).expect("abrir sólo lectura");
    assert!(
        lector
            .registrar_transicion(
                "c1",
                None,
                EstadoDeCelula::EnEjecucion,
                MOTIVO_DE_ALTA_IMPLICITA,
                1_700_000_000_000
            )
            .is_err(),
        "el motor rechaza escribir sobre un descriptor de sólo lectura"
    );
    assert!(
        lector
            .leer_estado("c1")
            .expect("leer sigue permitido")
            .is_none(),
        "la escritura rechazada no dejó fila"
    );
}

/// Sólo lectura: un esquema sin migrar se rechaza en vez de consultarse a ciegas contra tablas
/// que quizá no existen.
#[test]
fn abrir_solo_lectura_sobre_un_esquema_sin_migrar_se_rechaza() {
    let temporal = AlmacenTemporal::nuevo("solo-lectura-sin-migrar");
    rusqlite::Connection::open(temporal.ruta())
        .expect("crear un archivo SQLite")
        .execute_batch("CREATE TABLE ajena (x INTEGER);")
        .expect("dejar esquema ajeno con user_version 0");

    let error = AlmacenDelPlanoDeControl::abrir_solo_lectura(temporal.ruta())
        .err()
        .expect("un esquema sin migrar no se lee");
    assert!(
        matches!(
            error,
            ErrorDeAlmacenDePlano::EsquemaSinMigrar {
                encontrada: 0,
                esperada: VERSION_DE_ESQUEMA_DEL_PLANO
            }
        ),
        "se esperaba EsquemaSinMigrar 0 -> {VERSION_DE_ESQUEMA_DEL_PLANO}: {error:?}"
    );
}

/// AC-1: el codec redondea las cinco variantes y la etiqueta es ASCII snake_case, NO el `Display`
/// acentuado: persistir «en ejecución» metería un acento y un espacio en una columna de datos.
/// Los literales se escriben aquí para que una mutación no mueva los dos lados a la vez.
#[test]
fn el_codec_de_etiquetas_redondea_las_cinco_variantes_en_ascii() {
    for estado in EstadoDeCelula::TODOS {
        let etiqueta = etiqueta_persistida(estado);
        assert!(
            etiqueta.is_ascii() && !etiqueta.contains(' '),
            "la etiqueta de {estado:?} tiene que ser ASCII sin espacios: {etiqueta:?}"
        );
        assert_eq!(estado_desde_etiqueta(etiqueta).unwrap(), estado);
    }
    assert_eq!(
        etiqueta_persistida(EstadoDeCelula::EnEjecucion),
        "en_ejecucion"
    );
    assert_eq!(
        etiqueta_persistida(EstadoDeCelula::Suspendida),
        "suspendida"
    );
    assert_eq!(etiqueta_persistida(EstadoDeCelula::Retirada), "retirada");
}

/// AC-1: una etiqueta desconocida se rechaza con la variante tipada, no con un estado por
/// omisión silencioso.
#[test]
fn una_etiqueta_desconocida_se_rechaza() {
    let error = estado_desde_etiqueta("estado_inventado")
        .err()
        .expect("una etiqueta ajena no se resuelve");
    assert!(
        matches!(&error, ErrorDeAlmacenDePlano::EstadoDesconocido { etiqueta } if etiqueta == "estado_inventado"),
        "se esperaba EstadoDesconocido: {error:?}"
    );
}

/// AC-2: registrar una transición deja la fila de `celulas` y UNA de `transiciones`, con el reloj
/// inyectado y no con el del sistema.
#[test]
fn registrar_transicion_inserta_en_ambas_tablas() {
    let temporal = AlmacenTemporal::nuevo("registrar-transicion");
    let almacen = AlmacenDelPlanoDeControl::abrir(temporal.ruta()).unwrap();
    almacen
        .registrar_transicion(
            "c1",
            Some(EstadoDeCelula::EnEjecucion),
            EstadoDeCelula::Suspendida,
            "",
            1_700_000_123_000,
        )
        .unwrap();
    let fila = almacen.leer_estado("c1").unwrap().expect("leer la fila");
    assert_eq!(fila.id, "c1");
    assert_eq!(fila.estado, EstadoDeCelula::Suspendida);
    assert_eq!(fila.motivo, "");
    assert_eq!(fila.actualizado_ms, 1_700_000_123_000);
    drop(almacen);
    assert_eq!(
        transiciones(&lector(&temporal)),
        ["c1 en_ejecucion>suspendida  1700000123000"]
    );
}

/// AC-4: sin fila previa la célula se da de alta con motivo `alta_implicita` y su transición deja
/// el origen vacío, porque no había ninguno.
#[test]
fn una_celula_sin_fila_se_crea_con_alta_implicita() {
    let temporal = AlmacenTemporal::nuevo("alta-implicita");
    let almacen = AlmacenDelPlanoDeControl::abrir(temporal.ruta()).unwrap();
    almacen
        .registrar_transicion(
            "c2",
            None,
            EstadoDeCelula::Suspendida,
            MOTIVO_DE_ALTA_IMPLICITA,
            1_700_000_001_000,
        )
        .unwrap();
    let fila = almacen.leer_estado("c2").unwrap().expect("leer la fila");
    assert_eq!(fila.estado, EstadoDeCelula::Suspendida);
    assert_eq!(fila.motivo, MOTIVO_DE_ALTA_IMPLICITA);
    drop(almacen);
    assert_eq!(
        transiciones(&lector(&temporal)),
        ["c2 >suspendida alta_implicita 1700000001000"]
    );
}

/// AC-1: el historial de sustituciones se lee ORDENADO y sale vacío mientras la tarea 13 no
/// escriba. Las filas del accesorio las inserta la prueba: esta tarea crea la tabla y sólo la lee.
#[test]
fn leer_sustituciones_devuelve_el_historial_ordenado() {
    let temporal = AlmacenTemporal::nuevo("sustituciones");
    let almacen = AlmacenDelPlanoDeControl::abrir(temporal.ruta()).unwrap();
    assert!(almacen.leer_sustituciones("c1").unwrap().is_empty());
    drop(almacen);

    rusqlite::Connection::open(temporal.ruta())
        .unwrap()
        .execute_batch(
            "INSERT INTO sustituciones (id_celula, motivo, registrado_ms)
             VALUES ('c1', 'baneo permanente', 200), ('c1', 'linea devuelta', 100);",
        )
        .unwrap();

    let almacen = AlmacenDelPlanoDeControl::abrir_solo_lectura(temporal.ruta()).unwrap();
    assert_eq!(
        almacen
            .leer_sustituciones("c1")
            .unwrap()
            .iter()
            .map(|s| format!("{} {}", s.motivo, s.registrado_ms))
            .collect::<Vec<_>>(),
        ["linea devuelta 100", "baneo permanente 200"],
        "el historial sale ordenado por registrado_ms ascendente"
    );
}

/// AC-14/AC-15 (HEX-085-b): `confirmar_reemparejamiento` escribe la fila de `celulas` como
/// `EnEjecucion` con motivo `emparejamiento_confirmado`, una transición `Reemparejando` →
/// `EnEjecucion` y una fila de `sustituciones` con el motivo del operador, todo atómicamente.
#[test]
fn confirmar_reemparejamiento_escribe_tres_filas_atomicas() {
    let almacen = AlmacenTemporal::nuevo("conf-reemp");
    let a = AlmacenDelPlanoDeControl::abrir(almacen.ruta()).unwrap();

    // Parte de una célula en `Reemparejando`.
    a.registrar_transicion(
        "c1",
        Some(EstadoDeCelula::EnEjecucion),
        EstadoDeCelula::Reemparejando,
        "baneo-permanente",
        1000,
    )
    .unwrap();

    a.confirmar_reemparejamiento("c1", "baneo-permanente", 5000)
        .unwrap();

    let fila = a.leer_estado("c1").unwrap().unwrap();
    assert_eq!(fila.estado, EstadoDeCelula::EnEjecucion);
    assert_eq!(fila.motivo, MOTIVO_DE_EMPAREJAMIENTO_CONFIRMADO);
    assert_eq!(fila.actualizado_ms, 5000);

    let sustituciones = a.leer_sustituciones("c1").unwrap();
    assert_eq!(sustituciones.len(), 1);
    assert_eq!(sustituciones[0].id_celula, "c1");
    assert_eq!(sustituciones[0].motivo, "baneo-permanente");
    assert_eq!(sustituciones[0].registrado_ms, 5000);
}

/// AC-15 (HEX-085-b): la tabla `sustituciones` guarda exactamente las columnas `id`, `id_celula`,
/// `motivo`, `registrado_ms`, y ninguna de ellas contiene un número de teléfono ni un valor de
/// emparejamiento.
#[test]
fn sustituciones_no_guarda_ni_telefono_ni_valor_de_emparejamiento() {
    let almacen = AlmacenTemporal::nuevo("sustituciones-limpias");
    let a = AlmacenDelPlanoDeControl::abrir(almacen.ruta()).unwrap();

    a.confirmar_reemparejamiento("cel-42", "cambio de número por baneo", 7000)
        .unwrap();

    let sustituciones = a.leer_sustituciones("cel-42").unwrap();
    assert_eq!(sustituciones.len(), 1);
    let fila = &sustituciones[0];
    // Ningún texto almacenado en sustituciones parece un teléfono ni un código de emparejamiento.
    assert_ne!(fila.motivo, "+34600123456");
    assert_ne!(fila.motivo, "ABCD-EFGH");
    // El motivo es literalmente el --motivo del operador.
    assert_eq!(fila.motivo, "cambio de número por baneo");

    // La tabla sustituciones tiene exactamente 4 columnas.
    let conexion = {
        // Abrir una conexión de sólo lectura independiente para consultar el esquema.
        rusqlite::Connection::open_with_flags(
            almacen.ruta(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    };
    let mut sentencia = conexion
        .prepare("PRAGMA table_info(sustituciones)")
        .unwrap();
    let columnas: Vec<String> = sentencia
        .query_map([], |f| f.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(columnas, vec!["id", "id_celula", "motivo", "registrado_ms"]);
}

/// Guarda de fuente: ni el módulo ni la migración contienen tokens de transporte.
#[test]
fn el_modulo_y_la_migracion_no_contienen_tokens_de_transporte() {
    let fuente = include_str!("../src/almacen_plano_de_control.rs");
    let migracion = include_str!("../migraciones/0001-plano-de-control.sql");
    for token in &["jid", "telefono", "numero", "msisdn", "whatsapp"] {
        assert!(
            !fuente.to_lowercase().contains(token),
            "la fuente contiene «{token}»"
        );
        assert!(
            !migracion.to_lowercase().contains(token),
            "la migración contiene «{token}»"
        );
    }
}
