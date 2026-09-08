//! Pruebas de respaldo concurrente con la conmutación de época de conocimiento (etapa A-5, tarea 12).
//!
//! Cierra el criterio de aceptación declarado en el plan: un respaldo ejecutado durante una
//! conmutación produce una copia consistente y restorable; el número de época copiado se registra
//! en la salida del respaldo, leído de la copia y no de la ruta del pool vivo, para que la
//! procedencia sea verificable; y un respaldo que sobrevive al límite de drenaje fuerza un
//! `DesenlaceDeDrenaje::Expirada` que deja la época superseída huérfana y protegida por retención
//! como `SuperseidaSinDrenar`.
//!
//! Las tres pruebas se marcan `#[ignore]` y se invocan por nombre desde un paso dedicado de CI en
//! `.github/workflows/ci.yml` por la misma razón que `crates/hexcell-storage/tests/estres_conmutacion.rs`
//! (HEX-061, `adr-0030`): medir una interacción de ordenamiento entre hilos exige que ninguna otra
//! prueba compita por el proceso, y `cargo` ejecuta los binarios de test de integración
//! secuencialmente, de modo que cada `tests/*.rs` corre solo dentro de su proceso cuando se
//! invoca por nombre.
//!
//! # El solapamiento se fuerza con estado, nunca con el reloj: freno donde se puede, observación
//! donde no
//!
//! Un `Barrier` solo iguala el ARRANQUE de dos hilos; deducir de ahí que la conmutación cayó
//! dentro del respaldo sería un argumento de velocidad relativa que pasaría idéntico con
//! solapamiento cero. En las dos pruebas concurrentes de este archivo el solapamiento es una
//! propiedad del estado, y en ambas lo AFIRMA la lectura de un `AtomicBool` en el instante en que
//! `promover_epoca` devuelve, no una duración.
//!
//! Lo que cambia es cómo se llega a él. **Frenar** el pool de conocimiento es inalcanzable:
//! `promover_epoca` lee el número de la época previa con `pool_anterior.con_lectura` **antes** del
//! intercambio, así que retener todas sus celdas detendría también a la promoción; retener una
//! sola no detiene a nadie, porque el reparto de `PoolDeConocimiento::con_lectura` es un
//! `fetch_add` por turno **determinista** que entrega al siguiente llamante la otra celda; y abrir
//! el pool con anchura 1 —legal— dejaría a `promover_epoca` esperando para siempre con el enlace
//! vivo **ya** repuntado. Por eso la primera prueba frena la conexión única de `sessions.db`, el
//! único punto que detiene al respaldo sin detener a la promoción.
//!
//! **Observar** ese pool sin retenerlo sí es alcanzable, y es lo que hace la tercera prueba:
//! `lecturas_en_reposo()` pasa a `false` en cuanto alguien toma una celda, y si la prueba no
//! retiene ninguna, ese `false` solo puede deberse al respaldo, es decir a que ya fijó su `Guard`
//! sobre la época viva. Conmutar a partir de ahí cae **dentro** de la copia de conocimiento, que
//! es la única ventana en la que derivar el ordinal de la ruta del pool daría un número que la
//! copia desmiente.
//!
//! # Por qué las dos épocas se siembran con marcadores distintos
//!
//! Si ambas épocas salen del mismo fixture su contenido es idéntico, y una copia rota —mezcla de
//! la época N y la N+1— resulta indistinguible de una correcta: la afirmación de pureza no podría
//! fallar nunca. Se adopta la disciplina que HEX-061 ya fijó en este crate
//! (`tests/estres_conmutacion.rs`, `adr-0030`): cada época marca el texto de sus fragmentos, y la
//! copia se juzga por el conjunto de marcadores que de verdad contiene, no por su tamaño.
//!
//! # Por qué los descriptores de `arc-swap` no se prueban aquí
//!
//! `GestorDePools::conocimiento()` devuelve un `Arc` propietario (`load_full`), no el
//! `arc_swap::Guard` que `respaldar_en` usa internamente: la tenencia del respaldo no es
//! reproducible tal cual desde un test. La prueba H3 dice con qué la sustituye.

mod comun;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Barrier, mpsc};
use std::thread;
use std::time::Duration;

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_storage::conocimiento::NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA;
use hexcell_storage::drenaje::{DesenlaceDeDrenaje, drenar_epoca_superseida};
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO};
use hexcell_storage::promocion::{DesenlaceDePromocion, promover_epoca};
use hexcell_storage::retencion::{MotivoDeConservacion, purgar_epocas_retiradas};
use rusqlite::{Connection, OpenFlags};

/// Dimensión sembrada en los fixtures de conocimiento. Reutiliza el valor por omisión de la fila
/// semilla de la migración 0002; subirlo no aporta señal y multiplica el coste lineal del
/// `VectorDeEmbedding` que `validar_integridad_del_indice` materializa en cada promoción.
const DIMENSION_DE_EMBEDDING: usize = 768;

/// Fragmentos por época marcada. Basta un puñado: la señal es **cuáles** marcadores trae la copia,
/// no cuánto tarda un barrido sobre ellos —eso era el objeto de HEX-061, que siembra 1.500—.
const FRAGMENTOS_POR_EPOCA: usize = 8;

/// Fragmentos de la época que la tercera prueba deja viva antes de conmutar. `VACUUM INTO` y el
/// `integrity_check` que `verificar_copia` corre después son lineales en el tamaño de la base, y
/// esa duración **es** la ventana dentro de la cual debe caer la conmutación. Medido el 2026-09-08:
/// con este volumen la copia aún tiene ~28 ms por delante cuando la promoción entera lleva ~2 ms,
/// un margen de unas 15 veces. La cifra no PRUEBA nada —eso lo hace la aserción de solapamiento—:
/// solo hace el caso alcanzable con holgura frente al ruido del planificador de un runner.
const FRAGMENTOS_DE_EPOCA_ANCHA: usize = 6_000;

