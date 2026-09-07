//! Prueba de estrés de conmutación de época bajo lecturas concurrentes (etapa A-5, tarea 11).
//!
//! El PRD declara como criterio de QA de etapa una «Prueba de Consistencia en Modo WAL»: conmutar
//! la época viva con lecturas RAG simultáneas y demostrar que ninguna recibe `SQLITE_BUSY` ni
//! observa una época a medio construir. Hasta esta tarea ese criterio estaba **declarado y no
//! verificado**, que es la peor de las dos situaciones posibles: un criterio ausente se nota; uno
//! declarado sin ejecutar se confunde con uno cumplido. El razonamiento completo vive en
//! `docs/adr/adr-0030-...md`; aquí queda lo que hace falta para leer el código.
//!
//! # Por qué vive aislada y marcada `#[ignore]`
//!
//! Mide `/proc/self/fd`, que es **del proceso entero**: cualquier otro test corriendo en paralelo
//! abriría y cerraría archivos bajo la medición y la convertiría en ruido. Forzar la ejecución
//! secuencial de toda la batería está descartado como principio de diseño en D-33 de
//! `docs/bitacora-de-descartes.md`, así que el aislamiento se consigue de otra forma: `cargo`
//! ejecuta los binarios de test de integración **secuencialmente**, y cada `tests/*.rs` es su
//! propio binario y su propio proceso; al ser este el único test de su archivo, cuando corre no
//! hay ningún otro test vivo en su proceso. `#[ignore]` la saca de `cargo test --workspace`, y un
//! paso dedicado de `.github/workflows/ci.yml` la invoca por nombre para que el criterio del PRD
//! se verifique de verdad en cada empuje en vez de quedarse escrito.
//!
//! Solo funciona en Linux: `/proc/self/fd` no existe en otros sistemas, y este árbol ya asume
//! Linux como destino de despliegue y de CI (ver `CLAUDE.md`).

mod comun;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use comun::DirectorioTemporal;
use hexcell_core::fragmentacion::ConfiguracionDeFragmentacion;
use hexcell_core::recuperacion::{ConfiguracionDeRecuperacion, ContextoRecuperado};
use hexcell_storage::conocimiento::{
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA, SUFIJO_DE_ARCHIVO_SHM,
};
use hexcell_storage::drenaje::{DesenlaceDeDrenaje, drenar_epoca_superseida};
use hexcell_storage::error::ErrorDeAlmacen;
use hexcell_storage::migraciones::aplicar_migraciones_de_conocimiento;
use hexcell_storage::pools::{
    CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO, GestorDePools, SUFIJO_DE_ARCHIVO_WAL,
};
use hexcell_storage::promocion::{DesenlaceDePromocion, EpocaSuperseida, promover_epoca};
use hexcell_storage::recuperacion::recuperar_contexto;
use hexcell_storage::retencion::purgar_epocas_retiradas;
use rusqlite::Connection;

/// Anchura del pool de lecturas, igual al número de lectores a propósito.
///
/// `PoolDeConocimiento::con_lectura` reparte con `fetch_add % len` y luego toma un `Mutex`
/// **bloqueante**: con la anchura por omisión (2) los veinte lectores no serían veinte lecturas
/// simultáneas sino veinte lectores haciendo cola sobre dos cerrojos, y `SQLITE_BUSY` resultaría
/// imposible **por construcción** en vez de por corrección. La prueba no demostraría nada.
const ANCHURA_DE_LECTURAS: usize = 20;

/// Número de hilos lectores concurrentes exigido por el criterio de QA de la etapa.
const HILOS_LECTORES: usize = 20;

/// Fragmentos sembrados en cada época.
///
/// El fixture compartido siembra **uno**: un barrido coseno sobre él termina en microsegundos y la
/// ventana de solapamiento con la conmutación sería nula. Con esta cantidad el barrido dura lo
/// bastante como para que la conmutación caiga de verdad en medio de lecturas en vuelo, que es lo
/// único que hace significativa la aserción de ausencia de `SQLITE_BUSY`.
const FRAGMENTOS_POR_EPOCA: usize = 1_500;

/// Tamaño de fragmento, en caracteres exactos de [`UNIDAD_DE_CONTENIDO`].
///
/// La compuerta de integridad re-fragmenta el `contenido` del documento y exige que el número de
/// trozos coincida con el de filas de `fragmentos`. Repetir una unidad de este tamaño exacto, con
/// solapamiento cero, hace esa igualdad aritmética en vez de una casualidad del texto elegido.
const TAMANO_DE_FRAGMENTO: usize = 16;

