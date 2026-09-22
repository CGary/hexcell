//! Pruebas externas del comando «reporte tokens» de `hexcell-admin` (AC-1 a AC-6).
//!
//! Cada prueba siembra una `sessions.db` temporal a través de `hexcell-storage`
//! (dependencia de desarrollo), produce su copia `VACUUM INTO` con `respaldar_base` —la
//! misma ruta de respaldo de la etapa A-2, la única que este comando lee— y entrega la
//! copia al comando por el camino real del operador: `analizar` + `ejecutar` con sumideros
//! en memoria. Ninguna prueba usa el doble de Docker de `tests/comun/mod.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use hexcell_admin::argumentos::analizar;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::ejecutar;
use hexcell_admin::salida::Salida;
use hexcell_core::identidad::{IdConversacion, IdRemitente};
use hexcell_storage::{
    GestorDePools, NOMBRE_DE_ARCHIVO_DE_SESIONES, RepositorioDeSesiones,
    VERSION_DE_ESQUEMA_DE_SESIONES, VeredictoDeReserva, respaldar_base,
};
use rusqlite::Connection;

/// Anclas de tiempo en milisegundos desde la época Unix, comprobadas con `date -u`.
const MS_2026_09_01_T12: i64 = 1_788_264_000_000; // 2026-09-01T12:00:00Z
const MS_2026_09_10_T00: i64 = 1_788_998_400_000; // 2026-09-10T00:00:00Z (borde de --desde)
const MS_2026_09_15_T12: i64 = 1_789_473_600_000; // 2026-09-15T12:00:00Z

/// Distingue dos directorios creados por el mismo proceso: `process::id()` solo separa procesos.
static SECUENCIA: AtomicUsize = AtomicUsize::new(0);

/// Directorio temporal propio de un test, borrado al salir de alcance, siguiendo el patrón
/// de `crates/hexcell-storage/tests/comun/mod.rs` y `crates/hexcell/tests/`.
struct DirectorioTemporal {
    ruta: PathBuf,
}

impl DirectorioTemporal {
    fn nuevo(etiqueta: &str) -> Self {
        let secuencia = SECUENCIA.fetch_add(1, Ordering::Relaxed);
        let ruta = std::env::temp_dir().join(format!(
            "hexcell-admin-{etiqueta}-{}-{secuencia}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&ruta);
        std::fs::create_dir_all(&ruta).expect("crear el directorio temporal del test");
        Self { ruta }
    }

    fn ruta(&self) -> &Path {
        &self.ruta
    }
}

impl Drop for DirectorioTemporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.ruta);
    }
}

fn args(snippet: &[&str]) -> Vec<String> {
    snippet.iter().map(|s| (*s).to_string()).collect()
}

/// Ejecuta `analizar` + `ejecutar` sobre un snippet y devuelve código, estándar y diagnóstico.
fn ejecutar_con(snippet: &[&str]) -> (CodigoDeSalida, String, String) {
    let argumentos = args(snippet);
    let resultado = analizar(&argumentos);
    let mut bufer_estandar: Vec<u8> = Vec::new();
    let mut bufer_diagnostico: Vec<u8> = Vec::new();
    {
        let mut salida = Salida::nueva(&mut bufer_estandar, &mut bufer_diagnostico);
        let codigo = ejecutar(resultado, &mut salida);
        drop(salida);
        let estandar = String::from_utf8(bufer_estandar).expect("UTF-8 en el estándar");
        let diagnostico = String::from_utf8(bufer_diagnostico).expect("UTF-8 en el diagnóstico");
        (codigo, estandar, diagnostico)
    }
}

fn instante(milisegundos: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_millis(milisegundos as u64)
}