/// Tope de vueltas de la espera activa sobre `lecturas_en_reposo()`. Es una guarda de vivacidad,
/// no la prueba de nada: si se agota, el respaldo terminó su copia de conocimiento sin que la
/// prueba llegase a verla y el caso no quedó ejercitado, que es justo lo que el mensaje dice.
const VUELTAS_MAXIMAS_DE_ESPERA: u64 = 50_000_000;

/// Tamaño de fragmento, en caracteres exactos de [`UNIDAD_DE_CONTENIDO`]. La compuerta de
/// integridad re-fragmenta el `contenido` del documento y exige que el número de trozos coincida
/// con el de filas de `fragmentos`; repetir una unidad de este tamaño con solapamiento cero hace
/// esa igualdad aritmética en vez de una casualidad del texto elegido.
const TAMANO_DE_FRAGMENTO: usize = 16;

/// Unidad repetida para construir el contenido del documento: exactamente
/// [`TAMANO_DE_FRAGMENTO`] caracteres ASCII.
const UNIDAD_DE_CONTENIDO: &str = "0123456789abcdef";

/// Marcador textual de la época previa a la conmutación. Hace la procedencia verificable por
/// contenido, y no solo por la fila `metadatos_de_epoca` que es lo que la prueba juzga.
const MARCADOR_EPOCA_UNO: &str = "EPOCA-UNO";

/// Marcador textual de la época posterior a la conmutación.
const MARCADOR_EPOCA_DOS: &str = "EPOCA-DOS";

/// Límite deliberadamente bajo del drenaje para la prueba de expiración. El plazo se cuenta desde
/// el **instante del reemplazo** del pool, no desde la llamada al drenaje, así que con 1 ms la
/// primera evaluación del bucle ya lo encuentra vencido y `Expirada` sale sin llegar a dormir ni
/// una vez el `INTERVALO_DE_SONDEO_DE_DRENAJE` de 5 ms. Ese es el punto: lo que hace determinista
/// a la prueba es el cerrojo sostenido, no el reloj; el límite bajo solo evita la espera inútil.
const LIMITE_BAJO_DE_DRENAJE_DE_EPOCA: Duration = Duration::from_millis(1);

/// Límite holgado para los drenajes de higiene, que no son objeto de ninguna aserción: uno bajo
/// los volvería espurios sin añadir señal.
const LIMITE_HOLGADO_DE_DRENAJE_DE_EPOCA: Duration = Duration::from_secs(10);

/// Siembra un staging válido con el fixture compartido que HEX-061 promovió a `tests/comun`
/// (`adr-0030`, decisión 5). Lo usa la prueba de drenaje, a la que el contenido le da igual.
fn sembrar_staging(ruta_datos: &Path) -> ConfiguracionDeFragmentacion {
    comun::preparar_staging_valido(ruta_datos, DIMENSION_DE_EMBEDDING)
}

/// Siembra un staging válido cuyos fragmentos llevan todos `marcador` en su texto.
///
/// No es una cuarta copia de `comun::preparar_staging_valido`, sino un fixture con otro fin —dar
/// a cada época una identidad legible dentro de la copia— que ninguna otra prueba de este binario
/// necesita, y por eso vive aquí en vez de en `tests/comun/mod.rs`. El vector es uniforme y
/// coincide con el de la sonda, así que la compuerta semántica ve similitud coseno 1,0 y el
/// marcador no altera ninguna de las comprobaciones que la promoción aplica.
fn sembrar_staging_marcado(
    ruta_datos: &Path,
    marcador: &str,
    fragmentos: usize,
) -> ConfiguracionDeFragmentacion {
    let ruta_staging = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let conexion = Connection::open(&ruta_staging).expect("abrir base de staging marcada");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    // La ingesta real abre staging en lectura y escritura, que fija WAL desde la primera
    // conexión. Replicarlo evita que un cambio de modo de diario (delete -> wal), que sí exige
    // exclusividad, se confunda con lo que la prueba quiere ejercitar.
    conexion
        .query_row("PRAGMA journal_mode = WAL", [], |fila| {
            fila.get::<_, String>(0)
        })
        .unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging marcado");
    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = ?1 WHERE id = 1",
            rusqlite::params![DIMENSION_DE_EMBEDDING as i64],
        )
        .unwrap();

    let contenido = UNIDAD_DE_CONTENIDO.repeat(fragmentos);
    let vector_bytes: Vec<u8> = vec![1.0f32; DIMENSION_DE_EMBEDDING]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();

    // Un solo documento con ordinales 0..N-1 contiguos: la compuerta comprueba la contigüidad
    // sobre el conjunto GLOBAL de ordinales, así que repartirlos entre varios documentos la
    // rompería por una razón ajena a lo que aquí se mide.
    conexion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_1', 'Documento marcado', ?1, 1000)",
            rusqlite::params![contenido],
        )
        .unwrap();
    // Una sola transacción para las miles de filas de la época ancha: sin ella cada `INSERT`
    // paga su propia consolidación de WAL y el sembrado domina el coste de la prueba.
    conexion.execute_batch("BEGIN;").unwrap();
    for indice in 0..fragmentos {
        let id = (indice + 1) as i64;
        conexion
            .execute(
                "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (?1, 1, ?2, ?3)",
                rusqlite::params![id, indice as i64, format!("{marcador}-fragmento-{indice}")],
            )
            .unwrap();
        conexion
            .execute(
                "INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (?1, ?2)",
                rusqlite::params![id, &vector_bytes],
            )
            .unwrap();
    }
    conexion
        .execute(
            "INSERT INTO sonda_semantica (id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms) VALUES (1, 'consulta', ?1, 0.5, 1000)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();
    conexion.execute_batch("COMMIT;").unwrap();
    drop(conexion);

    ConfiguracionDeFragmentacion {
        tamano_de_fragmento: TAMANO_DE_FRAGMENTO,
        solapamiento: 0,
    }
}