/// Unidad repetida para construir el contenido del documento: exactamente [`TAMANO_DE_FRAGMENTO`]
/// caracteres ASCII.
const UNIDAD_DE_CONTENIDO: &str = "0123456789abcdef";

/// Dimensión de los vectores de embedding sembrados.
const DIMENSION_DE_EMBEDDING: usize = 768;

/// Marcador textual de la época previa a la conmutación.
///
/// `recuperar_contexto` no expone qué época sirvió un resultado, así que la procedencia se hace
/// **verificable por contenido**. Sin marcador, un resultado a medio construir sería
/// indistinguible de uno correcto y la pureza de época quedaría comprobada por suerte.
const MARCADOR_EPOCA_UNO: &str = "EPOCA-UNO";

/// Marcador textual de la época posterior a la conmutación.
const MARCADOR_EPOCA_DOS: &str = "EPOCA-DOS";

/// Techo de iteraciones por hilo lector: los lectores paran cuando el principal se lo indica, y
/// este techo solo evita que un fallo de coordinación cuelgue la batería en vez de fallar.
const MAXIMO_DE_ITERACIONES_POR_HILO: usize = 400;

/// Techo de regresión catastrófica para la conmutación, en milisegundos. **No es NFR-03**, que se
/// certifica estricto y sin hilos en `tests/promocion.rs`; aquí no se re-certifica (D-37 y
/// `adr-0030`), porque un muro de reloj de pared dentro de una prueba que mata de hambre a la CPU
/// a propósito mide el runner, no el sistema. Lo que este techo vigila es que la conmutación no
/// haya empezado a **esperar** por algo: E/S, un convoy de cerrojos, una espera de red. El número
/// sale de 44 corridas medidas el 2026-09-07 (sin restricción, fijadas a dos núcleos, y fijadas a
/// dos núcleos con carga externa) cuyo peor caso fue 0,047 ms: un segundo deja un margen de unas
/// 21.000 veces, fuera del alcance de cualquier expropiación del planificador, y sigue detectando,
/// porque queda un orden de magnitud por encima de la secuencia de promoción **entera** (88–140 ms
/// en esta máquina). Si el intercambio del puntero tarda más que la operación completa de la que es
/// una parte diminuta, no es ruido, es un defecto.
const TECHO_DE_REGRESION_CATASTROFICA_MS: f64 = 1_000.0;

/// Plazo de drenaje de la época superseída.
///
/// `drenar_epoca_superseida` cuenta desde el **instante del reemplazo**, no desde su invocación:
/// esperar a los lectores y unirlos ya consume parte del plazo. El límite por omisión de 10 s es
/// un presupuesto de producción, no de una prueba que satura la máquina a propósito; se usa uno
/// holgado para no confundir «lento por carga» con «no drena».
const PLAZO_DE_DRENAJE: Duration = Duration::from_secs(120);

/// Procedencia de un `ContextoRecuperado`, deducida del marcador textual de sus fragmentos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Procedencia {
    /// Todos los fragmentos devueltos llevan el marcador de la época previa.
    EpocaUno,
    /// Todos los fragmentos devueltos llevan el marcador de la época posterior.
    EpocaDos,
    /// El resultado no fue homogéneo: mezcla marcadores, o alguno no lleva ninguno. Es
    /// exactamente lo que delataría una lectura servida por una época a medio construir.
    Impura,
    /// El resultado llegó vacío. No debería ocurrir con ambas épocas sembradas y el umbral por
    /// debajo de la similitud exacta, así que se cuenta aparte en vez de asimilarse a un acierto.
    Vacia,
}

/// Clasifica un contexto recuperado por el marcador de sus fragmentos.
fn clasificar(contexto: &ContextoRecuperado) -> Procedencia {
    let fragmentos = contexto.fragmentos();
    if fragmentos.is_empty() {
        return Procedencia::Vacia;
    }

    let de_uno = fragmentos
        .iter()
        .filter(|f| f.texto.contains(MARCADOR_EPOCA_UNO))
        .count();
    let de_dos = fragmentos
        .iter()
        .filter(|f| f.texto.contains(MARCADOR_EPOCA_DOS))
        .count();

    if de_uno == fragmentos.len() {
        Procedencia::EpocaUno
    } else if de_dos == fragmentos.len() {
        Procedencia::EpocaDos
    } else {
        Procedencia::Impura
    }
}

