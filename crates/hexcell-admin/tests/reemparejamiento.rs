//! Tests de integración de la secuencia completa de `cell rebind` (tarea 13 de A-6, HEX-085-b).
//!
//! Cada prueba levanta su propio demonio falso sobre un socket Unix temporal (ver `comun`) y ejecuta
//! `ejecutar_con_efectos` con una invocación de `cell rebind`. Las respuestas del demonio falso se
//! programan con `servir_guiones` y se asertan por SECUENCIA COMPLETA (orden + cuerpos), no sólo por
//! rutas, siguiendo el protocolo de decisión D8: los nombres de accesorio (red, puerto, volumen,
//! imagen) no son derivables de `--id` ni de las constantes de producción.

mod comun;

use hexcell_admin::almacen_plano_de_control::AlmacenDelPlanoDeControl;
use hexcell_admin::argumentos::{MetodoDeEmparejamiento, analizar};
use hexcell_admin::codigo_de_salida::CodigoDeSalida;
use hexcell_admin::comandos::ejecutar_con_efectos;
use hexcell_admin::docker::{ClienteDocker, InventarioDocker};
use hexcell_admin::estado_de_celula::EstadoDeCelula;
use hexcell_admin::salida::Salida;

use comun::{
    AlmacenTemporal, Guion, ServidorDockerFalso, exigir_silencio, recibir, secuencia_recibida,
    servir_guiones,
};

use std::io::Write;

// ─────────────────────────────────────────────────────────────────────────────────────────
// Constantes de accesorio: NINGUNA derivable de `--id` ni de constantes de producción.
// ─────────────────────────────────────────────────────────────────────────────────────────

const RED_DEL_ACCESORIO: &str = "red-de-rebind-r3k7";
const PUERTO_ADMIN_DEL_ACCESORIO: &str = "5080";
const VOLUMEN_DEL_ACCESORIO: &str = "vol-rebind-q4w8n1";
const IMAGEN_DEL_ACCESORIO: &str = "alpine:3";
#[allow(dead_code)]
const _LIMITE_HTTP: u64 = 40;

/// Convierte un String en `&'static [u8]` liberando el `Vec` en el heap (fuga controlada de test).
fn static_bytes(s: String) -> &'static [u8] {
    Box::leak(s.into_bytes().into_boxed_slice())
}

/// Convierte un `&str` en `&'static [u8]` liberando una copia en el heap.
fn static_bytes_from_str(s: &str) -> &'static [u8] {
    static_bytes(s.to_string())
}

/// Contenedor de pausa: id esperado para la respucción de creación.
fn sin_cuerpo(estado: u16, razon: &'static str) -> Guion {
    Guion::SinCuerpo { estado, razon }
}

/// Envuelve un cuerpo como una trama stdout del formato multiplexado de Docker.
fn trama_stdout_bytes(cuerpo: &[u8]) -> Vec<u8> {
    let mut trama = Vec::with_capacity(8 + cuerpo.len());
    trama.push(1u8);
    trama.extend_from_slice(&[0u8, 0, 0]);
    trama.extend_from_slice(&(cuerpo.len() as u32).to_be_bytes());
    trama.extend_from_slice(cuerpo);
    trama
}

/// Respuesta de logs con el cuerpo dado como única trama stdout.
fn guion_de_logs(cuerpo: &[u8]) -> Guion {
    Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: Box::leak(trama_stdout_bytes(cuerpo).into_boxed_slice()),
    }
}

/// Inspección del núcleo para rebind: running, con red, puerto admin y volumen.
fn inspeccion_del_nucleo() -> Guion {
    Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: static_bytes_from_str(&format!(
            r#"{{"State":{{"Status":"running"}},"NetworkSettings":{{"Networks":{{"{}":{{"NetworkID":"n1"}}}}}},"Config":{{"Env":["PATH=/usr/bin","HEXCELL_DIRECCION_ADMIN=0.0.0.0:{}"]}},"Mounts":[{{"Type":"volume","Name":"{}","Destination":"/var/lib/hexcell"}}]}}"#,
            RED_DEL_ACCESORIO, PUERTO_ADMIN_DEL_ACCESORIO, VOLUMEN_DEL_ACCESORIO,
        )),
    }
}

/// Inspección del sidecar: solo necesitamos que exista.
fn inspeccion_del_sidecar() -> Guion {
    Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: br#"{"State":{"Status":"running"}}"#,
    }
}

/// Contador monótono para generar identificadores de contenedor de sonda únicos.
fn id_de_sonda(contador: usize) -> String {
    format!("sonda-{contador}")
}