/// Promueve y drena la primera época de un directorio recién creado a partir de un staging ya
/// sembrado.
///
/// La primera promoción del árbol **no** registra entrada en `epocas_en_uso`: `numero_anterior`
/// es `None` porque la fila sembrada por la migración 0002 trae `numero_de_epoca = NULL` (base
/// virgen, antes de cualquier sellado) y `registrar_epoca_en_uso` solo corre con `Some(_)`; por
/// eso el helper tampoco llama a `retirar_epoca_en_uso`. Termina con una purga en vacío que cierra
/// los `-wal`/`-shm` residuales: sin ella la copia saldría de la base inicial sin sellar y su
/// `numero_de_epoca` sería `None`, indistinguible del de una base nunca promovida.
fn promover_y_drenar_epoca_inicial(
    gestor: &GestorDePools,
    directorio: &Path,
    configuracion: &ConfiguracionDeFragmentacion,
) {
    let desenlace = promover_epoca(gestor, directorio, configuracion, 10_000)
        .expect("promoción inicial válida");
    let epoca_superseida = match desenlace {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción inicial no debió abortar: {motivo:?}")
        }
    };
    let drenaje = drenar_epoca_superseida(epoca_superseida, LIMITE_HOLGADO_DE_DRENAJE_DE_EPOCA)
        .expect("drenaje inicial");
    match drenaje {
        DesenlaceDeDrenaje::Drenada { .. } => {}
        otro => panic!("la época inicial debió drenar limpiamente: {otro:?}"),
    }
    let _ = purgar_epocas_retiradas(gestor, directorio, 0)
        .expect("purga en vacío tras la promoción inicial");
}