/// Indica si un error de almacén es una contención de SQLite (`SQLITE_BUSY` / `SQLITE_LOCKED`).
///
/// Se comprueba el código del motor y no el texto del mensaje: una aserción sobre el mensaje
/// pasaría a verde el día que cambie la redacción, no el día que se arregle la contención.
fn es_contencion_de_sqlite(error: &ErrorDeAlmacen) -> bool {
    match error {
        ErrorDeAlmacen::Sqlite { causa, .. } => match causa {
            rusqlite::Error::SqliteFailure(codigo, _) => matches!(
                codigo.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ),
            _ => false,
        },
        _ => false,
    }
}

/// Cuenta los descriptores abiertos por **este** proceso, con el mismo patrón de lectura de
/// `/proc` que `crates/hexcell/tests/rss_linea_base.rs` usa para medir en vez de estimar.
fn descriptores_abiertos() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("leer /proc/self/fd: esta prueba solo corre en Linux")
        .count()
}

/// Cuenta cuántos descriptores de **este** proceso apuntan al archivo indicado.
///
/// Es la única señal de simultaneidad real disponible sin tocar `crates/hexcell-storage/src/**`:
/// cada conexión de lectura del pool abre el archivo de la época al construirse, así que este
/// número **son** las conexiones SQLite vivas sobre esa época. Se descartó el medidor de pico de
/// hilos alrededor de `recuperar_contexto` (D-36): `con_lectura` toma un `Mutex` bloqueante, un
/// hilo en cola cuenta igual que uno leyendo, y el pico llegaría a veinte incluso con dos
/// conexiones; sería una guarda que aparenta comprobar lo que no comprueba.
fn conexiones_vivas_sobre(ruta: &Path) -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("leer /proc/self/fd: esta prueba solo corre en Linux")
        .filter_map(|entrada| entrada.ok())
        .filter(|entrada| std::fs::read_link(entrada.path()).is_ok_and(|destino| destino == ruta))
        .count()
}

/// Siembra un archivo de staging con muchos fragmentos, todos marcados con `marcador`.
///
/// Se mantiene local a este archivo a propósito: no es una tercera copia de
/// `comun::preparar_staging_valido`, sino un fixture con un fin distinto —barrido medible y
/// procedencia verificable por contenido— que ninguna otra prueba necesita.
fn sembrar_staging_marcado(
    ruta_datos: &Path,
    dimension: usize,
    marcador: &str,
) -> ConfiguracionDeFragmentacion {
    assert_eq!(
        UNIDAD_DE_CONTENIDO.chars().count(),
        TAMANO_DE_FRAGMENTO,
        "la unidad de contenido debe medir exactamente un fragmento"
    );

    let ruta_staging = ruta_datos.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO_EN_SOMBRA);
    let mut conexion = Connection::open(&ruta_staging).expect("abrir base de staging");
    conexion.execute("PRAGMA foreign_keys = ON;", []).unwrap();
    // La ingesta real abre staging en lectura y escritura, que fija WAL desde la primera conexión.
    // Replicarlo aquí evita que el cambio de modo de diario (delete -> wal), que sí exige
    // exclusividad, se confunda con la contención que esta prueba quiere medir.
    conexion
        .query_row("PRAGMA journal_mode = WAL", [], |fila| {
            fila.get::<_, String>(0)
        })
        .unwrap();
    aplicar_migraciones_de_conocimiento(&conexion).expect("migrar staging");

    conexion
        .execute(
            "UPDATE metadatos_de_epoca SET dimension_de_embedding = ?1 WHERE id = 1",
            rusqlite::params![dimension as i64],
        )
        .unwrap();

    let contenido = UNIDAD_DE_CONTENIDO.repeat(FRAGMENTOS_POR_EPOCA);
    let vector: Vec<f32> = vec![1.0; dimension];
    let vector_bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();

    // Un solo documento con ordinales 0..N-1 contiguos: la compuerta de integridad comprueba la
    // contigüidad sobre el conjunto GLOBAL de ordinales, así que repartir los fragmentos entre
    // varios documentos rompería esa comprobación por una razón que nada tiene que ver con lo que
    // esta prueba quiere ejercitar.
    let transaccion = conexion
        .transaction()
        .expect("abrir transacción de siembra");
    transaccion
        .execute(
            "INSERT INTO documentos (id, referencia_externa, titulo, contenido, actualizado_ms) VALUES (1, 'ref_1', 'Documento marcado', ?1, 1000)",
            rusqlite::params![contenido],
        )
        .unwrap();

    {
        let mut insertar_fragmento = transaccion
            .prepare(
                "INSERT INTO fragmentos (id, id_documento, ordinal, texto) VALUES (?1, 1, ?2, ?3)",
            )
            .unwrap();
        let mut insertar_vector = transaccion
            .prepare("INSERT INTO vectores_de_fragmento (id_fragmento, vector) VALUES (?1, ?2)")
            .unwrap();

        for indice in 0..FRAGMENTOS_POR_EPOCA {
            let id = (indice + 1) as i64;
            let texto = format!("{marcador}-fragmento-{indice}");
            insertar_fragmento
                .execute(rusqlite::params![id, indice as i64, texto])
                .unwrap();
            insertar_vector
                .execute(rusqlite::params![id, &vector_bytes])
                .unwrap();
        }
    }

    transaccion
        .execute(
            "INSERT INTO sonda_semantica (id, texto_de_la_sonda, vector, umbral_de_aceptacion, registrada_ms) VALUES (1, 'consulta', ?1, 0.5, 1000)",
            rusqlite::params![vector_bytes],
        )
        .unwrap();
    transaccion.commit().expect("confirmar siembra");

    drop(conexion);

    ConfiguracionDeFragmentacion {
        tamano_de_fragmento: TAMANO_DE_FRAGMENTO,
        solapamiento: 0,
    }
}