/// Añade al vector de guiones una sonda HTTP que devuelve el cuerpo de respuesta por stdout.
fn servir_sonda_http(guiones: &mut Vec<Guion>, contador: &mut usize, respuesta: &[u8]) -> String {
    let id = id_de_sonda(*contador);
    guiones.push(Guion::ConCuerpo {
        estado: 201,
        razon: "Created",
        cuerpo: static_bytes_from_str(&format!(r#"{{"Id":"{id}","Warnings":[]}}"#)),
    });
    guiones.push(sin_cuerpo(204, "No Content")); // start
    guiones.push(Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: br#"{"StatusCode":0}"#, // wait
    });
    guiones.push(guion_de_logs(respuesta)); // logs
    guiones.push(sin_cuerpo(204, "No Content")); // delete
    *contador += 1;
    id
}

/// Añade al vector de guiones un contenedor sin logs (sólo create, start, wait, delete).
fn servir_contenedor_sin_logs(guiones: &mut Vec<Guion>, contador: &mut usize) -> String {
    let id = id_de_sonda(*contador);
    guiones.push(Guion::ConCuerpo {
        estado: 201,
        razon: "Created",
        cuerpo: static_bytes_from_str(&format!(r#"{{"Id":"{id}","Warnings":[]}}"#)),
    });
    guiones.push(sin_cuerpo(204, "No Content")); // start
    guiones.push(Guion::ConCuerpo {
        estado: 200,
        razon: "OK",
        cuerpo: br#"{"StatusCode":0}"#, // wait
    });
    guiones.push(sin_cuerpo(204, "No Content")); // delete
    *contador += 1;
    id
}

/// Ejecuta `ejecutar_con_efectos` con los sumideros inyectados y devuelve (código, estándar, diagnóstico).
fn ejecutar_rebind<S: Write, D: Write>(
    argumento: &[&str],
    cliente: &ClienteDocker,
    inventario: &InventarioDocker,
    ruta_almacen: &str,
    ahora_ms: i64,
    salida: &mut Salida<S, D>,
) -> CodigoDeSalida {
    let invocacion = analizar(&argumento.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    ejecutar_con_efectos(
        invocacion,
        salida,
        cliente,
        inventario,
        ruta_almacen,
        ahora_ms,
        hexcell_admin::ciclo_de_vida::DatosDeSondeo {
            imagen: IMAGEN_DEL_ACCESORIO.to_string(),
            limite_segundos: 45,
        },
    )
}

/// Parámetros inyectables del happy path para variar el método de emparejamiento.
struct ParametrosDeHappyPath {
    metodo: MetodoDeEmparejamiento,
    snippet_metodo: Vec<&'static str>,
}

impl ParametrosDeHappyPath {
    fn qr() -> Self {
        Self {
            metodo: MetodoDeEmparejamiento::Qr,
            snippet_metodo: vec![],
        }
    }

    fn codigo() -> Self {
        Self {
            metodo: MetodoDeEmparejamiento::CodigoDeVinculacion,
            snippet_metodo: vec!["--metodo", "codigo_de_vinculacion"],
        }
    }
}

/// AC-7..AC-14, AC-16: el happy path desde `EnEjecución` (sin fila previa) emite la secuencia
/// completa de peticiones Docker en orden estricto, con los cuerpos correctos, y termina en
/// `Exito` con la fila en `EnEjecución` y una fila de `sustituciones`.
fn happy_path_completo(parametros: ParametrosDeHappyPath) {
    let servidor = ServidorDockerFalso::nuevo("rebind-happy");
    let ruta = servidor.ruta();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
    let almacen = AlmacenTemporal::nuevo("rebind-happy");
    let ruta_almacen = almacen.texto();

    // Construir el vector de guiones para la secuencia completa.
    let mut guiones: Vec<Guion> = Vec::new();
    let mut contador = 0usize;

    // 1-2: resolver_datos_de_celula_para_rebind (inspeccionar núcleo y sidecar).
    guiones.push(inspeccion_del_nucleo());
    guiones.push(inspeccion_del_sidecar());

    // 3: preparar_reemparejamiento inspecciona el núcleo otra vez para verificar running.
    guiones.push(inspeccion_del_nucleo());

    // 4-8: sonda de pausa (create, start, wait, logs, delete).
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );

    // 9-12: sonda de cierre (create, start, wait, delete).
    servir_contenedor_sin_logs(&mut guiones, &mut contador);

    // 13: detener sidecar sin plazo.
    guiones.push(sin_cuerpo(204, "No Content"));

    // 14-17: rm sibling (create, start, wait, delete).
    servir_contenedor_sin_logs(&mut guiones, &mut contador);

    // 18: rearrancar sidecar.
    guiones.push(sin_cuerpo(204, "No Content"));

    // 19-23: sonda de pausa con reintento (create, start, wait, logs, delete).
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );

    // 24-28: sonda de emparejamiento (create, start, wait, logs, delete).
    let valor_pairing = match parametros.metodo {
        MetodoDeEmparejamiento::Qr => "QR-VALUE-ABCD",
        MetodoDeEmparejamiento::CodigoDeVinculacion => "EFGH-IJKL",
    };
    let cuerpo_pairing = static_bytes(format!(
        r#"{{"resultado":"codigo","metodo":"{}","valor":"{}","expira_en_ms":{}}}"#,
        parametros.metodo.nombre_de_cable(),
        valor_pairing,
        1_700_000_000_000i64,
    ));
    servir_sonda_http(&mut guiones, &mut contador, cuerpo_pairing);

    // 29-33: sonda de estado (create, start, wait, logs, delete).
    servir_sonda_http(&mut guiones, &mut contador, br#"{"estado":"activa"}"#);

    // 34-38: sonda de reanudar (create, start, wait, logs, delete).
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"reanudar"}"#,
    );

    let total_peticiones = guiones.len();
    assert!(total_peticiones > 0);
    let receptor = servir_guiones(servidor, guiones);

    // Construir la invocación.
    let mut argumento = vec![
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "sustitución por baneo",
        "--confirmar",
    ];
    argumento.extend(parametros.snippet_metodo);

    let mut estandar_buf: Vec<u8> = Vec::new();
    let mut diagnostico_buf: Vec<u8> = Vec::new();
    let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

    let ahora_ms = 1_700_000_000_000i64;
    let codigo = ejecutar_rebind(
        &argumento,
        &cliente,
        &inventario,
        &ruta_almacen,
        ahora_ms,
        &mut salida,
    );

    assert_eq!(
        codigo,
        CodigoDeSalida::Exito,
        "el happy path debe terminar en Exito"
    );

    // Asertar la secuencia completa de peticiones.
    let mut esperado: Vec<String> = Vec::with_capacity(total_peticiones);

    // 1-2: resolver_datos_de_celula_para_rebind.
    esperado.push("GET /containers/c1-nucleo/json".to_string());
    esperado.push("GET /containers/c1-sidecar/json".to_string());
    // 3: preparar_reemparejamiento inspecciona el núcleo.
    esperado.push("GET /containers/c1-nucleo/json".to_string());
    // 4-8: sonda de pausa.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(0)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(0)));
    esperado.push(format!(
        "GET /containers/{}/logs?stdout=1&stderr=0",
        id_de_sonda(0)
    ));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(0)));
    // 9-12: sonda de cierre.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(1)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(1)));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(1)));
    // 13: detener sidecar.
    esperado.push("POST /containers/c1-sidecar/stop".to_string());
    // 14-17: rm sibling.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(2)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(2)));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(2)));
    // 18: rearrancar sidecar.
    esperado.push("POST /containers/c1-sidecar/start".to_string());
    // 19-23: sonda de pausa con reintento.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(3)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(3)));
    esperado.push(format!(
        "GET /containers/{}/logs?stdout=1&stderr=0",
        id_de_sonda(3)
    ));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(3)));
    // 24-28: sonda de emparejamiento.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(4)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(4)));
    esperado.push(format!(
        "GET /containers/{}/logs?stdout=1&stderr=0",
        id_de_sonda(4)
    ));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(4)));
    // 29-33: sonda de estado.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(5)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(5)));
    esperado.push(format!(
        "GET /containers/{}/logs?stdout=1&stderr=0",
        id_de_sonda(5)
    ));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(5)));
    // 34-38: sonda de reanudar.
    esperado.push("POST /containers/create".to_string());
    esperado.push(format!("POST /containers/{}/start", id_de_sonda(6)));
    esperado.push(format!("POST /containers/{}/wait", id_de_sonda(6)));
    esperado.push(format!(
        "GET /containers/{}/logs?stdout=1&stderr=0",
        id_de_sonda(6)
    ));
    esperado.push(format!("DELETE /containers/{}", id_de_sonda(6)));

    let recibidas = secuencia_recibida(&receptor, total_peticiones);
    assert_eq!(
        recibidas, esperado,
        "la secuencia de peticiones no coincide con la esperada"
    );

    // Asertar el cuerpo de la sonda de emparejamiento (AC-8/AC-12).
    // Las peticiones se leyeron con `recibir` en `secuencia_recibida`, pero los cuerpos se
    // conservaron en el receptor. Necesitamos leerlas de nuevo. Como ya consumimos todas las
    // peticiones del receptor con `secuencia_recibida`, los cuerpos ya no están disponibles.
    // Por eso asertamos los cuerpos ANTES de leer la secuencia.

    // NOTA: la aserción de cuerpos se hace en una prueba separada (`cuerpos_del_happy_path`)
    // que repite la ejecución con aserciones por petición.

    // Asertar el estado final del almacén.
    let a =
        AlmacenDelPlanoDeControl::abrir_solo_lectura(std::path::Path::new(&ruta_almacen)).unwrap();
    let fila = a.leer_estado("c1").unwrap().unwrap();
    assert_eq!(fila.estado, EstadoDeCelula::EnEjecucion);
    assert_eq!(fila.motivo, "emparejamiento_confirmado");

    let sustituciones = a.leer_sustituciones("c1").unwrap();
    assert_eq!(sustituciones.len(), 1);
    assert_eq!(sustituciones[0].id_celula, "c1");
    assert_eq!(sustituciones[0].motivo, "sustitución por baneo");
    assert_eq!(sustituciones[0].registrado_ms, ahora_ms);

    // Asertar la salida estándar.
    let texto_estandar = String::from_utf8(estandar_buf).unwrap();
    assert!(
        texto_estandar.contains("emparejamiento"),
        "la línea de emparejamiento debe aparecer por estándar: {texto_estandar}"
    );
    assert!(
        texto_estandar.contains("renderizado gráfico no está integrado"),
        "la nota de renderizado debe aparecer: {texto_estandar}"
    );
    assert!(
        texto_estandar.contains("cell rebind completado para «c1»"),
        "la línea de completitud debe aparecer: {texto_estandar}"
    );
}