/// Lee `metadatos_de_epoca.numero_de_epoca` de un archivo de base como observación **independiente**
/// del campo que `CopiaVerificada` reporta: un defecto que falsease el `Value Object` dejando la
/// fila intacta se delataría aquí.
fn leer_numero_de_epoca_de_la_copia(ruta: &Path) -> Option<i64> {
    let conexion = Connection::open_with_flags(ruta, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("abrir la copia en solo lectura para leer su número de época");
    let mut sentencia = conexion
        .prepare("SELECT numero_de_epoca FROM metadatos_de_epoca WHERE id = 1")
        .expect("preparar la lectura del número de época");
    sentencia
        .query_row([], |fila| fila.get::<_, Option<i64>>(0))
        .expect("consultar el número de época de la copia")
}

/// Reparto de marcadores en los fragmentos de un archivo de conocimiento: `(con EPOCA-UNO, con
/// EPOCA-DOS, total)`.
///
/// Devuelve las tres cifras y no un veredicto a propósito: cero fragmentos, una mezcla y la época
/// equivocada entera apuntan a defectos distintos, y el mensaje del fallo debe distinguirlos.
fn repartir_marcadores(ruta: &Path) -> (i64, i64, i64) {
    let conexion = Connection::open_with_flags(ruta, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("abrir la base en solo lectura para clasificar sus marcadores");
    let contar = |patron: &str| -> i64 {
        conexion
            .query_row(
                "SELECT COUNT(*) FROM fragmentos WHERE texto LIKE ?1",
                rusqlite::params![format!("%{patron}%")],
                |fila| fila.get(0),
            )
            .expect("contar fragmentos por marcador")
    };
    let total: i64 = conexion
        .query_row("SELECT COUNT(*) FROM fragmentos", [], |fila| fila.get(0))
        .expect("contar los fragmentos");
    (
        contar(MARCADOR_EPOCA_UNO),
        contar(MARCADOR_EPOCA_DOS),
        total,
    )
}

/// H1: un respaldo en vuelo cuando ocurre una conmutación copia **una sola** época, entera y sin
/// mezclar, y la etiqueta con el ordinal que esa copia contiene de verdad.
/// H2: ese ordinal viaja en la salida del respaldo (`CopiaVerificada::numero_de_epoca`), leído de
/// la copia producida y no de la ruta del pool vivo.
///
/// El solapamiento no se espera del reloj: se fuerza reteniendo la celda de lectura de
/// `sessions.db` —la primera copia de la ronda— para que el respaldo no pueda terminar, y se
/// afirma leyendo un `AtomicBool` en el instante en que la promoción devuelve. La pureza no se
/// da por supuesta: cada época lleva su marcador, así que una copia mezclada o la copia de la
/// época equivocada fallan con un mensaje que dice cuál de las dos cosas pasó.
#[test]
#[ignore]
fn el_respaldo_concurrente_con_una_conmutacion_copia_una_sola_epoca_y_la_registra() {
    let directorio = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h1");
    let destino = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h1-destino");

    let gestor = Arc::new(
        GestorDePools::abrir_con_anchura_de_conocimiento(directorio.ruta(), 2)
            .expect("abrir el gestor con la anchura por omisión"),
    );

    // Época 1, marcada EPOCA-UNO: es la que sirve el pool vivo cuando el respaldo arranca.
    let configuracion_uno =
        sembrar_staging_marcado(directorio.ruta(), MARCADOR_EPOCA_UNO, FRAGMENTOS_POR_EPOCA);
    promover_y_drenar_epoca_inicial(&gestor, directorio.ruta(), &configuracion_uno);

    // Época 2, marcada EPOCA-DOS: se siembra antes de arrancar los hilos para que el coste de la
    // siembra no forme parte de la ventana que la prueba observa.
    let configuracion_dos =
        sembrar_staging_marcado(directorio.ruta(), MARCADOR_EPOCA_DOS, FRAGMENTOS_POR_EPOCA);

    // Freno estructural: un hilo auxiliar toma la ÚNICA conexión de lectura de `sessions.db` y no
    // la suelta hasta que se le indique. `respaldar_en` copia `sessions.db` antes que
    // `knowledge_live.db`, así que el respaldo quedará detenido dentro de su propia llamada.
    let liberar_freno = Arc::new(AtomicBool::new(false));
    let freno_tomado = Arc::new(Barrier::new(2));
    let gestor_freno = Arc::clone(&gestor);
    let liberar_freno_hilo = Arc::clone(&liberar_freno);
    let freno_tomado_hilo = Arc::clone(&freno_tomado);
    let hilo_freno = thread::spawn(move || {
        let _ = gestor_freno.sesiones().con_lectura(|_conexion| {
            freno_tomado_hilo.wait();
            while !liberar_freno_hilo.load(Ordering::Acquire) {
                thread::yield_now();
            }
            Ok(())
        });
    });
    // Al volver de esta barrera la celda de lectura de `sessions.db` está tomada de verdad: el
    // hilo la señala desde DENTRO de `con_lectura`, no antes de llamarla.
    freno_tomado.wait();

    let respaldo_invocado = Arc::new(AtomicBool::new(false));
    let respaldo_terminado = Arc::new(AtomicBool::new(false));
    let respaldo_invocado_hilo = Arc::clone(&respaldo_invocado);
    let respaldo_terminado_hilo = Arc::clone(&respaldo_terminado);
    let gestor_respaldo = Arc::clone(&gestor);
    let destino_respaldo = destino.ruta().to_path_buf();
    let (tx_respaldo, rx_respaldo) = mpsc::channel();
    let hilo_respaldo = thread::spawn(move || {
        respaldo_invocado_hilo.store(true, Ordering::Release);
        let resultado = gestor_respaldo.respaldar_en(&destino_respaldo);
        respaldo_terminado_hilo.store(true, Ordering::Release);
        let _ = tx_respaldo.send(resultado);
    });

    // Esperar a que el hilo haya entrado en `respaldar_en`. A partir de aquí el freno garantiza
    // que no puede salir: la ronda de respaldo está abierta y seguirá abierta.
    while !respaldo_invocado.load(Ordering::Acquire) {
        thread::yield_now();
    }

    let desenlace_promocion =
        promover_epoca(&gestor, directorio.ruta(), &configuracion_dos, 20_000)
            .expect("la promoción durante el respaldo debe completarse sin error");

    // ASERCIÓN DE SOLAPAMIENTO. Es la que convierte «las dos operaciones se solapan» de argumento
    // de velocidad en hecho observado: la conmutación ya devolvió y la ronda de respaldo sigue
    // abierta, porque el freno se lo impide. Si alguien quitase el freno, esta lectura podría ser
    // `true` y la prueba fallaría en vez de seguir certificando un solapamiento que no ocurrió.
    assert!(
        !respaldo_terminado.load(Ordering::Acquire),
        "la conmutación debió completarse con la ronda de respaldo todavía abierta"
    );

    let (numero_promovido, epoca_superseida) = match desenlace_promocion {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            epoca_superseida,
            ..
        } => (numero_de_epoca, epoca_superseida),
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción durante el respaldo no debió abortar: {motivo:?}")
        }
    };
    assert_eq!(
        numero_promovido, 2,
        "la conmutación de esta prueba lleva la época viva de 1 a 2"
    );
    assert_eq!(
        epoca_superseida.numero_de_epoca(),
        Some(1),
        "la época superseída por la conmutación debe ser la época 1"
    );
    let ruta_epoca_superseida: PathBuf = epoca_superseida.ruta_del_archivo().to_path_buf();

    // Soltar el freno: el respaldo continúa ya con la época 2 viva y termina su ronda.
    liberar_freno.store(true, Ordering::Release);
    hilo_freno
        .join()
        .expect("el hilo del freno no debe entrar en pánico");
    let resumen_respaldo = rx_respaldo
        .recv()
        .expect("el hilo de respaldo debe emitir su resultado")
        .expect("el respaldo debe completarse sin error");
    hilo_respaldo
        .join()
        .expect("el hilo de respaldo no debe entrar en pánico");

    let copia_conocimiento = resumen_respaldo
        .copias
        .iter()
        .find(|c| c.nombre_logico == NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO)
        .expect("la copia de knowledge_live.db debe estar presente");
    let copia_sesiones = resumen_respaldo
        .copias
        .iter()
        .find(|c| c.nombre_logico == "sessions.db")
        .expect("la copia de sessions.db debe estar presente");

    // `sessions.db` no modela épocas: nunca debe llevar número, esté o no promovida la base de
    // conocimiento. Cubre también la regresión de que alguien haga que la copia devuelva siempre
    // un número.
    assert!(
        copia_sesiones.numero_de_epoca.is_none(),
        "sessions.db no modela épocas y debe reportar None: {:?}",
        copia_sesiones.numero_de_epoca
    );

    let ruta_copia = destino.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    assert!(
        ruta_copia.exists(),
        "la copia de knowledge_live.db debe existir en disco"
    );
    let integridad: String =
        Connection::open_with_flags(&ruta_copia, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("abrir la copia para integridad")
            .query_row("PRAGMA integrity_check", [], |fila| fila.get(0))
            .expect("ejecutar integrity_check sobre la copia");
    assert_eq!(integridad, "ok", "integrity_check de la copia");

    // Las dos épocas son distinguibles de verdad: la superseída conserva en disco sus marcadores
    // EPOCA-UNO y ninguno de la época dos. Sin esta comprobación, «la copia no trae EPOCA-UNO»
    // pasaría en verde incluso si el marcador nunca se hubiera sembrado.
    let (uno_en_superseida, dos_en_superseida, total_en_superseida) =
        repartir_marcadores(&ruta_epoca_superseida);
    assert_eq!(
        (uno_en_superseida, dos_en_superseida),
        (FRAGMENTOS_POR_EPOCA as i64, 0),
        "la época superseída debe conservar en disco solo sus marcadores EPOCA-UNO ({total_en_superseida} fragmentos)"
    );

    // H1: la copia contiene UNA sola época, entera. Todos sus fragmentos llevan el marcador de la
    // época dos y NINGUNO el de la época uno; una copia rota a caballo entre ambas mezclaría los
    // marcadores y esta aserción la delataría en vez de aceptarla por indistinguible.
    let (uno_en_copia, dos_en_copia, total_en_copia) = repartir_marcadores(&ruta_copia);
    assert_eq!(
        total_en_copia, FRAGMENTOS_POR_EPOCA as i64,
        "la copia debe traer la época entera, no un prefijo: {total_en_copia} fragmentos"
    );
    assert_eq!(
        (uno_en_copia, dos_en_copia),
        (0, FRAGMENTOS_POR_EPOCA as i64),
        "la copia debe ser de una sola época: {uno_en_copia} fragmentos EPOCA-UNO y {dos_en_copia} EPOCA-DOS"
    );

    // H2: el ordinal que el respaldo reporta es el de la época que la copia contiene de verdad.
    // La aserción cruza tres caminos independientes —el campo del `Value Object`, la fila leída
    // de la copia y el conjunto de marcadores— así que una etiqueta fijada a mano (a `1`, a `2` o
    // a cualquier constante) se separa de al menos uno de ellos y falla.
    let numero_fisico = leer_numero_de_epoca_de_la_copia(&ruta_copia);
    assert_eq!(
        copia_conocimiento.numero_de_epoca,
        Some(numero_promovido),
        "el respaldo debe etiquetar la copia con la época que sus marcadores acreditan"
    );
    assert_eq!(
        numero_fisico, copia_conocimiento.numero_de_epoca,
        "el ordinal grabado en la copia ({numero_fisico:?}) debe ser el mismo que el reportado"
    );

    // Higiene: con el respaldo cerrado el pool superseído queda en reposo y drena limpiamente,
    // de modo que el directorio no arrastra una época huérfana a la siguiente prueba.
    let drenaje = drenar_epoca_superseida(epoca_superseida, LIMITE_HOLGADO_DE_DRENAJE_DE_EPOCA)
        .expect("el drenaje de cierre debe completarse sin error fatal");
    match drenaje {
        DesenlaceDeDrenaje::Drenada { constancia, .. } => {
            gestor
                .retirar_epoca_en_uso(&constancia)
                .expect("retirar del inventario la constancia legítima del cierre");
        }
        otro => panic!("con el respaldo ya cerrado el drenaje debió completarse: {otro:?}"),
    }

    println!(
        "el_respaldo_concurrente_con_una_conmutacion_numero_promovido={numero_promovido} \
         numero_reportado={:?} numero_fisico={numero_fisico:?} \
         marcadores_en_la_copia=uno:{uno_en_copia},dos:{dos_en_copia}",
        copia_conocimiento.numero_de_epoca
    );
}

/// H3: un respaldo que supera el límite de drenaje deja la época superseída huérfana y
/// protegida por retención.
///
/// El determinismo NO viene del reloj: un `VACUUM INTO` sobre el sembrado de un fragmento
/// termina en milisegundos. Viene del hecho de que un hilo sostiene una lectura del pool
/// superseído durante toda la ventana del drenaje, de modo que `lecturas_en_reposo()` es falso
/// en toda evaluación del bucle, y `Arc::strong_count` del pool es al menos 2. Esas dos condiciones
/// son las que el predicado del drenaje exige; ambas son inalcanzables mientras el lector
/// sostiene el cerrojo, así que `Expirada` es el único desenlace posible.
#[test]
#[ignore]
fn un_respaldo_que_supera_el_limite_de_drenaje_deja_la_epoca_superseida_sin_drenar_y_protegida() {
    let directorio = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h3");
    let destino = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h3-destino");

    let gestor = Arc::new(
        GestorDePools::abrir_con_anchura_de_conocimiento(directorio.ruta(), 2)
            .expect("abrir el gestor con la anchura por omisión"),
    );

    // Sembrar la época 1 y promoverla para que viva al iniciar la prueba. El helper documenta
    // por qué la promoción desde la base virgen no registra entrada en `epocas_en_uso`.
    let configuracion_uno = sembrar_staging(directorio.ruta());
    promover_y_drenar_epoca_inicial(&gestor, directorio.ruta(), &configuracion_uno);

    // Sembrar la época 2 y promoverla. La `EpocaSuperseida` resultante es la que el lector
    // sostendrá durante toda la ventana de drenaje del segundo intento; el primer intento de
    // drenaje será el que expire. La segunda promoción SÍ registra la entrada de la época
    // superseida en `epocas_en_uso`, porque su `numero_anterior` es `Some(1)`.
    let configuracion_dos = sembrar_staging(directorio.ruta());
    let desenlace_promocion =
        promover_epoca(&gestor, directorio.ruta(), &configuracion_dos, 20_000)
            .expect("promoción de la segunda época válida");
    let epoca_uno_superseida = match desenlace_promocion {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción durante la prueba no debió abortar: {motivo:?}")
        }
    };
    assert_eq!(
        epoca_uno_superseida.numero_de_epoca(),
        Some(1),
        "la época superseída por la segunda promoción debe ser la época 1"
    );

    // Tomar un respaldo para que la prueba **también** ejercite la ruta que dice el contrato:
    // H3 habla de un respaldo que sobrevive al drenaje, así que debe haber un respaldo abierto.
    // El respaldo se completa inmediatamente (la base es un solo fragmento) y deja el archivo
    // escrito; lo que sostiene la lectura durante la ventana del drenaje es una conexión del
    // pool **superseído** tomada con `con_lectura`, no la `VACUUM INTO` del respaldo. Esa
    // conexión es la que vuelve inalcanzable el predicado del drenaje.
    let _resumen_respaldo = gestor
        .respaldar_en(destino.ruta())
        .expect("el respaldo previo al drenaje debe completarse");

    // Tomar un clon del pool superseído y abrir una consulta sostenida con `con_lectura`. El
    // predicado del drenaje exige `lecturas_en_reposo() == true && Arc::strong_count == 1`.
    // Mientras el lector sostenga el `Mutex` de su celda, `lecturas_en_reposo()` será falso en
    // cada poll. El `Arc::strong_count` también será ≥ 2 por la clonación, pero esa mitad ya la
    // cubre la existencia misma de `epoca_uno_superseida.pool()`, que mantiene una referencia
    // fuerte al pool durante todo el argumento. Sobreestimamos por uno respecto al caso real
    // de `respaldar_en` (que mantiene un `arc_swap::Guard`, no un `Arc` adicional) y el
    // resultado es el mismo: el predicado es falso y la única salida es `Expirada`.
    let pool_sostenido = Arc::clone(epoca_uno_superseida.pool());
    let lector_debe_salir = Arc::new(AtomicBool::new(false));
    let lector_debe_salir_hilo = Arc::clone(&lector_debe_salir);
    let barrera_lector_iniciado = Arc::new(Barrier::new(2));

    let barrera_inicio = Arc::clone(&barrera_lector_iniciado);
    let hilo_lector = thread::spawn(move || {
        // La consulta dura mientras `lector_debe_salir` siga en `false`. Como
        // `con_lectura` toma un `Mutex` bloqueante, la única manera de sostener el cerrojo a
        // través de múltiples polls del drenaje es ejecutar UNA llamada que dure más que el
        // límite. Se usa una transacción con `BEGIN IMMEDIATE` y un `sleep` para garantizar
        // que la celda queda tomada durante toda la ventana.
        let _ = pool_sostenido.con_lectura(|conexion| {
            conexion
                .execute_batch("BEGIN IMMEDIATE;")
                .expect("abrir la transacción que sostendrá la lectura");
            barrera_inicio.wait();
            while !lector_debe_salir_hilo.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            conexion
                .execute_batch("COMMIT;")
                .expect("confirmar la transacción sostenida antes de soltar");
            Ok(())
        });
    });

    barrera_lector_iniciado.wait();

    // Verificar que el predicado es falso antes de invocar el drenaje: `lecturas_en_reposo()`
    // debe ser `false` y `Arc::strong_count` ≥ 2 (una por `epoca_uno_superseida`, una por
    // `pool_sostenido`).
    assert!(
        !epoca_uno_superseida.lecturas_en_reposo(),
        "lecturas_en_reposo() debe ser falso mientras la lectura sostenida está activa"
    );
    let titulares_pre_drenaje = Arc::strong_count(epoca_uno_superseida.pool());
    assert!(
        titulares_pre_drenaje >= 2,
        "Arc::strong_count del pool debe ser ≥ 2 mientras la lectura está activa: {titulares_pre_drenaje}"
    );

    // Invocar el drenaje con el límite deliberadamente bajo, **pasado como parámetro directo**
    // y nunca vía la variable de entorno que el binario del sidecar usa para el límite. Es la
    // única manera de mantener `hexcell-storage` executor-free y libre de variables de entorno,
    // y la guarda de CI contra la mutación del entorno del proceso (adr-0028) no admite
    // excepciones.
    let ruta_epoca_superseida: PathBuf = epoca_uno_superseida.ruta_del_archivo().to_path_buf();
    assert!(
        ruta_epoca_superseida.exists(),
        "el archivo de la época superseída debe existir antes del drenaje"
    );

    let resultado_drenaje =
        drenar_epoca_superseida(epoca_uno_superseida, LIMITE_BAJO_DE_DRENAJE_DE_EPOCA)
            .expect("el drenaje debe completarse sin error fatal (devuelve Expirada o Drenada)");

    // Liberar al lector: el predicado es inalcanzable mientras sostenga el cerrojo.
    lector_debe_salir.store(true, Ordering::Relaxed);
    hilo_lector
        .join()
        .expect("el hilo lector no debe entrar en pánico");

    // La única salida posible mientras el lector sostiene la celda es `Expirada`, con las dos
    // condiciones reportadas observadas.
    let epoca_devuelta = match resultado_drenaje {
        DesenlaceDeDrenaje::Expirada {
            epoca_superseida,
            titulares,
            lecturas_en_reposo,
        } => {
            assert!(
                !lecturas_en_reposo,
                "lecturas_en_reposo debe ser false en el momento de la expiración"
            );
            assert!(
                titulares >= 2,
                "Arc::strong_count debe ser ≥ 2 en el momento de la expiración: {titulares}"
            );
            epoca_superseida
        }
        DesenlaceDeDrenaje::Drenada { .. } => {
            panic!(
                "el drenaje debió expirar con la lectura activa; el único modo de que termine \
                 Drenada es que el lector haya soltado el cerrojo, lo que rompe el invariante \
                 de la prueba"
            );
        }
        DesenlaceDeDrenaje::Retenida { .. } => {
            panic!(
                "Retenida requiere que se cumpla el predicado y aparezca otra referencia antes \
                 del consumo exclusivo; con la lectura activa el predicado nunca se cumple"
            );
        }
    };

    // Tras la expiración, el descriptor vivo se devolvió intacto: el archivo de la época 1
    // sigue en disco y el inventario de `epocas_en_uso` aún contiene la entrada. Estos dos
    // hechos son la observación que demuestra el invariante de retención.
    assert!(
        ruta_epoca_superseida.exists(),
        "el archivo de la época superseída no debe eliminarse en la expiración"
    );
    assert!(
        gestor.epocas_en_uso().contains_key(&1),
        "la época superseída sin drenar debe seguir en epocas_en_uso"
    );

    // Confirmar la clasificación de la purga: con ventana 0, la época 1 **no** se purga porque
    // está en `epocas_en_uso`, y el motivo exacto es `SuperseidaSinDrenar`. La purga observa el
    // estado y lo clasifica, no lo asume.
    let purga = purgar_epocas_retiradas(&gestor, directorio.ruta(), 0)
        .expect("la purga debe completarse sin error");
    assert!(
        purga.epocas_purgadas.is_empty(),
        "la época huérfana debe conservarse, no purgarse: {:?}",
        purga.epocas_purgadas
    );
    let conservada = purga
        .epocas_conservadas
        .iter()
        .find(|c| c.numero_de_epoca == 1)
        .expect("la purga debe clasificar la época huérfana");
    assert_eq!(
        conservada.motivo,
        MotivoDeConservacion::SuperseidaSinDrenar,
        "el motivo exacto de conservación debe ser SuperseidaSinDrenar"
    );
    assert!(
        ruta_epoca_superseida.exists(),
        "el archivo de la época huérfana debe seguir presente tras la purga"
    );

    // H3, lado de la reutilización: la época devuelta por la expiración es exactamente la
    // misma y, una vez liberado el lector, drena limpiamente. Esto cierra el ciclo: el
    // drenaje no es destructivo, el operador puede reintentar tras liberar el cerrojo.
    let resultado_drenaje_reintentado =
        drenar_epoca_superseida(epoca_devuelta, LIMITE_HOLGADO_DE_DRENAJE_DE_EPOCA)
            .expect("el reintento de drenaje debe completarse sin error fatal");
    let constancia_reintentada = match resultado_drenaje_reintentado {
        DesenlaceDeDrenaje::Drenada { constancia, .. } => constancia,
        otro => panic!(
            "tras liberar el lector el drenaje debió completarse limpiamente, se obtuvo: {otro:?}"
        ),
    };
    assert_eq!(
        constancia_reintentada.numero_de_epoca(),
        Some(1),
        "la constancia del reintento debe pertenecer a la época 1"
    );
    gestor
        .retirar_epoca_en_uso(&constancia_reintentada)
        .expect("retirar la constancia legítima del inventario tras el reintento");

    println!(
        "un_respaldo_que_supera_el_limite_de_drenaje_titulares_pre_drenaje={titulares_pre_drenaje}"
    );
    println!(
        "un_respaldo_que_supera_el_limite_de_drenaje_limite_usado={:?}",
        LIMITE_BAJO_DE_DRENAJE_DE_EPOCA
    );
}