/// Extrae el desenlace de una promoción que debe haber sido `Promovida`.
fn desglosar_promocion(desenlace: DesenlaceDePromocion) -> (i64, PathBuf, EpocaSuperseida, f64) {
    match desenlace {
        DesenlaceDePromocion::Promovida {
            numero_de_epoca,
            ruta_del_archivo,
            epoca_superseida,
            duracion_de_conmutacion_ms,
        } => (
            numero_de_epoca,
            ruta_del_archivo,
            epoca_superseida,
            duracion_de_conmutacion_ms,
        ),
        DesenlaceDePromocion::Abortada { motivo } => {
            panic!("la promoción no debió abortar: {motivo:?}")
        }
    }
}

/// Ruta de un acompañante (`-wal` o `-shm`) de una base de datos. Se construye sobre `OsString` y
/// no con `with_extension`, que reemplazaría el `.db` del nombre canónico en vez de añadir sufijo.
fn ruta_acompanante(ruta_base: &Path, sufijo: &str) -> PathBuf {
    let mut ruta = ruta_base.as_os_str().to_owned();
    ruta.push(sufijo);
    PathBuf::from(ruta)
}

/// Conmuta la época viva mientras veinte lecturas RAG están en vuelo sobre un pool de anchura 20.
///
/// Cubre AC-1 a AC-6: concurrencia real, solapamiento demostrado por marcador, pureza de época por
/// lectura, cero contenciones de SQLite, ausencia de diarios huérfanos tras drenar y purgar, las
/// dos duraciones medidas por separado y el retorno de los descriptores de archivo a su línea base.
#[test]
#[ignore]
fn estres_conmutacion_veinte_lecturas_concurrentes() {
    let temp = DirectorioTemporal::nuevo("estres-conmutacion");

    // AC-1: la anchura del pool es la condición que hace concurrentes a las veinte lecturas.
    let gestor = Arc::new(
        GestorDePools::abrir_con_anchura_de_conocimiento(temp.ruta(), ANCHURA_DE_LECTURAS)
            .expect("abrir el gestor con anchura de lecturas ampliada"),
    );
    // La anchura no basta con configurarla: hay que afirmarla, y contra el criterio, no contra la
    // constante que la fija. `assert_eq!(anchura_efectiva, ANCHURA_DE_LECTURAS)` compararía la
    // constante consigo misma y seguiría en verde con anchura 2, dejando la prueba vacía: los
    // veinte lectores harían cola sobre dos cerrojos, `SQLITE_BUSY` sería imposible por
    // construcción y el criterio del PRD quedaría certificado por CI sin haberse ejercitado.
    let anchura_efectiva = gestor.anchura_de_lecturas_de_conocimiento();
    assert!(
        anchura_efectiva >= HILOS_LECTORES,
        "el criterio de QA exige una conexión de lectura viva por cada uno de los {HILOS_LECTORES} \
         lectores concurrentes; con anchura efectiva {anchura_efectiva} los lectores se serializan \
         sobre los cerrojos del pool y la prueba dejaría de demostrar nada"
    );
    assert!(
        anchura_efectiva > CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO,
        "la anchura efectiva {anchura_efectiva} no supera la de omisión \
         ({CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO}): esta prueba solo es significativa sobre un pool \
         deliberadamente más ancho que el de producción"
    );

    // Época 1: la que estará viva cuando empiecen las lecturas.
    let configuracion_uno =
        sembrar_staging_marcado(temp.ruta(), DIMENSION_DE_EMBEDDING, MARCADOR_EPOCA_UNO);
    let (_numero_uno, ruta_epoca_uno, superseida_inicial, _ms_inicial) = desglosar_promocion(
        promover_epoca(&gestor, temp.ruta(), &configuracion_uno, 10_000)
            .expect("promover la época marcada como uno"),
    );

    // La base inicial queda superseída por esta primera promoción y sus conexiones siguen abiertas.
    // Hay que drenarla ANTES de tomar la línea base de descriptores: si se contara con ella dentro,
    // la cuenta final quedaría por debajo de la línea base y la aserción de AC-6 fallaría por un
    // pool que esta prueba ni siquiera estaba midiendo.
    let drenaje_inicial = drenar_epoca_superseida(superseida_inicial, PLAZO_DE_DRENAJE)
        .expect("drenar la base inicial superseída");
    let constancia_inicial = match drenaje_inicial {
        DesenlaceDeDrenaje::Drenada { constancia, .. } => constancia,
        otro => panic!("la base inicial debió drenar limpiamente, se obtuvo: {otro:?}"),
    };
    gestor.retirar_epoca_en_uso(&constancia_inicial);

    // Purga en vacío ANTES de tomar la línea base. No hay nada que purgar —la época 1 es la viva—
    // y ese es el punto: iguala el estado de la caché de descriptores diferidos del VFS de SQLite
    // entre las dos mediciones de AC-6. Ese VFS no cierra de inmediato el descriptor de un archivo
    // sobre el que otra conexión del mismo proceso mantiene cerrojos POSIX (cerrarlo borraría los
    // cerrojos ajenos, el defecto histórico de `close()`): lo aparca por inodo y lo reutiliza en la
    // apertura siguiente. La purga abre una conexión transitoria sobre la época viva, así que la
    // PRIMERA sobre un inodo deja un descriptor aparcado y las siguientes no. Sin esta purga
    // previa, la línea base se tomaría en frío y la cuenta final en caliente, y la aserción mediría
    // el calentamiento de esa caché en vez del ciclo de vida de los pools. De paso queda
    // demostrado que la purga es neutra en descriptores.
    let purga_en_vacio =
        purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purga en vacío previa");
    assert!(
        purga_en_vacio.epocas_purgadas.is_empty(),
        "la purga previa no debía eliminar nada: {:?}",
        purga_en_vacio.epocas_purgadas
    );

    // Época 2 en staging, sembrada ANTES de soltar a los lectores: sembrar mil quinientos
    // fragmentos dura bastante más que la conmutación, y hacerlo con los lectores ya girando
    // alargaría su bucle sin añadir nada a lo que se quiere medir.
    let configuracion_dos =
        sembrar_staging_marcado(temp.ruta(), DIMENSION_DE_EMBEDDING, MARCADOR_EPOCA_DOS);

    let configuracion_de_recuperacion = ConfiguracionDeRecuperacion {
        maximo_de_fragmentos: 5,
        umbral_de_similitud: 0.5,
    };
    let vector_de_consulta: Vec<f32> = vec![1.0; DIMENSION_DE_EMBEDDING];

    let lecturas_de_epoca_uno = Arc::new(AtomicUsize::new(0));
    let lecturas_de_epoca_dos = Arc::new(AtomicUsize::new(0));
    let lecturas_impuras = Arc::new(AtomicUsize::new(0));
    let lecturas_vacias = Arc::new(AtomicUsize::new(0));
    let contenciones_de_sqlite = Arc::new(AtomicUsize::new(0));
    let fallos_de_lectura = Arc::new(AtomicUsize::new(0));
    let detener = Arc::new(AtomicBool::new(false));

    // La barrera incluye al hilo principal (de ahí el +1): los veinte lectores no arrancan de forma
    // escalonada según los planifique el sistema, sino todos en el mismo instante, que es la única
    // manera de que «veinte lecturas simultáneas» signifique algo.
    let barrera = Arc::new(Barrier::new(HILOS_LECTORES + 1));

    // Dos canales en vez de esperas activas sobre contadores: si un lector entrara en pánico, una
    // espera activa dejaría al principal girando para siempre, mientras que un receptor cuyos
    // emisores murieron devuelve error y el `join` propaga el pánico real. Un test colgado no dice
    // qué se rompió; uno que falla, sí.
    let (tx_primera_lectura, rx_primera_lectura) = mpsc::channel::<()>();
    let (tx_epoca_dos, rx_epoca_dos) = mpsc::channel::<()>();

    let mut hilos = Vec::with_capacity(HILOS_LECTORES);
    for _ in 0..HILOS_LECTORES {
        let gestor_hilo = Arc::clone(&gestor);
        let barrera_hilo = Arc::clone(&barrera);
        let detener_hilo = Arc::clone(&detener);
        let de_uno = Arc::clone(&lecturas_de_epoca_uno);
        let de_dos = Arc::clone(&lecturas_de_epoca_dos);
        let impuras = Arc::clone(&lecturas_impuras);
        let vacias = Arc::clone(&lecturas_vacias);
        let contenciones = Arc::clone(&contenciones_de_sqlite);
        let fallos = Arc::clone(&fallos_de_lectura);
        let tx_primera = tx_primera_lectura.clone();
        let tx_dos = tx_epoca_dos.clone();
        let consulta = vector_de_consulta.clone();
        let recuperacion = configuracion_de_recuperacion.clone();

        hilos.push(thread::spawn(move || {
            barrera_hilo.wait();

            let mut primera_anunciada = false;
            for _ in 0..MAXIMO_DE_ITERACIONES_POR_HILO {
                if detener_hilo.load(Ordering::Relaxed) {
                    break;
                }

                match recuperar_contexto(&gestor_hilo, &consulta, &recuperacion) {
                    Ok(contexto) => {
                        match clasificar(&contexto) {
                            Procedencia::EpocaUno => {
                                de_uno.fetch_add(1, Ordering::Relaxed);
                            }
                            Procedencia::EpocaDos => {
                                de_dos.fetch_add(1, Ordering::Relaxed);
                                // Se anuncia CADA lectura de la época nueva, no solo la primera:
                                // el principal espera veinte para asegurarse de que las veinte
                                // conexiones del pool recién abierto han sido ejercitadas, y no
                                // solo la que tocó por reparto.
                                let _ = tx_dos.send(());
                            }
                            Procedencia::Impura => {
                                impuras.fetch_add(1, Ordering::Relaxed);
                            }
                            Procedencia::Vacia => {
                                vacias.fetch_add(1, Ordering::Relaxed);
                            }
                        }

                        if !primera_anunciada {
                            primera_anunciada = true;
                            let _ = tx_primera.send(());
                        }
                    }
                    Err(error) => {
                        if es_contencion_de_sqlite(&error) {
                            contenciones.fetch_add(1, Ordering::Relaxed);
                        }
                        fallos.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }

    // El principal suelta sus copias de los emisores: mientras las conserve, un receptor nunca
    // vería el cierre del canal aunque murieran los veinte lectores.
    drop(tx_primera_lectura);
    drop(tx_epoca_dos);

    barrera.wait();

    // Esperar a que los veinte hilos hayan completado al menos una lectura. Todas son forzosamente
    // de la época uno, porque la conmutación todavía no ha empezado: así el solapamiento de AC-2
    // queda garantizado por construcción y no por la suerte del planificador.
    for _ in 0..HILOS_LECTORES {
        if rx_primera_lectura.recv().is_err() {
            break;
        }
    }

    // AC-1, comprobación de hecho y no de intención: contar los descriptores que apuntan al
    // archivo de la época viva mide las conexiones SQLite realmente abiertas sobre ella. La
    // anchura afirmada arriba dice lo que se pidió; esto dice lo que hay.
    let ruta_epoca_uno_canonica =
        std::fs::canonicalize(&ruta_epoca_uno).expect("resolver la ruta física de la época viva");
    let conexiones_vivas = conexiones_vivas_sobre(&ruta_epoca_uno_canonica);
    assert!(
        conexiones_vivas >= HILOS_LECTORES,
        "solo hay {conexiones_vivas} conexiones SQLite vivas sobre la época viva y el criterio \
         exige al menos {HILOS_LECTORES}: con menos, los veinte lectores no leen a la vez, hacen \
         cola"
    );

    // AC-6: línea base de descriptores tomada con las veinte conexiones de la época viva ya
    // ejercitadas y justo antes de conmutar.
    let descriptores_linea_base = descriptores_abiertos();

    // AC-5, medición ancha: desde la invocación de la promoción hasta que la primera lectura
    // servida por la época nueva devuelve. Es una magnitud DISTINTA de la conmutación del
    // `ArcSwap`, e incluye el sellado, el renombrado y la apertura del pool nuevo.
    let instante_hasta_primera_lectura = Instant::now();
    let desenlace_de_promocion = promover_epoca(&gestor, temp.ruta(), &configuracion_dos, 20_000)
        .expect("promover la época marcada como dos");
    let contexto_posterior =
        recuperar_contexto(&gestor, &vector_de_consulta, &configuracion_de_recuperacion)
            .expect("primera lectura posterior a la conmutación");
    let ms_hasta_primera_lectura_servida =
        instante_hasta_primera_lectura.elapsed().as_secs_f64() * 1000.0;

    let (numero_dos, ruta_epoca_dos, epoca_superseida, ms_de_conmutacion) =
        desglosar_promocion(desenlace_de_promocion);

    assert_eq!(
        clasificar(&contexto_posterior),
        Procedencia::EpocaDos,
        "la primera lectura posterior a la conmutación debe venir de la época nueva"
    );

    // Esperar a que la época nueva haya servido tantas lecturas como conexiones tiene su pool.
    // Además de reforzar el solapamiento, iguala su estado con el que tenía el pool anterior al
    // tomarse la línea base: SQLite abre el `-wal` y el `-shm` de una conexión de solo lectura en
    // su primera lectura, no al abrirla, y comparar un pool ejercitado con uno recién abierto
    // compararía dos cosas distintas.
    for _ in 0..ANCHURA_DE_LECTURAS {
        if rx_epoca_dos.recv().is_err() {
            break;
        }
    }
    detener.store(true, Ordering::Relaxed);

    for hilo in hilos {
        hilo.join().expect("un hilo lector entró en pánico");
    }

    let observadas_de_uno = lecturas_de_epoca_uno.load(Ordering::Relaxed);
    let observadas_de_dos = lecturas_de_epoca_dos.load(Ordering::Relaxed);
    let observadas_impuras = lecturas_impuras.load(Ordering::Relaxed);
    let observadas_vacias = lecturas_vacias.load(Ordering::Relaxed);
    let observadas_contenciones = contenciones_de_sqlite.load(Ordering::Relaxed);
    let observados_fallos = fallos_de_lectura.load(Ordering::Relaxed);

    // AC-2: ambos marcadores observados. Sin las dos mitades, la prueba habría corrido sin que la
    // conmutación llegara a caer dentro del bucle de lecturas y no demostraría solapamiento alguno.
    assert!(
        observadas_de_uno > 0,
        "ninguna lectura vino de la época previa: no hubo solapamiento real"
    );
    assert!(
        observadas_de_dos > 0,
        "ninguna lectura vino de la época posterior: no hubo solapamiento real"
    );

    // AC-2 (invariante de pureza): ninguna lectura mezcló épocas ni volvió vacía.
    assert_eq!(
        observadas_impuras, 0,
        "hubo {observadas_impuras} lecturas que mezclaron marcadores de época"
    );
    assert_eq!(
        observadas_vacias, 0,
        "hubo {observadas_vacias} lecturas vacías con ambas épocas sembradas"
    );

    // AC-3: cero contenciones de SQLite, y cero fallos de cualquier clase. Comprobar solo las
    // contenciones dejaría pasar una lectura que fallara por otro motivo durante la conmutación,
    // que es igual de grave para la invariante «ninguna lectura en vuelo falla».
    assert_eq!(
        observadas_contenciones, 0,
        "se observaron {observadas_contenciones} contenciones SQLITE_BUSY durante la conmutación"
    );
    assert_eq!(
        observados_fallos, 0,
        "se observaron {observados_fallos} lecturas fallidas durante la conmutación"
    );

    // AC-5, medición estrecha. `duracion_de_conmutacion_ms` abarca el intercambio del `ArcSwap`
    // **más** la toma de un cerrojo del pool nuevo y la consulta de vitalidad que sirve la primera
    // lectura de la época nueva: justo el tramo que NFR-03 define, y por eso el campo correcto para
    // el requisito y a la vez el equivocado para acotarlo **aquí**, donde esa consulta tiene que
    // ganarle un cerrojo a veinte hilos que saturan el pool a propósito. Un muro de 10 ms en esta
    // prueba mediría la suerte del planificador, no la latencia del sistema: una intermitencia
    // cableada en CI que estallaría semanas después sobre trabajo ajeno. NFR-03 ya está certificado,
    // estricto y sin hilos, en `tests/promocion.rs`, que esta tarea no toca; duplicarlo bajo
    // contención artificial no añadiría certeza y pagaría fragilidad por ella (decisión humana del
    // 2026-09-07, D-37). Aquí ambas duraciones se **reportan** y solo se afirma el techo de
    // catástrofe.
    assert!(
        ms_de_conmutacion.is_finite() && ms_de_conmutacion >= 0.0,
        "la duración de conmutación no es un número utilizable: {ms_de_conmutacion}"
    );
    assert!(
        ms_de_conmutacion < TECHO_DE_REGRESION_CATASTROFICA_MS,
        "la conmutación tardó {ms_de_conmutacion} ms y supera el techo de regresión catastrófica de \
         {TECHO_DE_REGRESION_CATASTROFICA_MS} ms: a esa escala no es ruido del planificador, la \
         conmutación espera por algo (E/S, convoy). NFR-03 no se evalúa aquí, vive en promocion.rs"
    );
    assert!(
        ms_hasta_primera_lectura_servida.is_finite() && ms_hasta_primera_lectura_servida >= 0.0,
        "la latencia hasta la primera lectura servida no es utilizable: {ms_hasta_primera_lectura_servida}"
    );

    // AC-4, primera mitad: drenar la época superseída deja su pool cerrado y sus diarios
    // consolidados. Los lectores ya están unidos, así que el predicado de dos lados se cumple sin
    // reintentos.
    let numero_superseido = epoca_superseida.numero_de_epoca();
    let desenlace_de_drenaje = drenar_epoca_superseida(epoca_superseida, PLAZO_DE_DRENAJE)
        .expect("drenar la época superseída por la conmutación");
    let constancia = match desenlace_de_drenaje {
        DesenlaceDeDrenaje::Drenada { constancia, .. } => constancia,
        otro => panic!("la época superseída debió drenar limpiamente, se obtuvo: {otro:?}"),
    };
    assert!(
        gestor.retirar_epoca_en_uso(&constancia).is_some(),
        "la constancia legítima debe retirar la época del inventario en uso"
    );
    assert_eq!(
        numero_superseido,
        Some(1),
        "la época superseída por la segunda promoción es la número 1"
    );
    assert_eq!(numero_dos, 2, "la época promovida es la número 2");

    // AC-4, segunda mitad: con ventana de retención 0 la época drenada es purgable, que es el mismo
    // precedente que ya usa `tests/retencion.rs` para forzar la eliminación efectiva en vez de
    // dejarla dentro de la ventana y no poder afirmar nada sobre sus archivos.
    let desenlace_de_purga =
        purgar_epocas_retiradas(&gestor, temp.ruta(), 0).expect("purgar épocas retiradas");
    assert_eq!(
        desenlace_de_purga.epocas_purgadas.len(),
        1,
        "solo la época 1 debía purgarse: {:?}",
        desenlace_de_purga.epocas_purgadas
    );
    assert_eq!(desenlace_de_purga.epocas_purgadas[0].numero_de_epoca, 1);

    assert!(
        !ruta_epoca_uno.exists(),
        "el archivo de la época superseída sigue en disco: {ruta_epoca_uno:?}"
    );
    let wal_superseido = ruta_acompanante(&ruta_epoca_uno, SUFIJO_DE_ARCHIVO_WAL);
    let shm_superseido = ruta_acompanante(&ruta_epoca_uno, SUFIJO_DE_ARCHIVO_SHM);
    assert!(
        !wal_superseido.exists(),
        "quedó un diario -wal huérfano de la época superseída: {wal_superseido:?}"
    );
    assert!(
        !shm_superseido.exists(),
        "quedó un archivo -shm huérfano de la época superseída: {shm_superseido:?}"
    );
    assert!(
        ruta_epoca_dos.exists(),
        "la época viva debe seguir en disco: {ruta_epoca_dos:?}"
    );

    // AC-6: un pool superseído que no cerrara sus conexiones no se notaría en ninguna aserción
    // funcional —las lecturas seguirían siendo correctas— y aun así agotaría el presupuesto de
    // descriptores de la célula con cada conmutación.
    let descriptores_finales = descriptores_abiertos();
    assert_eq!(
        descriptores_finales, descriptores_linea_base,
        "los descriptores de archivo no volvieron a su línea base tras drenar y purgar"
    );

    println!("estres_conmutacion_lecturas_de_epoca_uno={observadas_de_uno}");
    println!("estres_conmutacion_lecturas_de_epoca_dos={observadas_de_dos}");
    println!("estres_conmutacion_contenciones_sqlite={observadas_contenciones}");
    println!("estres_conmutacion_duracion_de_conmutacion_ms={ms_de_conmutacion:.3}");
    println!(
        "estres_conmutacion_hasta_primera_lectura_servida_ms={ms_hasta_primera_lectura_servida:.3}"
    );
    println!("estres_conmutacion_anchura_efectiva={anchura_efectiva}");
    println!("estres_conmutacion_conexiones_vivas_sobre_la_epoca={conexiones_vivas}");
    println!("estres_conmutacion_descriptores_linea_base={descriptores_linea_base}");
    println!("estres_conmutacion_descriptores_finales={descriptores_finales}");
}