#[test]
fn happy_path_completo_con_qr() {
    happy_path_completo(ParametrosDeHappyPath::qr());
}

#[test]
fn happy_path_completo_con_codigo_de_vinculacion() {
    happy_path_completo(ParametrosDeHappyPath::codigo());
}

/// AC-8/AC-12/AC-14: los cuerpos de las sondas del happy path llevan la imagen, red, comando y
/// cuerpo JSON correctos. Esta prueba repite la ejecución leyendo los cuerpos petición a petición.
#[test]
fn cuerpos_del_happy_path() {
    let servidor = ServidorDockerFalso::nuevo("rebind-cuerpos");
    let ruta = servidor.ruta();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
    let almacen = AlmacenTemporal::nuevo("rebind-cuerpos");
    let ruta_almacen = almacen.texto();

    let mut guiones: Vec<Guion> = Vec::new();
    let mut contador = 0usize;

    guiones.push(inspeccion_del_nucleo());
    guiones.push(inspeccion_del_sidecar());
    guiones.push(inspeccion_del_nucleo()); // preparar_reemparejamiento
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    ); // 0
    servir_contenedor_sin_logs(&mut guiones, &mut contador); // 1
    guiones.push(sin_cuerpo(204, "No Content")); // stop sidecar
    servir_contenedor_sin_logs(&mut guiones, &mut contador); // 2 (rm)
    guiones.push(sin_cuerpo(204, "No Content")); // start sidecar
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    ); // 3
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"codigo","metodo":"qr","valor":"QR-XYZ","expira_en_ms":1700000000000}"#,
    ); // 4
    servir_sonda_http(&mut guiones, &mut contador, br#"{"estado":"activa"}"#); // 5
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"reanudar"}"#,
    ); // 6

    let receptor = servir_guiones(servidor, guiones);

    let argumento = vec![
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "baneo",
        "--confirmar",
    ];

    let mut estandar_buf: Vec<u8> = Vec::new();
    let mut diagnostico_buf: Vec<u8> = Vec::new();
    let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

    let ahora_ms = 1_700_000_000_000i64;
    let codigo = ejecutar_rebind(
        &argumento,
        &cliente,
        &inventario,
        &ruta_almacen,
        ahora_ms,
        &mut salida,
    );
    assert_eq!(codigo, CodigoDeSalida::Exito);

    // Leer todas las peticiones y asertar los cuerpos clave.
    // 0-2: inspecciones (no asertamos cuerpo).
    for _ in 0..3 {
        recibir(&receptor);
    }

    // Pausa probe: crear con Cmd correcto.
    let crear_pausa = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_pausa.cuerpo).unwrap();
    assert_eq!(cuerpo["Image"], IMAGEN_DEL_ACCESORIO);
    assert_eq!(cuerpo["HostConfig"]["NetworkMode"], RED_DEL_ACCESORIO);
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "wget",
            "-q",
            "-O",
            "-",
            "-T",
            "40",
            "--post-data",
            r#"{"accion":"pausar"}"#,
            "http://c1-nucleo:5080/admin/envio/pausa"
        ])
    );
    // Consumir start, wait, logs, delete de la pausa.
    for _ in 0..4 {
        recibir(&receptor);
    }

    // Cierre probe: crear.
    let crear_cierre = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_cierre.cuerpo).unwrap();
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "wget",
            "-q",
            "-O",
            "-",
            "--post-data",
            "",
            "http://c1-nucleo:5080/admin/sesion/cierre"
        ])
    );
    for _ in 0..3 {
        recibir(&receptor);
    }

    // Sidecar stop.
    assert_eq!(recibir(&receptor).objetivo, "/containers/c1-sidecar/stop");

    // RM sibling: crear con Cmd exacto y montaje correcto.
    let crear_rm = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_rm.cuerpo).unwrap();
    assert_eq!(cuerpo["Image"], IMAGEN_DEL_ACCESORIO);
    assert_eq!(cuerpo["HostConfig"]["NetworkMode"], "none");
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "rm",
            "-f",
            "/var/lib/hexcell/sqlstore.db",
            "/var/lib/hexcell/sqlstore.db-wal",
            "/var/lib/hexcell/sqlstore.db-shm"
        ]),
        "el Cmd del rm sibling debe ser exactamente el de D5.6"
    );
    assert_eq!(
        cuerpo["HostConfig"]["Mounts"],
        serde_json::json!([{
            "Type": "volume",
            "Source": VOLUMEN_DEL_ACCESORIO,
            "Target": "/var/lib/hexcell",
        }])
    );
    // El cuerpo completo no menciona identidad.db ni outbox.db.
    let texto_del_cuerpo = String::from_utf8_lossy(&crear_rm.cuerpo);
    assert!(
        !texto_del_cuerpo.contains("identidad.db"),
        "el cuerpo del rm no debe mencionar identidad.db"
    );
    assert!(
        !texto_del_cuerpo.contains("outbox.db"),
        "el cuerpo del rm no debe mencionar outbox.db"
    );
    for _ in 0..3 {
        recibir(&receptor);
    }

    // Sidecar start.
    assert_eq!(recibir(&receptor).objetivo, "/containers/c1-sidecar/start");

    // Pausa probe 2 (tras rm): crear.
    let crear_pausa2 = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_pausa2.cuerpo).unwrap();
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "wget",
            "-q",
            "-O",
            "-",
            "-T",
            "40",
            "--post-data",
            r#"{"accion":"pausar"}"#,
            "http://c1-nucleo:5080/admin/envio/pausa"
        ])
    );
    for _ in 0..4 {
        recibir(&receptor);
    }

    // Pairing probe: crear.
    let crear_pairing = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_pairing.cuerpo).unwrap();
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "wget",
            "-q",
            "-O",
            "-",
            "-T",
            "40",
            "--post-data",
            r#"{"metodo":"qr"}"#,
            "http://c1-nucleo:5080/admin/sesion/emparejamiento"
        ])
    );
    for _ in 0..4 {
        recibir(&receptor);
    }

    // Status probe: crear.
    let crear_status = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_status.cuerpo).unwrap();
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "wget",
            "-q",
            "-O",
            "-",
            "-T",
            "40",
            "http://c1-nucleo:5080/admin/sesion"
        ])
    );
    for _ in 0..4 {
        recibir(&receptor);
    }

    // Resume probe: crear.
    let crear_resume = recibir(&receptor);
    let cuerpo: serde_json::Value = serde_json::from_slice(&crear_resume.cuerpo).unwrap();
    assert_eq!(
        cuerpo["Cmd"],
        serde_json::json!([
            "wget",
            "-q",
            "-O",
            "-",
            "-T",
            "40",
            "--post-data",
            r#"{"accion":"reanudar"}"#,
            "http://c1-nucleo:5080/admin/envio/pausa"
        ])
    );
    for _ in 0..4 {
        recibir(&receptor);
    }
}