/// Siembra la escena de AC-1/AC-2 en una `sessions.db` nueva y produce su copia
/// `VACUUM INTO` con nombre `copia.db`, devolviendo la ruta de la copia.
///
/// * `conv-a`: reserva 10, conciliada con consumo 4 → 4 unidades, `resuelta_ms`
///   2026-09-01T12:00Z.
/// * `conv-b`: reserva 20, conciliada con consumo 12 → 12 unidades, `resuelta_ms`
///   EXACTAMENTE 2026-09-10T00:00Z, el borde inclusivo de `--desde 2026-09-10`: la mutación
///   «>=» por «>» en el límite inferior deja fuera a `conv-b` y pone roja a AC-2.
/// * `conv-c`: reserva 5, liberada → nunca cuenta.
///
/// El patrón de siembra (anotar la conversación antes de reservar, por la clave foránea de
/// `reservas.id_conversacion`) es el de `crates/hexcell-storage/tests/presupuesto.rs`.
fn sembrar_y_copiar(directorio: &DirectorioTemporal) -> PathBuf {
    let pools = Arc::new(GestorDePools::abrir(directorio.ruta()).expect("abrir los pools"));
    let repo = RepositorioDeSesiones::nuevo(pools.clone());

    let conv_a = IdConversacion::nuevo("conv-a");
    let conv_b = IdConversacion::nuevo("conv-b");
    let conv_c = IdConversacion::nuevo("conv-c");
    for conv in [&conv_a, &conv_b, &conv_c] {
        repo.anotar_entrante(
            conv,
            &IdRemitente::nuevo("remitente-de-prueba"),
            "mensaje inicial",
            SystemTime::UNIX_EPOCH,
        )
        .expect("anotar el mensaje entrante que crea la conversación");
    }
    repo.aportar_presupuesto(1_000, SystemTime::UNIX_EPOCH)
        .expect("aportar presupuesto");

    let r_a = match repo.reservar_presupuesto(&conv_a, 10, SystemTime::UNIX_EPOCH) {
        Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) => id_reserva,
        _ => panic!("reserva de conv-a concedida"),
    };
    repo.conciliar_presupuesto(r_a, 4, instante(MS_2026_09_01_T12))
        .expect("conciliar conv-a");

    let r_b = match repo.reservar_presupuesto(&conv_b, 20, SystemTime::UNIX_EPOCH) {
        Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) => id_reserva,
        _ => panic!("reserva de conv-b concedida"),
    };
    repo.conciliar_presupuesto(r_b, 12, instante(MS_2026_09_10_T00))
        .expect("conciliar conv-b");

    let r_c = match repo.reservar_presupuesto(&conv_c, 5, SystemTime::UNIX_EPOCH) {
        Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) => id_reserva,
        _ => panic!("reserva de conv-c concedida"),
    };
    repo.liberar_presupuesto(r_c, instante(MS_2026_09_15_T12))
        .expect("liberar conv-c");

    let destino = directorio.ruta().join("copia.db");
    pools
        .sesiones()
        .con_lectura(|conexion| {
            respaldar_base(
                conexion,
                &destino,
                VERSION_DE_ESQUEMA_DE_SESIONES,
                NOMBRE_DE_ARCHIVO_DE_SESIONES,
            )
        })
        .expect("producir la copia VACUUM INTO");
    destino
}

/// AC-1: sin periodo, una línea por conversación ordenada por identificador y un `TOTAL`
/// igual a la suma de las conciliaciones sembradas. La conversación liberada contribuye
/// cero —la misma semántica de la vista `consumo_por_conversacion` de la migración 0004—
/// y queda excluida del total. Mutaciones que ponen roja esta prueba: «conciliada» por
/// otra literal en el `CASE`, `-` por `+` en la fórmula, o `COALESCE(m.monto, 0)` por `0`.
#[test]
fn ac_1_sin_periodo_agrega_las_conciliaciones_y_excluye_la_liberada() {
    let directorio = DirectorioTemporal::nuevo("reporte-ac1");
    let copia = sembrar_y_copiar(&directorio);
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        copia.to_str().unwrap(),
    ]);
    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert_eq!(
        estandar,
        "conv-a 4\nconv-b 12\nconv-c 0\nTOTAL piloto-01 inicio fin 16\n"
    );
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");
}

/// AC-2: la ventana `--desde/--hasta` (desde inclusivo, hasta exclusivo) acota por
/// `resuelta_ms`; la conversación resuelta antes de `desde` queda fuera de las líneas y del
/// total. `conv-b` está sembrada exactamente sobre `--desde 2026-09-10` para que la
/// mutación «>=» por «>» en el límite inferior ponga esta prueba roja.
#[test]
fn ac_2_ventana_por_resuelta_ms_deja_fuera_la_conversacion_excluida() {
    let directorio = DirectorioTemporal::nuevo("reporte-ac2");
    let copia = sembrar_y_copiar(&directorio);
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        copia.to_str().unwrap(),
        "--desde",
        "2026-09-10",
        "--hasta",
        "2026-09-11",
    ]);
    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert_eq!(
        estandar,
        "conv-b 12\nTOTAL piloto-01 2026-09-10 2026-09-11 12\n"
    );
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");
}