/// Espera activa y **acotada** a que el respaldo tome una celda de lectura del pool vivo.
///
/// Costura de OBSERVACIÓN, no freno: la prueba no retiene ninguna celda —sería inalcanzable, ver
/// el encabezado del módulo—, así que un `lecturas_en_reposo()` en `false` solo puede deberse al
/// respaldo, y eso implica que `respaldar_en` ya ejecutó `self.conocimiento.load()`: su `Guard`
/// quedó fijado a la época viva **actual** y ninguna conmutación posterior cambia de qué archivo
/// sale la copia. Ni duerme ni consulta el reloj —dormir metería una duración en un argumento que
/// es de estado—; el tope solo evita que un fallo aparezca como un test colgado.
fn esperar_a_que_el_respaldo_tome_una_celda_de_conocimiento(gestor: &GestorDePools) {
    for _ in 0..VUELTAS_MAXIMAS_DE_ESPERA {
        if !gestor.conocimiento().lecturas_en_reposo() {
            return;
        }
        thread::yield_now();
    }
    panic!(
        "el respaldo nunca fue visto sosteniendo una celda del pool de conocimiento: o no llegó a \
         esa fase, o su copia terminó antes de que la prueba pudiera observarla. En ambos casos la \
         conmutación caería FUERA de la copia y la prueba no ejercitaría lo que dice ejercitar"
    );
}