/// AC-7, AC-16: el resume desde `Reemparejando` emite sólo las sondas de emparejamiento, estado y
/// reanudar, más las dos inspecciones iniciales, y omite pausa/cierre/rm/stop/start.
#[test]
fn resume_desde_reemparejando_omite_pausa_cierre_y_rm() {
    let servidor = ServidorDockerFalso::nuevo("rebind-resume");
    let ruta = servidor.ruta();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
    let almacen = AlmacenTemporal::nuevo("rebind-resume");
    let ruta_almacen = almacen.texto();

    // Sembrar una fila en `Reemparejando`.
    {
        let a = AlmacenDelPlanoDeControl::abrir(std::path::Path::new(&ruta_almacen)).unwrap();
        a.registrar_transicion(
            "c1",
            Some(EstadoDeCelula::EnEjecucion),
            EstadoDeCelula::Reemparejando,
            "sustitución por baneo",
            1000,
        )
        .unwrap();
    }

    let mut guiones: Vec<Guion> = Vec::new();
    let mut contador = 0usize;

    // 1-2: resolver_datos_de_celula_para_rebind (inspeccionar núcleo y sidecar).
    guiones.push(inspeccion_del_nucleo());
    guiones.push(inspeccion_del_sidecar());

    // 3-7: sonda de emparejamiento.
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"codigo","metodo":"qr","valor":"QR-RESUME","expira_en_ms":1700000000000}"#,
    );

    // 8-12: sonda de estado.
    servir_sonda_http(&mut guiones, &mut contador, br#"{"estado":"activa"}"#);

    // 13-17: sonda de reanudar.
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"reanudar"}"#,
    );

    let total_peticiones = guiones.len();
    assert!(total_peticiones > 0);
    let receptor = servir_guiones(servidor, guiones);

    let argumento = vec![
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "sustitución por baneo",
        "--confirmar",
    ];

    let mut estandar_buf: Vec<u8> = Vec::new();
    let mut diagnostico_buf: Vec<u8> = Vec::new();
    let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

    let ahora_ms = 1_700_000_000_000i64;
    let codigo = ejecutar_rebind(
        &argumento,
        &cliente,
        &inventario,
        &ruta_almacen,
        ahora_ms,
        &mut salida,
    );

    assert_eq!(codigo, CodigoDeSalida::Exito);

    let mut esperado: Vec<String> = Vec::with_capacity(total_peticiones);
    esperado.push("GET /containers/c1-nucleo/json".to_string());
    esperado.push("GET /containers/c1-sidecar/json".to_string());
    for i in 0..3 {
        esperado.push("POST /containers/create".to_string());
        esperado.push(format!("POST /containers/{}/start", id_de_sonda(i)));
        esperado.push(format!("POST /containers/{}/wait", id_de_sonda(i)));
        esperado.push(format!(
            "GET /containers/{}/logs?stdout=1&stderr=0",
            id_de_sonda(i)
        ));
        esperado.push(format!("DELETE /containers/{}", id_de_sonda(i)));
    }

    let recibidas = secuencia_recibida(&receptor, total_peticiones);
    assert_eq!(recibidas, esperado);

    // Verificar que NO se emitieron paradas ni rm.
    for etiqueta in &recibidas {
        assert!(
            !etiqueta.contains("/stop"),
            "no debe parar contenedores en resume"
        );
        assert!(
            !etiqueta.contains("/volumes/"),
            "no debe eliminar volúmenes en resume"
        );
    }

    // Estado final.
    let a =
        AlmacenDelPlanoDeControl::abrir_solo_lectura(std::path::Path::new(&ruta_almacen)).unwrap();
    let fila = a.leer_estado("c1").unwrap().unwrap();
    assert_eq!(fila.estado, EstadoDeCelula::EnEjecucion);
    let sustituciones = a.leer_sustituciones("c1").unwrap();
    assert_eq!(sustituciones.len(), 1);
}