/// El borde superior es EXCLUSIVO: la mutación «<» por «<=» en el límite superior pone esta
/// prueba roja (`conv-b`, resuelta exactamente sobre `--hasta 2026-09-10`, pasaría a contarse).
#[test]
fn hasta_exclusivo_deja_fuera_la_conversacion_resuelta_exactamente_en_el_borde() {
    let directorio = DirectorioTemporal::nuevo("reporte-hasta-exclusivo");
    let copia = sembrar_y_copiar(&directorio);
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        copia.to_str().unwrap(),
        "--desde",
        "2026-09-09",
        "--hasta",
        "2026-09-10",
    ]);
    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    // Ninguna conversación cae en [2026-09-09, 2026-09-10): sin líneas por conversación.
    assert_eq!(estandar, "TOTAL piloto-01 2026-09-09 2026-09-10 0\n");
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");
}

/// AC-3: una copia llamada `sessions.db`, o una ruta que termina en `-wal`/`-shm`, se
/// rechaza con `UsoIncorrecto` y el mensaje exacto, antes de abrir nada. Los archivos
/// existen en disco (premisa del AC) con contenido basura: de abrirse, sería `Fallo` y la
/// prueba se pondría roja. El mensaje exacto se compara contra la primera línea del
/// diagnóstico, donde el error va solo, antes del texto de uso.
#[test]
fn ac_3_rechaza_copia_llamada_sessions_db_o_con_sufijo_de_wal_o_shm() {
    let directorio = DirectorioTemporal::nuevo("reporte-ac3");
    for nombre in ["sessions.db", "copia.db-wal", "copia.db-shm"] {
        std::fs::write(directorio.ruta().join(nombre), b"basura")
            .expect("escribir el archivo de copia falsa");
    }
    for nombre in ["sessions.db", "copia.db-wal", "copia.db-shm"] {
        let (codigo, estandar, diagnostico) = ejecutar_con(&[
            "reporte",
            "tokens",
            "--celula",
            "piloto-01",
            "--copia",
            directorio.ruta().join(nombre).to_str().unwrap(),
        ]);
        assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto, "copia {nombre:?}");
        assert!(estandar.is_empty(), "estándar vacío para {nombre:?}");
        assert_eq!(
            diagnostico.lines().next().unwrap(),
            "el reporte sólo lee copias VACUUM INTO, nunca sessions.db",
            "primera línea del diagnóstico para {nombre:?}"
        );
    }
    // El rechazo es incondicional: tampoco lo salta --simular (AC-3 manda sobre AC-4).
    let (codigo, _, _) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        "sessions.db",
        "--simular",
    ]);
    assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto);
}

/// AC-4: `--simular` corta en el despacho sin abrir la copia, ni siquiera cuando la ruta no
/// existe en disco: la simulación termina en `Exito` con el estándar y sin diagnóstico.
#[test]
fn ac_4_simular_no_abre_la_copia_inexistente() {
    let directorio = DirectorioTemporal::nuevo("reporte-ac4");
    let inexistente = directorio.ruta().join("copia-inexistente.db");
    assert!(!inexistente.exists(), "premisa: la ruta no existe");
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        inexistente.to_str().unwrap(),
        "--simular",
    ]);
    assert_eq!(codigo, CodigoDeSalida::Exito);
    assert_eq!(
        estandar,
        format!(
            "simulación: reporte tokens --celula piloto-01 --copia {}\n",
            inexistente.display()
        )
    );
    assert!(diagnostico.is_empty(), "diagnóstico vacío: {diagnostico:?}");
}

