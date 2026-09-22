//! Pruebas externas de `cell status` y `cell list`, a través de `ejecutar_con_efectos`: lo que se
//! fija es lo que el operador ve y el código con el que sale el proceso.
//!
//! Tres reglas las hacen capaces de ponerse rojas. Primera: **cada código de discrepancia tiene un
//! accesorio que lo dispara y otro que no**, y cada prueba exige además que los otros cuatro NO
//! aparezcan; sin eso, una implementación que los empuje siempre pasa las cinco pruebas de
//! disparo. Segunda: **ninguna espera es un `join` ciego**; las peticiones viajan por un canal
//! leído con `recv_timeout` y el silencio se afirma con un guion pendiente sin consumir, así que
//! una petición que sobra o falta pone la prueba roja dentro del límite en vez de colgarla.
//! Tercera: **la salida se compara entera**, con igualdad; un `contains("a") || contains("b")`
//! cuyas dos ramas se mueven juntas bajo una mutación no prueba nada.

mod comun;

use std::time::Duration;

use hexcell_admin::almacen_plano_de_control::{AlmacenDelPlanoDeControl, etiqueta_persistida};
use hexcell_admin::argumentos::analizar;
use hexcell_admin::ciclo_de_vida::DatosDeSondeo;
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::{TEXTO_DE_FUENTE_DOCKER_FALLIDA, ejecutar_con_efectos};
use hexcell_admin::docker::{ClienteDocker, InventarioDocker};
use hexcell_admin::estado_de_celula::EstadoDeCelula;
use hexcell_admin::salida::Salida;

use comun::{
    AlmacenTemporal, Guion, PeticionRecibida, ServidorDockerFalso, exigir_silencio,
    secuencia_recibida, servir_guiones,
};

/// Reloj inyectado: distinto de cualquier valor que devuelva el reloj del sistema.
const RELOJ_INYECTADO: i64 = 1_700_000_000_000;

/// Los cinco códigos, escritos aquí y no leídos de producción: si la aserción tomara la cadena del
/// código bajo prueba, una mutación movería los dos lados a la vez.
const CODIGOS: [&str; 5] = ["DISC-01", "DISC-02", "DISC-03", "DISC-04", "DISC-05"];

const CORRIENDO: &[u8] = br#"{"State":{"Status":"running"}}"#;
const DETENIDO: &[u8] = br#"{"State":{"Status":"exited"}}"#;
/// Inspección del núcleo con la red y el puerto de salud que la sonda corta necesita leer.
const NUCLEO_CON_RED: &[u8] = br#"{"State":{"Status":"running"},"NetworkSettings":{"Networks":{"red-del-operador":{"NetworkID":"n1"}}},"Config":{"Env":["HEXCELL_DIRECCION_SALUD=0.0.0.0:9099"]}}"#;

fn cuerpo(cuerpo: &'static [u8]) -> Guion {
    Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo,
    }
}

fn ausente() -> Guion {
    Guion::SinCuerpo {
        estado: 404,
        razon: "Not Found",
    }
}

fn sin_cuerpo() -> Guion {
    Guion::SinCuerpo {
        estado: 204,
        razon: "No Content",
    }
}

/// Los dos `inspect` del comando con ambos contenedores corriendo, los cinco pasos de la sonda
/// corta —reinspección del núcleo, creación, arranque, espera y limpieza— y un guion de sobra con
/// el que afirmar silencio.
fn guiones_coherentes(veredicto: &'static [u8]) -> Vec<Guion> {
    vec![
        cuerpo(CORRIENDO),
        cuerpo(CORRIENDO),
        cuerpo(NUCLEO_CON_RED),
        Guion::ConCuerpo {
            estado: 201,
            razon: "Created",
            cuerpo: br#"{"Id":"sonda1","Warnings":[]}"#,
        },
        sin_cuerpo(),
        cuerpo(veredicto),
        sin_cuerpo(),
        sin_cuerpo(),
    ]
}

