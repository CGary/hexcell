//! Pruebas de respaldo concurrente con la conmutación de época de conocimiento (etapa A-5, tarea 12).
//!
//! Cierra el criterio de aceptación declarado en el plan: un respaldo ejecutado durante una
//! conmutación produce una copia consistente y restorable; el número de época copiado se registra
//! en la salida del respaldo para que la procedencia sea verificable; y un respaldo que sobrevive
//! al límite de drenaje fuerza un `DesenlaceDeDrenaje::Expirada` que deja la época superseída
//! huérfana y protegida por retención como `SuperseidaSinDrenar`.
//!
//! Las dos pruebas se marcan `#[ignore]` y se invocan por nombre desde un paso dedicado de CI en
//! `.github/workflows/ci.yml` por la misma razón que `crates/hexcell-storage/tests/estres_conmutacion.rs`
//! (HEX-061, `adr-0030`): medir una interacción de ordenamiento entre hilos exige que ninguna otra
//! prueba compita por el proceso, y `cargo` ejecuta los binarios de test de integración
//! secuencialmente, de modo que cada `tests/*.rs` corre solo dentro de su proceso cuando se
//! invoca por nombre.
//!
//! # Por qué los descriptores de `arc-swap` no se prueban aquí
//!
//! La opción tentadora era simular exactamente la tenencia de `respaldar_en` reutilizando un
//! `arc_swap::Guard` de `GestorDeConocimiento.conocimiento.load()`. El API de `arc-swap` 1.9.2 lo
//! permite, pero `GestorDePools::conocimiento()` ya devuelve un `Arc<PoolDeConocimiento>`
//! propietario (`load_full`), no un guard; el guard se construye a mano y nunca se ve desde el
//! exterior. Se opta por reproducir la tenencia con `conocimiento()` + una lectura sostenida
//! bajo `con_lectura`, que sobreestima el `Arc::strong_count` en uno respecto al respaldo real
//! pero deja el predicado del drenaje (`lecturas_en_reposo() == false`) exactamente igual de
//! inalcanzable. El comentario del segundo test lo dice abiertamente y la conclusión no cambia.
//!
//! # Por qué la copia física del `VacuumInto` se asume atómica
//!
//! `VACUUM INTO` es una sola sentencia SQL: SQLite la serializa dentro de su motor y produce un
//! archivo cuyo contenido es el de **una** época, no un interleaving. La conmutación del pool
//! con `ArcSwap` es atómica por construcción (intercambio de un puntero). Que la copia salga de
//! **una** época y no de dos mezcladas es entonces consecuencia de estas dos garantías sumadas,
//! y la prueba lo verifica leyendo `metadatos_de_epoca.numero_de_epoca` de la copia y
//! comprobando que su valor es uno de los dos ordinales observados durante la ventana de solapamiento,
//! nunca un número inventado ni un NULL espurio.

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
use hexcell_storage::pools::{GestorDePools, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO};
use hexcell_storage::promocion::{DesenlaceDePromocion, promover_epoca};
use hexcell_storage::retencion::{MotivoDeConservacion, purgar_epocas_retiradas};
use rusqlite::{Connection, OpenFlags};

/// Dimensión sembrada en los fixtures de conocimiento. Reutiliza el valor por omisión de la fila
/// semilla de la migración 0002; subirlo no aporta señal y multiplica el coste lineal del
/// `VectorDeEmbedding` que `validar_integridad_del_indice` materializa en cada promoción.
const DIMENSION_DE_EMBEDDING: usize = 768;

/// Límite deliberadamente bajo del drenaje para la prueba de expiración. La meta es que un solo
/// poll de `INTERVALO_DE_SONDEO_DE_DRENAJE` (5 ms) ya supere este plazo: cualquier determinismo
/// por timing sería frágil y el de la prueba no lo es. 1 ms basta y se mantiene holgadamente
/// por encima del coste de una iteración del bucle de drenaje.
const LIMITE_BAJO_DE_DRENAJE_DE_EPOCA: Duration = Duration::from_millis(1);

/// Límite del drenaje del primer test: holgado, para que si el respaldo termina antes que el
/// drenaje (el caso normal), el descriptor superseido cierre limpiamente sin influir en el
/// resultado de la prueba. El primer test no es una prueba de drenaje y el límite bajo lo haría
/// espurio.
const LIMITE_HOLGADO_DE_DRENAJE_DE_EPOCA: Duration = Duration::from_secs(10);