/// AC-5: una fecha malformada en `--desde` o `--hasta` es `UsoIncorrecto` sin abrir la
/// copia. La copia apunta a una ruta que no existe: de intentarse la lectura, el desenlace
/// sería `Fallo`, nunca `UsoIncorrecto`.
#[test]
fn ac_5_fechas_malformadas_se_rechazan_como_uso_incorrecto() {
    let directorio = DirectorioTemporal::nuevo("reporte-ac5");
    let copia = directorio.ruta().join("copia.db"); // nunca creada.
    let malformadas = [
        "2026-02-30", // febrero de 2026 no tiene 30 días.
        "2026-13-01",
        "2026-00-10",
        "2026-02-29", // 2026 no es bisiesto.
        "2023-02-29",
        "1900-02-29", // divisible por 100 y no por 400.
        "2026-2-01",
        "2026-02-1",
        "abcd-02-01",
        "2026-02-01x",
    ];
    for mala in malformadas {
        for bandera in ["--desde", "--hasta"] {
            let (codigo, estandar, diagnostico) = ejecutar_con(&[
                "reporte",
                "tokens",
                "--celula",
                "piloto-01",
                "--copia",
                copia.to_str().unwrap(),
                bandera,
                mala,
            ]);
            assert_eq!(codigo, CodigoDeSalida::UsoIncorrecto, "{bandera} {mala:?}");
            assert!(
                estandar.is_empty(),
                "estándar vacío para {bandera} {mala:?}"
            );
            assert!(
                diagnostico.contains("fecha inválida"),
                "diag para {bandera} {mala:?}: {diagnostico:?}"
            );
        }
    }
}

/// AC-6: una copia que no es una base SQLite válida, o que es válida pero no tiene el
/// esquema que la consulta necesita, es `Fallo`, no `UsoIncorrecto`. Tres casos: archivo
/// de basura, base válida sin las tablas `reservas`/`movimientos`, y base con `reservas`
/// pero sin la columna `resuelta_ms` que la ventana exige.
#[test]
fn ac_6_copia_ilegible_o_sin_esquema_es_fallo() {
    let directorio = DirectorioTemporal::nuevo("reporte-ac6");

    // (a) Archivo que no es una base SQLite: la apertura o la primera consulta fallan.
    let basura = directorio.ruta().join("basura.db");
    std::fs::write(&basura, b"esto no es una base de datos sqlite")
        .expect("escribir el archivo de basura");
    let (codigo, estandar, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        basura.to_str().unwrap(),
    ]);
    assert_eq!(codigo, CodigoDeSalida::Fallo, "diag: {diagnostico:?}");
    assert!(estandar.is_empty(), "estándar vacío: {estandar:?}");
    assert!(
        !diagnostico.is_empty(),
        "diagnóstico presente: {diagnostico:?}"
    );

    // (b) Base SQLite válida sin las tablas del esquema de sesiones.
    let sin_tablas = directorio.ruta().join("sin-tablas.db");
    {
        let conexion = Connection::open(&sin_tablas).expect("abrir la base vacía");
        conexion
            .execute_batch("CREATE TABLE foo (id INTEGER PRIMARY KEY);")
            .expect("crear la tabla ajena");
    }
    let (codigo, _, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        sin_tablas.to_str().unwrap(),
    ]);
    assert_eq!(codigo, CodigoDeSalida::Fallo, "diag: {diagnostico:?}");
    assert!(
        !diagnostico.is_empty(),
        "diagnóstico presente: {diagnostico:?}"
    );

    // (c) Base con `reservas` y `movimientos` pero sin `resuelta_ms`: la consulta no puede
    //     prepararse (columna ausente) y el desenlace es `Fallo`.
    let sin_resuelta = directorio.ruta().join("sin-resuelta.db");
    {
        let conexion = Connection::open(&sin_resuelta).expect("abrir la base sin resuelta_ms");
        conexion
            .execute_batch(
                "CREATE TABLE reservas (id INTEGER PRIMARY KEY, id_conversacion TEXT, \
                 monto_reservado INTEGER, estado TEXT, creada_ms INTEGER); \
                 CREATE TABLE movimientos (id INTEGER PRIMARY KEY, id_reserva INTEGER, \
                 id_conversacion TEXT, clase TEXT, monto INTEGER, saldo_resultante INTEGER, \
                 registrado_ms INTEGER);",
            )
            .expect("crear el esquema incompleto");
    }
    let (codigo, _, diagnostico) = ejecutar_con(&[
        "reporte",
        "tokens",
        "--celula",
        "piloto-01",
        "--copia",
        sin_resuelta.to_str().unwrap(),
    ]);
    assert_eq!(codigo, CodigoDeSalida::Fallo, "diag: {diagnostico:?}");
    assert!(
        !diagnostico.is_empty(),
        "diagnóstico presente: {diagnostico:?}"
    );
}