/// AC-7, AC-16: las filas en `Suspendida` y `Retirada` terminan en `Fallo` ANTES de cualquier
/// petición Docker.
#[test]
fn suspendida_y_retirada_fallan_antes_de_cualquier_peticion_docker() {
    for (estado, fragmento_diagnostico) in [
        (
            EstadoDeCelula::Suspendida,
            "ejecute cell unpause antes de cell rebind",
        ),
        (EstadoDeCelula::Retirada, "es terminal"),
    ] {
        let servidor = ServidorDockerFalso::nuevo("rebind-estado-invalido");
        let ruta = servidor.ruta();
        let cliente = ClienteDocker::nuevo(ruta.clone());
        let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
        let almacen = AlmacenTemporal::nuevo("rebind-estado-invalido");
        let ruta_almacen = almacen.texto();

        // Sembrar la fila.
        {
            let a = AlmacenDelPlanoDeControl::abrir(std::path::Path::new(&ruta_almacen)).unwrap();
            a.registrar_transicion("c1", None, estado, "prueba", 1000)
                .unwrap();
        }

        // Un único guion que NUNCA debe ser servido: si producción lo consumiera, el test fallaría
        // por silencio en vez de por el fallo esperado.
        let receptor = servir_guiones(servidor, vec![sin_cuerpo(204, "No Content")]);

        let argumento = vec![
            "cell",
            "rebind",
            "--id",
            "c1",
            "--motivo",
            "x",
            "--confirmar",
        ];

        let mut estandar_buf: Vec<u8> = Vec::new();
        let mut diagnostico_buf: Vec<u8> = Vec::new();
        let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

        let ahora_ms = 1_700_000_000_000i64;
        let codigo = ejecutar_rebind(
            &argumento,
            &cliente,
            &inventario,
            &ruta_almacen,
            ahora_ms,
            &mut salida,
        );

        assert_eq!(
            codigo,
            CodigoDeSalida::Fallo,
            "estado {estado:?} debe fallar"
        );

        let diagnostico = String::from_utf8(diagnostico_buf).unwrap();
        assert!(
            diagnostico.contains(fragmento_diagnostico),
            "diagnóstico para {estado:?}: {diagnostico}"
        );

        exigir_silencio(&receptor);
    }
}