/// H2-bis: la copia conserva la época que el respaldo tenía **fijada** aunque el enlace vivo ya
/// apunte a la siguiente.
///
/// Es la prueba que hace NECESARIA la decisión central del diseño —leer el ordinal de la copia
/// producida y no derivarlo de `PoolDeConocimiento::ruta()`—, y no solo elegante: aquí la ruta del
/// pool vivo resuelve a `knowledge_epoch_2.db` mientras la copia contiene, entera, la época 1. Una
/// etiqueta derivada de la ruta diría 2 sobre un archivo que dice 1 de sí mismo, que es el error
/// que el operador vería al restaurar. La primera prueba de este archivo no puede demostrarlo: su
/// freno vive en `sessions.db`, que `respaldar_en` copia **antes** de resolver el puntero de
/// conocimiento, así que allí la conmutación termina entera antes de que la copia de conocimiento
/// empiece y las dos fuentes coinciden.
///
/// El orden lo da la costura de observación: el respaldo arranca primero, la prueba espera a verlo
/// sosteniendo una celda de conocimiento y solo entonces conmuta; que la conmutación cayera DENTRO
/// de esa copia se afirma leyendo un `AtomicBool`, no se supone.
#[test]
#[ignore]
fn la_copia_conserva_la_epoca_fijada_aunque_el_enlace_vivo_ya_apunte_a_la_siguiente() {
    let directorio = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h2bis");
    let destino = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h2bis-destino");

    let gestor = Arc::new(
        GestorDePools::abrir_con_anchura_de_conocimiento(directorio.ruta(), 2)
            .expect("abrir el gestor con la anchura por omisión"),
    );

    // Época 1 ancha y época 2 mínima: la copia de la primera es la ventana, la promoción de la
    // segunda es lo que tiene que caber dentro. Las dos cifras hacen el caso ALCANZABLE; quien lo
    // da por ocurrido es la aserción de solapamiento, no el tamaño de las bases.
    let configuracion_uno = sembrar_staging_marcado(
        directorio.ruta(),
        MARCADOR_EPOCA_UNO,
        FRAGMENTOS_DE_EPOCA_ANCHA,
    );
    promover_y_drenar_epoca_inicial(&gestor, directorio.ruta(), &configuracion_uno);
    let configuracion_dos =
        sembrar_staging_marcado(directorio.ruta(), MARCADOR_EPOCA_DOS, FRAGMENTOS_POR_EPOCA);

    let respaldo_terminado = Arc::new(AtomicBool::new(false));
    let respaldo_terminado_hilo = Arc::clone(&respaldo_terminado);
    let gestor_respaldo = Arc::clone(&gestor);
    let destino_respaldo = destino.ruta().to_path_buf();
    let (tx_respaldo, rx_respaldo) = mpsc::channel();
    let hilo_respaldo = thread::spawn(move || {
        let resultado = gestor_respaldo.respaldar_en(&destino_respaldo);
        respaldo_terminado_hilo.store(true, Ordering::Release);
        let _ = tx_respaldo.send(resultado);
    });

    esperar_a_que_el_respaldo_tome_una_celda_de_conocimiento(&gestor);

    let desenlace_promocion =
        promover_epoca(&gestor, directorio.ruta(), &configuracion_dos, 20_000)
            .expect("la promoción durante el respaldo debe completarse sin error");

    // ASERCIÓN DE SOLAPAMIENTO. Sin ella la prueba certificaría una ventana que quizá no ocurrió:
    // si el respaldo hubiese cerrado su copia antes de que la promoción repuntara el enlace, la
    // etiqueta derivada de la ruta también habría acertado y el caso no discriminaría nada.
    assert!(
        !respaldo_terminado.load(Ordering::Acquire),
        "la conmutación debió completarse con la copia de conocimiento todavía en vuelo; de otro \
         modo el enlace vivo se repuntó fuera de la ventana y la prueba no separa leer el ordinal \
         de la copia de derivarlo de la ruta del pool"
    );

    // `promover_epoca` devuelve `Ok(Abortada)` cuando la compuerta rechaza el staging, así que el
    // `expect` de arriba no basta: hay que mirar el desenlace. La época superseída no se drena
    // aquí —eso es lo que H3 mide— y el directorio temporal muere con la prueba.
    assert!(
        matches!(
            desenlace_promocion,
            DesenlaceDePromocion::Promovida {
                numero_de_epoca: 2,
                ..
            }
        ),
        "la conmutación debió promover la época 2: {desenlace_promocion:?}"
    );

    // El contrafáctico, hecho observación: la ruta con la que se abrió el pool —el enlace
    // `knowledge_live.db`— resuelve YA a la época 2. Cualquier etiqueta derivada de ella daría 2.
    let enlace_resuelto =
        std::fs::canonicalize(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO))
            .expect("resolver el enlace de la época viva tras la conmutación");
    assert!(
        enlace_resuelto.ends_with("knowledge_epoch_2.db"),
        "tras la conmutación el enlace vivo debe resolver a la época 2: {enlace_resuelto:?}"
    );

    let resumen_respaldo = rx_respaldo
        .recv()
        .expect("el hilo de respaldo debe emitir su resultado")
        .expect("el respaldo debe completarse sin error");
    hilo_respaldo
        .join()
        .expect("el hilo de respaldo no debe entrar en pánico");
    let copia_conocimiento = resumen_respaldo
        .copias
        .iter()
        .find(|c| c.nombre_logico == NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO)
        .expect("la copia de knowledge_live.db debe estar presente");

    // Y el hecho que el contrafáctico contradice: la copia es la época 1, entera y sin mezclar,
    // y las tres fuentes independientes —marcadores, fila grabada y campo reportado— coinciden
    // en 1. Una etiqueta derivada de la ruta se habría separado de las tres a la vez.
    let ruta_copia = destino.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);
    let (uno_en_copia, dos_en_copia, total_en_copia) = repartir_marcadores(&ruta_copia);
    assert_eq!(
        (uno_en_copia, dos_en_copia),
        (FRAGMENTOS_DE_EPOCA_ANCHA as i64, 0),
        "la copia debe traer la época 1 entera y ningún marcador de la 2 ({total_en_copia} fragmentos)"
    );
    assert_eq!(
        copia_conocimiento.numero_de_epoca,
        Some(1),
        "el respaldo debe reportar la época que la copia contiene, no la que el enlace ya sirve"
    );
    assert_eq!(
        leer_numero_de_epoca_de_la_copia(&ruta_copia),
        Some(1),
        "el ordinal grabado dentro de la copia debe ser el mismo que el reportado"
    );
}