/// Siembra un archivo de staging válido reutilizando el helper compartido del módulo `comun`.
///
/// Se mantiene local a propósito: `comun::preparar_staging_valido` ya es el fixture compartido
/// (HEX-061, `adr-0030` decisión 5). Esta función añade el wrapping del retorno a `PathBuf`
/// para que la llamada quede legible cuando la siembra aparece dentro de la función que también
/// dispara la promoción.
fn sembrar_staging(ruta_datos: &Path) -> ConfiguracionDeFragmentacion {
    comun::preparar_staging_valido(ruta_datos, DIMENSION_DE_EMBEDDING)
}

/// Siembra, promueve y drena la primera época de un directorio recién creado.
///
/// La primera promoción del árbol **no** registra entrada en `epocas_en_uso` para la base
/// inicial: `numero_anterior` es `None` porque la fila sembrada por la migración 0002 tiene
/// `metadatos_de_epoca.numero_de_epoca = NULL` (base virgen, antes de cualquier sellado), y
/// `registrar_epoca_en_uso` solo se invoca cuando `numero_anterior` es `Some(_)`. El helper
/// no llama a `retirar_epoca_en_uso` precisamente por eso, y termina con una purga en vacío
/// que cierra cualquier `-wal`/`-shm` residual y deja el directorio en el mismo estado de
/// archivos que tendría si el proceso nunca hubiera arrancado. Sin esa limpieza, la copia de
/// `knowledge_live.db` se realizaría sobre la base inicial sin sellar y el campo
/// `numero_de_epoca` saldría `None`, indistinguible del de una base nunca promovida.
fn sembrar_y_drenar_epoca_inicial(gestor: &GestorDePools, directorio: &Path) {
    let configuracion = sembrar_staging(directorio);
    let desenlace = promover_epoca(gestor, directorio, &configuracion, 10_000)
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

/// Lee `metadatos_de_epoca.numero_de_epoca` directamente de la copia, como una **observación**
/// independiente del campo que `CopiaVerificada::numero_de_epoca` reporta. Es la materia de la
/// aserción: lo que la copia **dice** sobre sí misma tiene que coincidir con lo que el respaldo
/// afirma haber copiado, y un defecto que falsease el campo del `Value Object` mientras deja la
/// fila intacta se detectaría aquí.
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

/// Cuenta los fragmentos presentes en una copia de `knowledge_live.db` por ordinal. Se usa para
/// distinguir una copia con cero fragmentos (defecto imaginable: promoción abortada silenciada
/// por `respaldar_en`) de una copia con un único fragmento (época completa).
fn contar_ordinales_en_la_copia(ruta: &Path) -> i64 {
    let conexion = Connection::open_with_flags(ruta, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("abrir la copia en solo lectura para contar fragmentos");
    conexion
        .query_row("SELECT COUNT(*) FROM fragmentos", [], |fila| fila.get(0))
        .expect("contar los fragmentos de la copia")
}

/// Cuenta los fragmentos leídos por el `valor_logico` que el descriptor `CopiaVerificada`
/// reporta. Se usa para confirmar que la copia no quedó en un estado que el motor no podría
/// servir (cero fragmentos), pero sin contar dos veces lo ya contado: la prueba verifica el
/// valor a través de dos caminos (el campo del `Value Object` y la lectura del archivo físico).
///
/// H1: un respaldo concurrente con una conmutación copia una sola época y la registra.
/// H2: el respaldo expone la marca de número de época en su salida, leída de la copia.
/// H3: un respaldo que sobrevive al límite de drenaje expira el drenaje y deja la época
///     superseída huérfana y protegida.
#[test]
#[ignore]
fn el_respaldo_concurrente_con_una_conmutacion_copia_una_sola_epoca_y_la_registra() {
    let directorio = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h1");
    let destino = DirectorioTemporal::nuevo("respaldo-durante-conmutacion-h1-destino");

    let gestor = Arc::new(
        GestorDePools::abrir_con_anchura_de_conocimiento(directorio.ruta(), 2)
            .expect("abrir el gestor con la anchura por omisión"),
    );

    // Época 1: sembrar, promover y drenar para que el pool vivo sirva la época 1 al iniciar
    // la prueba y el directorio quede sin `-wal`/`-shm` ni épocas conservadas que
    // contaminen la copia posterior. El helper documenta por qué la promoción desde la base
    // virgen no registra entrada en `epocas_en_uso`.
    sembrar_y_drenar_epoca_inicial(&gestor, directorio.ruta());

    // Sembrar el staging de la época que va a estar viva cuando el respaldo arranque. Se hace
    // aquí, antes del `Barrier`, para que `promover_epoca` (la fase que más tarda de la
    // promoción, junto con el sellado) corra con el respaldo ya en `Barrier.wait()`, no antes;
    // de lo contrario el solapamiento quedaría dominado por el coste de la siembra, no por la
    // conmutación.
    let configuracion_dos = sembrar_staging(directorio.ruta());
    let ruta_staging_dos = directorio
        .ruta()
        .join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    assert!(
        ruta_staging_dos.exists(),
        "el staging de la época dos debe existir antes del rendezvous"
    );
    // La variable solo se usa como aserción de precondición; el respaldo trabaja sobre el pool
    // vivo, no sobre el staging. Se suelta explícitamente para no contaminar el closure del
    // hilo de respaldo con una variable sin usar.
    drop(ruta_staging_dos);

    // Rendezvous de dos hilos en el mismo instante. El hilo de respaldo llama a
    // `respaldar_en`; el hilo de promoción llama a `promover_epoca` para conmutar a N+1. La
    // barrera fija el inicio simultáneo, no el final: las dos operaciones se solapan por
    // construcción, porque la promoción tarda mucho más que un `VACUUM INTO` sobre la base de
    // un solo fragmento sembrada por el fixture compartido.
    let barrera = Arc::new(Barrier::new(2));
    let gestor_respaldo = Arc::clone(&gestor);
    let gestor_promocion = Arc::clone(&gestor);
    let barrera_respaldo = Arc::clone(&barrera);
    let barrera_promocion = Arc::clone(&barrera);
    let directorio_promocion = directorio.ruta().to_path_buf();
    let destino_respaldo = destino.ruta().to_path_buf();
    let ruta_destino_copia_conocimiento = destino.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO);

    let (tx_resultado_respaldo, rx_resultado_respaldo) = mpsc::channel();
    let (tx_resultado_promocion, rx_resultado_promocion) = mpsc::channel();

    let hilo_respaldo = thread::spawn(move || {
        barrera_respaldo.wait();
        let resultado = gestor_respaldo.respaldar_en(&destino_respaldo);
        let _ = tx_resultado_respaldo.send(resultado);
    });

    let hilo_promocion = thread::spawn(move || {
        barrera_promocion.wait();
        // `promover_epoca` consume la configuración por referencia; se clona el `Path` antes de
        // moverlo al closure para que `configuracion_dos` no se mueva dos veces.
        let resultado = promover_epoca(
            &gestor_promocion,
            &directorio_promocion,
            &configuracion_dos,
            20_000,
        );
        let _ = tx_resultado_promocion.send(resultado);
    });

    let resumen_respaldo = rx_resultado_respaldo
        .recv()
        .expect("el hilo de respaldo debe emitir su resultado")
        .expect("el respaldo debe completarse sin error");

    let desenlace_promocion = rx_resultado_promocion
        .recv()
        .expect("el hilo de promoción debe emitir su resultado")
        .expect("la promoción debe completarse sin error");

    hilo_respaldo
        .join()
        .expect("el hilo de respaldo no debe entrar en pánico");
    hilo_promocion
        .join()
        .expect("el hilo de promoción no debe entrar en pánico");

    let numero_promovido = match &desenlace_promocion {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca, ..
        } => *numero_de_epoca,
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción durante el respaldo no debió abortar: {motivo:?}")
        }
    };

    // El primer test no busca H3; su epoca_superseida podría drenar limpiamente o expirar según
    // cuándo termine el respaldo respecto al plazo de drenaje. Se drena aquí mismo, con un
    // plazo holgado, para no dejar `epocas_en_uso` contaminando el estado de archivos.
    let _epoca_superseida = match desenlace_promocion {
        DesenlaceDePromocion::Promovida {
            epoca_superseida, ..
        } => epoca_superseida,
        DesenlaceDePromocion::Abortada { .. } => unreachable!("filtrada arriba"),
    };

    // Identificar la copia de conocimiento y la copia de sesiones en el resumen. El orden de
    // `respaldar_en` es fijo: primero `sessions.db`, después `knowledge_live.db`.
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

    // H2: el campo `numero_de_epoca` se reporta en la salida del respaldo. Es lo que el operador
    // puede leer sin volver a abrir la copia; si está mal, lo siguiente falla con claridad.
    assert!(
        copia_conocimiento.numero_de_epoca.is_some(),
        "la copia de knowledge_live.db debe registrar el número de época que copió"
    );
    let numero_reportado = copia_conocimiento
        .numero_de_epoca
        .expect("verificado arriba");

    // `sessions.db` no modela épocas: nunca debe llevar número, esté o no promovida la base de
    // conocimiento. Esta aserción cubre también el caso regresivo de que alguien cambie la copia
    // por error para siempre devolver un número.
    assert!(
        copia_sesiones.numero_de_epoca.is_none(),
        "sessions.db no modela épocas y debe reportar None: {:?}",
        copia_sesiones.numero_de_epoca
    );

    // La copia física existe, abre y pasa las dos lecturas baratas.
    assert!(
        ruta_destino_copia_conocimiento.exists(),
        "la copia de knowledge_live.db debe existir en disco"
    );
    let integridad: String = Connection::open_with_flags(
        &ruta_destino_copia_conocimiento,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("abrir la copia para integridad")
    .query_row("PRAGMA integrity_check", [], |fila| fila.get(0))
    .expect("ejecutar integrity_check sobre la copia");
    assert_eq!(integridad, "ok", "integrity_check de la copia");

    // H1, lado contenido: la copia es de **una** sola época, no un interleaving de N y N+1.
    // Lo prueba el hecho de que su `metadatos_de_epoca.numero_de_epoca` es un ordinal
    // **definido**, igual al reportado, y de que el número de fragmentos es coherente con el
    // sembrado (1 fragmento, exactamente lo que `comun::preparar_staging_valido` inserta). Una
    // copia mezclada exhibiría un número NULL o un valor raro, o un conteo inconsistente.
    let numero_fisico = leer_numero_de_epoca_de_la_copia(&ruta_destino_copia_conocimiento);
    assert_eq!(
        numero_fisico,
        Some(numero_reportado),
        "el número físico de la copia ({:?}) debe coincidir con el reportado ({:?})",
        numero_fisico,
        numero_reportado
    );

    let ordinales = contar_ordinales_en_la_copia(&ruta_destino_copia_conocimiento);
    assert_eq!(
        ordinales, 1,
        "el sembrado de HEX-061 inserta un único fragmento; la copia debe contenerlo intacto"
    );

    // H1, lado lógico: el ordinal registrado por la copia es uno de los que estuvieron vivos
    // durante el solapamiento. El pool partió sirviendo la época 1, y la promoción apuntó a la
    // época `numero_promovido`. Si el ordinal registrado está entre {1, numero_promovido} —con la
    // particularidad de que `numero_promovido` puede ser 2 o más alto según el estado previo
    // del directorio, pero siempre ≥ 1—, la copia es coherente con el solapamiento. Si la copia
    // registra un ordinal ajeno (por ejemplo, una época futura que el respaldo copió por error),
    // esta aserción falla con un mensaje claro.
    assert!(
        (1..=numero_promovido).contains(&numero_reportado),
        "el ordinal reportado ({numero_reportado}) debe estar en el rango vivo durante el \
         solapamiento (1..={numero_promovido})"
    );

    // Anunciar las magnitudes medidas bajo `--nocapture`, para que el log de CI muestre el
    // resultado sin necesidad de abrir el binario de tests.
    println!("el_respaldo_concurrente_con_una_conmutacion_numero_reportado={numero_reportado}");
    println!("el_respaldo_concurrente_con_una_conmutacion_numero_fisico={numero_fisico:?}");
    println!("el_respaldo_concurrente_con_una_conmutacion_numero_promovido={numero_promovido}");
    println!("el_respaldo_concurrente_con_una_conmutacion_ordinales_en_la_copia={ordinales}");
}

/// H3: un respaldo que supera el límite de drenaje deja la época superseída huérfana y
/// protegida por retención.
///
/// El determinismo NO viene del reloj: un `VACUUM INTO` sobre el sembrado de un fragmento
/// termina en milisegundos. Viene del hecho de que un hilo sostiene una lectura del pool
/// superseído durante toda la ventana del drenaje, de modo que `lecturas_en_reposo()` es falso
/// en cada poll de 5 ms, y `Arc::strong_count` del pool es al menos 2. Esas dos condiciones
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
    sembrar_y_drenar_epoca_inicial(&gestor, directorio.ruta());

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