/// Crea y migra el almacén y siembra opcionalmente la fila de `c1`.
///
/// La fila se inserta con `rusqlite` para que `transiciones` quede vacía: así las guardas de sólo
/// lectura pueden exigir los mismos conteos sin arrastrar filas del accesorio.
fn preparar_almacen(temporal: &AlmacenTemporal, fila: Option<EstadoDeCelula>) {
    drop(AlmacenDelPlanoDeControl::abrir(temporal.ruta()).expect("crear el almacén del accesorio"));
    if let Some(estado) = fila {
        rusqlite::Connection::open(temporal.ruta())
            .expect("abrir el accesorio")
            .execute(
                "INSERT INTO celulas (id, estado, motivo, actualizado_ms) VALUES ('c1', ?1, 'sembrada', ?2)",
                rusqlite::params![etiqueta_persistida(estado), RELOJ_INYECTADO],
            )
            .expect("sembrar la fila de celulas");
    }
}

/// Inserta una sustitución del accesorio: esta tarea crea la tabla y sólo la lee, el escritor de
/// producción es la tarea 13 (cell rebind).
fn sembrar_sustitucion(temporal: &AlmacenTemporal, motivo: &str, registrado_ms: i64) {
    rusqlite::Connection::open(temporal.ruta())
        .expect("abrir el accesorio")
        .execute(
            "INSERT INTO sustituciones (id_celula, motivo, registrado_ms) VALUES ('c1', ?1, ?2)",
            rusqlite::params![motivo, registrado_ms],
        )
        .expect("sembrar la sustitución");
}