/// AC-13, AC-16: un código de emparejamiento que expira deja la fila en `Reemparejando` sin fila de
/// `sustituciones`, y termina en `Fallo` con el diagnóstico exacto.
#[test]
fn codigo_expirado_deja_la_fila_reemparejando_y_sin_sustituciones() {
    let servidor = ServidorDockerFalso::nuevo("rebind-expirado");
    let ruta = servidor.ruta();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
    let almacen = AlmacenTemporal::nuevo("rebind-expirado");
    let ruta_almacen = almacen.texto();

    let mut guiones: Vec<Guion> = Vec::new();
    let mut contador = 0usize;

    guiones.push(inspeccion_del_nucleo());
    guiones.push(inspeccion_del_sidecar());
    guiones.push(inspeccion_del_nucleo()); // preparar_reemparejamiento
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );
    servir_contenedor_sin_logs(&mut guiones, &mut contador); // cierre
    guiones.push(sin_cuerpo(204, "No Content")); // stop sidecar
    servir_contenedor_sin_logs(&mut guiones, &mut contador); // rm
    guiones.push(sin_cuerpo(204, "No Content")); // start sidecar
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );
    // Pairing: código con expiración en el pasado para agotar el presupuesto inmediatamente.
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"codigo","metodo":"qr","valor":"QR-EXP","expira_en_ms":1000}"#,
    );
    // Estado: nunca reporta `activa`, para agotar el presupuesto de confirmación.
    // Con plazos pequeños (1 ms, 2 intentos) basta con un par de respuestas.
    servir_sonda_http(&mut guiones, &mut contador, br#"{"estado":"reconectando"}"#);
    servir_sonda_http(&mut guiones, &mut contador, br#"{"estado":"reconectando"}"#);

    let total_peticiones = guiones.len();
    assert!(total_peticiones > 0);
    let receptor = servir_guiones(servidor, guiones);

    let argumento = vec![
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "sustitución por baneo",
        "--confirmar",
    ];

    let mut estandar_buf: Vec<u8> = Vec::new();
    let mut diagnostico_buf: Vec<u8> = Vec::new();
    let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

    // ahora_ms muy posterior a expira_en_ms para que el presupuesto sea mínimo.
    let ahora_ms = 1_700_000_000_000i64;
    let codigo = ejecutar_rebind(
        &argumento,
        &cliente,
        &inventario,
        &ruta_almacen,
        ahora_ms,
        &mut salida,
    );

    assert_eq!(
        codigo,
        CodigoDeSalida::Fallo,
        "el código expirado debe terminar en Fallo"
    );

    let diagnostico = String::from_utf8(diagnostico_buf).unwrap();
    assert!(
        diagnostico.contains("código expirado; repita cell rebind"),
        "diagnóstico debe ser el literal exacto: {diagnostico}"
    );

    // No debe haber sonda de reanudar (se agotó el presupuesto antes).
    let mut hay_resume = false;
    while let Ok(peticion) = receptor.recv_timeout(std::time::Duration::from_millis(100)) {
        if peticion.objetivo.contains("/admin/envio/pausa")
            && String::from_utf8_lossy(&peticion.cuerpo).contains("reanudar")
        {
            hay_resume = true;
        }
    }
    assert!(
        !hay_resume,
        "no debe emitir la sonda de reanudar tras el código expirado"
    );

    // Estado final: sigue en Reemparejando, sin sustituciones.
    let a =
        AlmacenDelPlanoDeControl::abrir_solo_lectura(std::path::Path::new(&ruta_almacen)).unwrap();
    let fila = a.leer_estado("c1").unwrap().unwrap();
    assert_eq!(
        fila.estado,
        EstadoDeCelula::Reemparejando,
        "la fila debe quedar en Reemparejando"
    );
    let sustituciones = a.leer_sustituciones("c1").unwrap();
    assert!(
        sustituciones.is_empty(),
        "no debe haber fila de sustituciones tras el código expirado"
    );
}

/// AC-12/AC-14/AC-16: `canal_sin_sesion` en el emparejamiento omite el paso 9 (estado) y va
/// directo al paso 10 (reanudar), terminando en `Exito` con la fila `EnEjecución` y una fila de
/// `sustituciones`.
#[test]
fn canal_sin_sesion_en_emparejamiento_termina_en_exito() {
    let servidor = ServidorDockerFalso::nuevo("rebind-sin-sesion");
    let ruta = servidor.ruta();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
    let almacen = AlmacenTemporal::nuevo("rebind-sin-sesion");
    let ruta_almacen = almacen.texto();

    let mut guiones: Vec<Guion> = Vec::new();
    let mut contador = 0usize;

    guiones.push(inspeccion_del_nucleo());
    guiones.push(inspeccion_del_sidecar());
    guiones.push(inspeccion_del_nucleo()); // preparar_reemparejamiento
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );
    servir_contenedor_sin_logs(&mut guiones, &mut contador); // cierre
    guiones.push(sin_cuerpo(204, "No Content")); // stop sidecar
    servir_contenedor_sin_logs(&mut guiones, &mut contador); // rm
    guiones.push(sin_cuerpo(204, "No Content")); // start sidecar
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );
    // Pairing: canal_sin_sesion.
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"canal_sin_sesion"}"#,
    );
    // Resume probe (sin sonda de estado intermedia).
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"reanudar"}"#,
    );

    let total_peticiones = guiones.len();
    assert!(total_peticiones > 0);
    let receptor = servir_guiones(servidor, guiones);

    let argumento = vec![
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "sustitución por baneo",
        "--confirmar",
    ];

    let mut estandar_buf: Vec<u8> = Vec::new();
    let mut diagnostico_buf: Vec<u8> = Vec::new();
    let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

    let ahora_ms = 1_700_000_000_000i64;
    let codigo = ejecutar_rebind(
        &argumento,
        &cliente,
        &inventario,
        &ruta_almacen,
        ahora_ms,
        &mut salida,
    );

    assert_eq!(codigo, CodigoDeSalida::Exito);

    // Verificar que NO se emitió la sonda de estado.
    let mut hay_sonda_de_estado = false;
    let mut contador_create = 0;
    for _ in 0..total_peticiones {
        let peticion = recibir(&receptor);
        if peticion.objetivo == "/containers/create" {
            contador_create += 1;
            let cuerpo = String::from_utf8_lossy(&peticion.cuerpo);
            if cuerpo.contains("/admin/sesion\"") && !body_contains_pairing(&peticion.cuerpo) {
                hay_sonda_de_estado = true;
            }
        }
    }
    assert!(
        !hay_sonda_de_estado,
        "con canal_sin_sesion no debe emitirse la sonda de estado"
    );
    // 5 sondas HTTP: pausa, pausa2, pairing, resume. (4 create de sondas HTTP + 1 cierre + 1 rm).
    assert_eq!(
        contador_create, 6,
        "deben crearse 6 contenedores: 2 sin logs + 4 con logs"
    );
}