/// Conteos de las tres tablas, para la guarda de sólo lectura.
fn conteos(temporal: &AlmacenTemporal) -> (i64, i64, i64) {
    let conexion = rusqlite::Connection::open_with_flags(
        temporal.ruta(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("abrir el almacén para contarlo");
    let contar = |tabla: &str| -> i64 {
        conexion
            .query_row(&format!("SELECT count(*) FROM {tabla}"), [], |f| f.get(0))
            .unwrap()
    };
    (
        contar("celulas"),
        contar("transiciones"),
        contar("sustituciones"),
    )
}

fn correr(
    snippet: &[&str],
    cliente: &ClienteDocker,
    inventario: &InventarioDocker,
    ruta_almacen: &str,
) -> (CodigoDeSalida, String, String) {
    let argumentos: Vec<String> = snippet.iter().map(|s| (*s).to_string()).collect();
    let datos = DatosDeSondeo {
        imagen: "sonda-de-prueba:1".to_string(),
        limite_segundos: 2,
    };
    let mut estandar: Vec<u8> = Vec::new();
    let mut diagnostico: Vec<u8> = Vec::new();
    let codigo = {
        let mut salida = Salida::nueva(&mut estandar, &mut diagnostico);
        ejecutar_con_efectos(
            analizar(&argumentos),
            &mut salida,
            cliente,
            inventario,
            ruta_almacen,
            RELOJ_INYECTADO,
            datos,
        )
    };
    (
        codigo,
        String::from_utf8(estandar).expect("UTF-8 en el estándar"),
        String::from_utf8(diagnostico).expect("UTF-8 en el diagnóstico"),
    )
}

struct Corrida {
    codigo: CodigoDeSalida,
    estandar: String,
    diagnostico: String,
    receptor: std::sync::mpsc::Receiver<PeticionRecibida>,
}

impl Corrida {
    /// Exige `Fallo`, que el diagnóstico nombre `codigo` y que NO nombre ninguno de los otros
    /// cuatro: la segunda mitad es la que mata la mutación «emitir siempre este código».
    fn exige_solo(&self, codigo: &str) {
        assert_eq!(
            self.codigo,
            CodigoDeSalida::Fallo,
            "una discrepancia da Fallo; diag: {:?}",
            self.diagnostico
        );
        assert!(
            self.diagnostico.contains(codigo),
            "se esperaba {codigo}: {:?}",
            self.diagnostico
        );
        for otro in CODIGOS.iter().filter(|c| **c != codigo) {
            assert!(
                !self.diagnostico.contains(otro),
                "{otro} no debía aparecer junto a {codigo}: {:?}",
                self.diagnostico
            );
        }
    }

    /// Exige `Exito` y que NINGÚN código de discrepancia aparezca.
    fn exige_sin_discrepancias(&self) {
        assert_eq!(
            self.codigo,
            CodigoDeSalida::Exito,
            "diag: {:?}",
            self.diagnostico
        );
        for codigo in CODIGOS {
            assert!(
                !self.diagnostico.contains(codigo),
                "{codigo} no debía aparecer: {:?}",
                self.diagnostico
            );
        }
    }
}

/// Monta el accesorio completo y corre `cell status --id c1` contra él.
fn correr_estado(etiqueta: &str, fila: Option<EstadoDeCelula>, guiones: Vec<Guion>) -> Corrida {
    let servidor = ServidorDockerFalso::nuevo(etiqueta);
    let ruta = servidor.ruta();
    let almacen = AlmacenTemporal::nuevo(etiqueta);
    preparar_almacen(&almacen, fila);
    let receptor = servir_guiones(servidor, guiones);
    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_secs(10));
    let inventario = InventarioDocker::nuevo(ruta, Duration::from_secs(10));
    let (codigo, estandar, diagnostico) = correr(
        &["cell", "status", "--id", "c1"],
        &cliente,
        &inventario,
        &almacen.texto(),
    );
    Corrida {
        codigo,
        estandar,
        diagnostico,
        receptor,
    }
}

// ------------------------------------------------------------------------------------------
// AC-5: los cinco códigos, cada uno con su accesorio que lo dispara y su accesorio que no.
// ------------------------------------------------------------------------------------------

/// DISC-01 con el almacén en `en_ejecucion` y el sidecar detenido. Con un contenedor caído la
/// sonda NO se lanza: aquí se afirma con la secuencia entera más el silencio, no por el código.
#[test]
fn disc_01_se_dispara_con_el_almacen_en_ejecucion_y_un_contenedor_detenido() {
    let c = correr_estado(
        "disc-01",
        Some(EstadoDeCelula::EnEjecucion),
        vec![cuerpo(CORRIENDO), cuerpo(DETENIDO), sin_cuerpo()],
    );
    c.exige_solo("DISC-01");
    assert_eq!(
        secuencia_recibida(&c.receptor, 2),
        [
            "GET /containers/c1-nucleo/json",
            "GET /containers/c1-sidecar/json"
        ],
        "cell status inspecciona los dos contenedores, en este orden, y nada más"
    );
    exigir_silencio(&c.receptor);
}

/// DISC-01 NO aparece con los dos contenedores corriendo y la sonda confirmando.
#[test]
fn disc_01_no_aparece_cuando_ambos_contenedores_corren() {
    correr_estado(
        "disc-01-no",
        Some(EstadoDeCelula::EnEjecucion),
        guiones_coherentes(br#"{"StatusCode":0}"#),
    )
    .exige_sin_discrepancias();
}

/// DISC-02 con el almacén en `suspendida` y el núcleo corriendo.
#[test]
fn disc_02_se_dispara_con_el_almacen_suspendido_y_un_contenedor_corriendo() {
    correr_estado(
        "disc-02",
        Some(EstadoDeCelula::Suspendida),
        vec![cuerpo(CORRIENDO), cuerpo(DETENIDO), sin_cuerpo()],
    )
    .exige_solo("DISC-02");
}

/// DISC-02 NO aparece con el almacén en `suspendida` y los dos contenedores detenidos: ése es el
/// estado coherente de una célula pausada.
#[test]
fn disc_02_no_aparece_cuando_la_celula_pausada_tiene_los_dos_contenedores_detenidos() {
    correr_estado(
        "disc-02-no",
        Some(EstadoDeCelula::Suspendida),
        vec![cuerpo(DETENIDO), cuerpo(DETENIDO), sin_cuerpo()],
    )
    .exige_sin_discrepancias();
}

/// DISC-03 con los dos contenedores corriendo y la sonda devolviendo distinto de cero. La
/// secuencia se compara ENTERA: siete peticiones, la reinspección del núcleo incluida.
#[test]
fn disc_03_se_dispara_cuando_la_sonda_no_confirma_disponibilidad() {
    let c = correr_estado(
        "disc-03",
        Some(EstadoDeCelula::EnEjecucion),
        guiones_coherentes(br#"{"StatusCode":1}"#),
    );
    c.exige_solo("DISC-03");
    assert_eq!(
        secuencia_recibida(&c.receptor, 7),
        [
            "GET /containers/c1-nucleo/json",
            "GET /containers/c1-sidecar/json",
            "GET /containers/c1-nucleo/json",
            "POST /containers/create",
            "POST /containers/sonda1/start",
            "POST /containers/sonda1/wait",
            "DELETE /containers/sonda1",
        ],
        "siete peticiones exactas: dos inspecciones y los cinco pasos de la sonda corta"
    );
    exigir_silencio(&c.receptor);
}

/// DISC-03 NO aparece cuando la sonda devuelve 0.
#[test]
fn disc_03_no_aparece_cuando_la_sonda_confirma_disponibilidad() {
    let c = correr_estado(
        "disc-03-no",
        Some(EstadoDeCelula::EnEjecucion),
        guiones_coherentes(br#"{"StatusCode":0}"#),
    );
    c.exige_sin_discrepancias();
    assert!(
        c.estandar.contains("salud: listo"),
        "la sonda en 0 se reporta como listo: {:?}",
        c.estandar
    );
}

/// DISC-04 con fila en el almacén y los dos contenedores ausentes de Docker. La fila se siembra
/// en `suspendida` a propósito: con `en_ejecucion` saltaría además DISC-01 y la prueba no aislaría
/// el código que dice fijar.
#[test]
fn disc_04_se_dispara_con_fila_en_el_almacen_y_sin_contenedores_en_docker() {
    let c = correr_estado(
        "disc-04",
        Some(EstadoDeCelula::Suspendida),
        vec![ausente(), ausente(), sin_cuerpo()],
    );
    c.exige_solo("DISC-04");
    assert!(
        c.estandar.contains("docker nucleo: ausente")
            && c.estandar.contains("docker sidecar: ausente"),
        "los dos se reportan ausentes: {:?}",
        c.estandar
    );
}

/// DISC-04 NO aparece cuando los dos contenedores existen, aunque estén detenidos: existir y
/// estar corriendo son cosas distintas.
#[test]
fn disc_04_no_aparece_cuando_los_contenedores_existen_detenidos() {
    correr_estado(
        "disc-04-no",
        Some(EstadoDeCelula::Suspendida),
        vec![cuerpo(DETENIDO), cuerpo(DETENIDO), sin_cuerpo()],
    )
    .exige_sin_discrepancias();
}

/// DISC-05 con contenedores en Docker y NINGUNA fila en el almacén. `cell status` lo reporta y no
/// lo repara: la guarda de sólo lectura de más abajo exige que la fila siga ausente después.
#[test]
fn disc_05_se_dispara_con_contenedores_en_docker_y_sin_fila_en_el_almacen() {
    let c = correr_estado("disc-05", None, guiones_coherentes(br#"{"StatusCode":0}"#));
    c.exige_solo("DISC-05");
    assert!(
        c.estandar.contains("estado: sin_fila"),
        "el estado almacenado se reporta sin_fila: {:?}",
        c.estandar
    );
}

/// DISC-05 NO aparece cuando la fila existe.
#[test]
fn disc_05_no_aparece_cuando_la_fila_existe() {
    correr_estado(
        "disc-05-no",
        Some(EstadoDeCelula::EnEjecucion),
        guiones_coherentes(br#"{"StatusCode":0}"#),
    )
    .exige_sin_discrepancias();
}

// ------------------------------------------------------------------------------------------
// AC-6 y AC-7: los cinco campos impresos, el historial y lo que NO se imprime.
// ------------------------------------------------------------------------------------------

/// AC-6: sobre una célula coherente se imprimen los CINCO campos y se sale `Exito`.
///
/// La salida se compara entera: quitar un campo, reordenarlos o cambiar una etiqueta pone roja
/// esta única aserción. El historial sale con su marcador de vacío, que es el caso legítimo
/// mientras la tarea 13 no escriba en `sustituciones`.
#[test]
fn cell_status_imprime_los_cinco_campos_y_sale_exito() {
    let c = correr_estado(
        "status-coherente",
        Some(EstadoDeCelula::EnEjecucion),
        guiones_coherentes(br#"{"StatusCode":0}"#),
    );
    assert_eq!(c.codigo, CodigoDeSalida::Exito);
    assert_eq!(
        c.estandar,
        "estado: en_ejecucion\n\
         docker nucleo: running\n\
         docker sidecar: running\n\
         salud: listo\n\
         sustituciones: (ninguna)\n"
    );
    assert!(c.diagnostico.is_empty(), "{:?}", c.diagnostico);
}

/// AC-6: con filas en `sustituciones` se imprime el historial en lugar del marcador de vacío.
#[test]
fn cell_status_imprime_el_historial_de_sustituciones_cuando_lo_hay() {
    let servidor = ServidorDockerFalso::nuevo("status-sustituciones");
    let ruta = servidor.ruta();
    let almacen = AlmacenTemporal::nuevo("status-sustituciones");
    preparar_almacen(&almacen, Some(EstadoDeCelula::EnEjecucion));
    sembrar_sustitucion(&almacen, "baneo permanente", 1_700_000_500_000);
    let _receptor = servir_guiones(servidor, guiones_coherentes(br#"{"StatusCode":0}"#));
    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_secs(10));
    let inventario = InventarioDocker::nuevo(ruta, Duration::from_secs(10));

    let (codigo, estandar, diagnostico) = correr(
        &["cell", "status", "--id", "c1"],
        &cliente,
        &inventario,
        &almacen.texto(),
    );

    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert_eq!(
        estandar,
        "estado: en_ejecucion\n\
         docker nucleo: running\n\
         docker sidecar: running\n\
         salud: listo\n\
         sustituciones:\n\
         \x20 - baneo permanente (1700000500000 ms)\n"
    );
    assert!(
        !estandar.contains("(ninguna)"),
        "con historial no se imprime el marcador de vacío: {estandar:?}"
    );
}

/// AC-7: la superficie de métricas de la tarea 20 no se filtra a `cell status`.
///
/// Se comprueba sobre DOS accesorios —uno coherente y uno con discrepancia— y sobre los dos
/// sumideros, porque un campo de métrica podría colarse sólo en uno de los caminos.
#[test]
fn cell_status_no_reporta_ratio_de_acuses_ni_ventana_de_silencio() {
    let corridas = [
        correr_estado(
            "status-sin-metricas-ok",
            Some(EstadoDeCelula::EnEjecucion),
            guiones_coherentes(br#"{"StatusCode":0}"#),
        ),
        correr_estado(
            "status-sin-metricas-disc",
            Some(EstadoDeCelula::EnEjecucion),
            vec![cuerpo(CORRIENDO), cuerpo(DETENIDO), sin_cuerpo()],
        ),
    ];
    for c in &corridas {
        let salida = format!("{}{}", c.estandar, c.diagnostico).to_lowercase();
        assert!(!salida.is_empty(), "la corrida tiene que producir salida");
        for token in ["ack", "acuse", "ratio", "silencio", "ventana"] {
            assert!(
                !salida.contains(token),
                "«{token}» pertenece a la tarea 20 y no puede aparecer: {salida:?}"
            );
        }
    }
}

/// Un `inspect` que falla con 500 no es una observación: se nombra la fuente Docker y se devuelve
/// `Fallo` SIN inventar DISC-04 ni DISC-05.
///
/// La segunda mitad de la aserción es la que importa: sin la guarda, el fallo de transporte se
/// lee como «contenedor ausente» y el comando afirma por escrito que los contenedores no existen
/// en Docker, que es exactamente lo que no se sabe.
#[test]
fn una_fuente_docker_que_falla_se_nombra_y_no_se_convierte_en_discrepancia() {
    let c = correr_estado(
        "fuente-docker-500",
        Some(EstadoDeCelula::EnEjecucion),
        vec![
            Guion::SinCuerpo {
                estado: 500,
                razon: "Internal Server Error",
            },
            sin_cuerpo(),
        ],
    );
    assert_eq!(c.codigo, CodigoDeSalida::Fallo);
    assert!(
        c.diagnostico.contains(TEXTO_DE_FUENTE_DOCKER_FALLIDA)
            && c.diagnostico.contains("c1-nucleo"),
        "el diagnóstico nombra la fuente y el contenedor: {:?}",
        c.diagnostico
    );
    for codigo in CODIGOS {
        assert!(
            !c.diagnostico.contains(codigo),
            "una fuente que falla no produce {codigo}: {:?}",
            c.diagnostico
        );
    }
}

/// Un almacén ausente es también una fuente que falla: `cell status` y `cell list` devuelven
/// `Fallo` nombrando la ruta y NO crean el archivo. Los dos comandos se comprueban porque cada
/// uno tiene su propia línea de apertura: con sólo `status`, una apertura de escritura en
/// `ejecutar_listado` pasaría.
#[test]
fn un_almacen_ausente_se_nombra_y_no_se_crea() {
    let servidor = ServidorDockerFalso::nuevo("almacen-ausente");
    let ruta = servidor.ruta();
    let almacen = AlmacenTemporal::nuevo("almacen-ausente");
    let receptor = servir_guiones(servidor, vec![cuerpo(CORRIENDO)]);
    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_secs(10));
    let inventario = InventarioDocker::nuevo(ruta, Duration::from_secs(10));

    for snippet in [&["cell", "status", "--id", "c1"][..], &["cell", "list"][..]] {
        let (codigo, estandar, diagnostico) =
            correr(snippet, &cliente, &inventario, &almacen.texto());
        assert_eq!(codigo, CodigoDeSalida::Fallo, "snippet {snippet:?}");
        assert!(estandar.is_empty(), "estándar vacío: {estandar:?}");
        assert!(
            diagnostico.contains(&almacen.ruta().to_string_lossy().into_owned()),
            "el diagnóstico de {snippet:?} nombra el almacén ausente: {diagnostico:?}"
        );
        assert!(
            almacen.bytes().is_none(),
            "{snippet:?} no crea el almacén que no encuentra"
        );
    }
    exigir_silencio(&receptor);
}

// ------------------------------------------------------------------------------------------
// AC-8 y la garantía de sólo lectura.
// ------------------------------------------------------------------------------------------

/// AC-8: `cell list` imprime la unión del almacén y de Docker, y la salida se compara ENTERA.
///
/// Una aserción como `contains("sin_fila") || contains("c2")` no prueba nada cuando ya se afirmó
/// `contains("c2")`: las dos ramas se mueven juntas. Aquí las dos líneas se fijan completas, con
/// el estado almacenado de la célula que sólo está en el almacén, el `sin_fila` de la que sólo
/// está en Docker y el estado Docker de cada contenedor en los dos casos.
#[test]
fn cell_list_imprime_la_union_entera_de_almacen_y_docker() {
    let servidor = ServidorDockerFalso::nuevo("list-union");
    let ruta = servidor.ruta();
    let almacen = AlmacenTemporal::nuevo("list-union");
    preparar_almacen(&almacen, Some(EstadoDeCelula::EnEjecucion));
    let receptor = servir_guiones(
        servidor,
        vec![
            cuerpo(
                br#"[{"Names":["/c2-nucleo"],"State":"running"},{"Names":["/c2-sidecar"],"State":"exited"}]"#,
            ),
            sin_cuerpo(),
        ],
    );
    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_secs(10));
    let inventario = InventarioDocker::nuevo(ruta, Duration::from_secs(10));

    let (codigo, estandar, diagnostico) =
        correr(&["cell", "list"], &cliente, &inventario, &almacen.texto());

    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert_eq!(
        estandar,
        "c1 estado=en_ejecucion nucleo=ausente sidecar=ausente\n\
         c2 estado=sin_fila nucleo=running sidecar=exited\n"
    );
    assert!(diagnostico.is_empty(), "{diagnostico:?}");
    assert_eq!(
        secuencia_recibida(&receptor, 1),
        ["GET /containers/json?all=true"],
        "cell list emite exactamente una petición de listado y ninguna sonda"
    );
    exigir_silencio(&receptor);
}

/// Sólo lectura: `cell status` y `cell list` dejan el archivo idéntico byte a byte, con la misma
/// marca de modificación y los mismos conteos en las tres tablas.
///
/// El caso de `cell status` se corre sobre una célula SIN fila —el de DISC-05— porque es el único
/// en el que el comando tendría algo que «arreglar»: reportarlo y no repararlo es la regla.
#[test]
fn status_y_list_no_escriben_en_el_almacen() {
    let almacen = AlmacenTemporal::nuevo("solo-lectura");
    preparar_almacen(&almacen, None);
    sembrar_sustitucion(&almacen, "sustitucion sembrada", 500);
    let bytes_antes = almacen.bytes().expect("el accesorio creó el almacén");
    let modificado_antes = almacen.modificado();
    let conteos_antes = conteos(&almacen);
    assert_eq!(
        conteos_antes,
        (0, 0, 1),
        "el accesorio deja una sustitución"
    );

    let servidor = ServidorDockerFalso::nuevo("solo-lectura-status");
    let ruta = servidor.ruta();
    let _r = servir_guiones(servidor, guiones_coherentes(br#"{"StatusCode":0}"#));
    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_secs(10));
    let inventario = InventarioDocker::nuevo(ruta, Duration::from_secs(10));
    let (codigo, _estandar, diagnostico) = correr(
        &["cell", "status", "--id", "c1"],
        &cliente,
        &inventario,
        &almacen.texto(),
    );
    assert_eq!(codigo, CodigoDeSalida::Fallo);
    assert!(diagnostico.contains("DISC-05"), "{diagnostico:?}");
    assert_eq!(almacen.bytes().as_deref(), Some(bytes_antes.as_slice()));
    assert_eq!(almacen.modificado(), modificado_antes);
    assert_eq!(conteos(&almacen), conteos_antes);

    let servidor = ServidorDockerFalso::nuevo("solo-lectura-list");
    let ruta = servidor.ruta();
    let _r = servir_guiones(
        servidor,
        vec![cuerpo(
            br#"[{"Names":["/c2-nucleo"],"State":"running"},{"Names":["/c2-sidecar"],"State":"running"}]"#,
        )],
    );
    let cliente = ClienteDocker::con_tiempo_limite(ruta.clone(), Duration::from_secs(10));
    let inventario = InventarioDocker::nuevo(ruta, Duration::from_secs(10));
    let (codigo, estandar, diagnostico) =
        correr(&["cell", "list"], &cliente, &inventario, &almacen.texto());
    assert_eq!(codigo, CodigoDeSalida::Exito, "diag: {diagnostico:?}");
    assert_eq!(
        estandar,
        "c2 estado=sin_fila nucleo=running sidecar=running\n"
    );
    assert_eq!(almacen.bytes().as_deref(), Some(bytes_antes.as_slice()));
    assert_eq!(almacen.modificado(), modificado_antes);
    assert_eq!(conteos(&almacen), conteos_antes);
}