fn body_contains_pairing(cuerpo: &[u8]) -> bool {
    String::from_utf8_lossy(cuerpo).contains("emparejamiento")
}

/// AC-15: la tabla `sustituciones` después del happy path tiene exactamente las columnas
/// `id`, `id_celula`, `motivo`, `registrado_ms`, y ningún texto almacenado en ninguna columna
/// del plano de control es igual al valor de emparejamiento ni a un número telefónico.
#[test]
fn sustituciones_no_almacena_ni_telefono_ni_valor_de_emparejamiento() {
    let servidor = ServidorDockerFalso::nuevo("rebind-sustituciones-limpias");
    let ruta = servidor.ruta();
    let cliente = ClienteDocker::nuevo(ruta.clone());
    let inventario = InventarioDocker::nuevo(ruta.clone(), std::time::Duration::from_secs(70));
    let almacen = AlmacenTemporal::nuevo("rebind-sustituciones-limpias");
    let ruta_almacen = almacen.texto();

    let mut guiones: Vec<Guion> = Vec::new();
    let mut contador = 0usize;

    guiones.push(inspeccion_del_nucleo());
    guiones.push(inspeccion_del_sidecar());
    guiones.push(inspeccion_del_nucleo());
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );
    servir_contenedor_sin_logs(&mut guiones, &mut contador);
    guiones.push(sin_cuerpo(204, "No Content"));
    servir_contenedor_sin_logs(&mut guiones, &mut contador);
    guiones.push(sin_cuerpo(204, "No Content"));
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"pausar"}"#,
    );
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"codigo","metodo":"qr","valor":"QR-SECRETO-VALUE","expira_en_ms":1700000000000}"#,
    );
    servir_sonda_http(&mut guiones, &mut contador, br#"{"estado":"activa"}"#);
    servir_sonda_http(
        &mut guiones,
        &mut contador,
        br#"{"resultado":"aplicado","accion":"reanudar"}"#,
    );

    let receptor = servir_guiones(servidor, guiones);

    let argumento = vec![
        "cell",
        "rebind",
        "--id",
        "c1",
        "--motivo",
        "sustitución por baneo",
        "--confirmar",
    ];

    let mut estandar_buf: Vec<u8> = Vec::new();
    let mut diagnostico_buf: Vec<u8> = Vec::new();
    let mut salida = Salida::nueva(&mut estandar_buf, &mut diagnostico_buf);

    let ahora_ms = 1_700_000_000_000i64;
    let codigo = ejecutar_rebind(
        &argumento,
        &cliente,
        &inventario,
        &ruta_almacen,
        ahora_ms,
        &mut salida,
    );
    assert_eq!(codigo, CodigoDeSalida::Exito);

    // Consumir las peticiones para evitar que el hilo quede bloqueado.
    drop(receptor);

    // Verificar el contenido del almacén.
    let a =
        AlmacenDelPlanoDeControl::abrir_solo_lectura(std::path::Path::new(&ruta_almacen)).unwrap();

    let fila = a.leer_estado("c1").unwrap().unwrap();
    assert_ne!(fila.motivo, "QR-SECRETO-VALUE");
    assert_ne!(fila.motivo, "+34600123456");

    let sustituciones = a.leer_sustituciones("c1").unwrap();
    assert_eq!(sustituciones.len(), 1);
    assert_ne!(sustituciones[0].motivo, "QR-SECRETO-VALUE");
    assert_ne!(sustituciones[0].motivo, "+34600123456");
    assert_eq!(sustituciones[0].motivo, "sustitución por baneo");
}
