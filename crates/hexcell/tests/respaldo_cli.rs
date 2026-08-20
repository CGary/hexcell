mod comun;

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use comun::{DirectorioTemporal, abrir_persistencia_con_identidad};
use hexcell::respaldar::{ErrorModoRespaldar, analizar_argumentos, ejecutar};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

static CONTADOR_RUTAS: AtomicUsize = AtomicUsize::new(0);

struct FakeSidecar {
    ruta_socket: PathBuf,
    listener: UnixListener,
    conexion: Option<(
        BufReader<tokio::io::ReadHalf<UnixStream>>,
        tokio::io::WriteHalf<UnixStream>,
    )>,
}

impl FakeSidecar {
    fn nuevo() -> Self {
        let mut ruta = std::env::temp_dir();
        ruta.push(format!(
            "hexcell-fake-sidecar-cli-{}-{}",
            std::process::id(),
            CONTADOR_RUTAS.fetch_add(1, Ordering::SeqCst)
        ));

        let listener = UnixListener::bind(&ruta).expect("vincular socket unix");

        Self {
            ruta_socket: ruta,
            listener,
            conexion: None,
        }
    }

    fn ruta_socket(&self) -> &PathBuf {
        &self.ruta_socket
    }

    async fn aceptar_y_saludar(&mut self, id_celula: &str) {
        let (stream, _) = self.listener.accept().await.expect("aceptar conexion");
        let (lectura, mut escritura) = tokio::io::split(stream);
        let mut lector = BufReader::new(lectura);

        let mut linea_saludo = String::new();
        lector.read_line(&mut linea_saludo).await.unwrap();

        let saludo_sidecar = format!(
            "{{\"version\":5,\"tipo\":\"saludo\",\"emisor\":\"sidecar\",\"id_celula\":\"{id_celula}\"}}\n"
        );
        escritura
            .write_all(saludo_sidecar.as_bytes())
            .await
            .unwrap();
        escritura.flush().await.unwrap();

        self.conexion = Some((lector, escritura));
    }

    async fn leer_linea(&mut self) -> String {
        let con = self.conexion.as_mut().expect("sin conexion");
        let mut linea = String::new();
        con.0.read_line(&mut linea).await.unwrap();
        linea.trim_end().to_string()
    }

    async fn enviar_linea(&mut self, linea: &str) {
        let con = self.conexion.as_mut().expect("sin conexion");
        con.1.write_all(linea.as_bytes()).await.unwrap();
        con.1.write_all(b"\n").await.unwrap();
        con.1.flush().await.unwrap();
    }

    /// Lee una orden de respaldo por IPC, escribe la copia simulada bajo su nombre canónico y
    /// responde con el acuse del TIPO que corresponde (sqlstore o identidad). Devuelve el nombre
    /// canónico que atendió, para que el test pueda comprobar qué base se ordenó.
    async fn atender_orden_respaldo(&mut self, destino: &std::path::Path) -> &'static str {
        let orden = self.leer_linea().await;
        let ronda = extraer_identificador_de_ronda(&orden);
        let (tipo_acuse, nombre) = if orden.contains("\"tipo\":\"orden_respaldo_identidad\"") {
            ("acuse_respaldo_identidad", "identidad.db")
        } else {
            ("acuse_respaldo_sqlstore", "sqlstore.db")
        };
        let copia = destino.join(nombre);
        std::fs::write(&copia, b"datos-ipc-simulados").unwrap();
        let acuse = format!(
            "{{\"version\":5,\"tipo\":\"{tipo_acuse}\",\"identificador_de_ronda\":\"{ronda}\",\"resultado\":\"completado\",\"ruta_de_la_copia\":\"{}\",\"bytes\":19,\"motivo\":\"\"}}",
            copia.to_string_lossy()
        );
        self.enviar_linea(&acuse).await;
        nombre
    }
}

impl Drop for FakeSidecar {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ruta_socket);
    }
}

#[test]
fn analizar_argumentos_valido_y_errores() {
    let args = vec!["--directorio".to_string(), "/tmp/respaldo-abs".to_string()];
    let res = analizar_argumentos(&args).unwrap();
    assert_eq!(res, PathBuf::from("/tmp/respaldo-abs"));

    let args = vec!["--directorio=/tmp/respaldo-junto".to_string()];
    let res = analizar_argumentos(&args).unwrap();
    assert_eq!(res, PathBuf::from("/tmp/respaldo-junto"));

    let args = vec!["--directorio".to_string()];
    match analizar_argumentos(&args) {
        Err(ErrorModoRespaldar::FaltaDirectorio) => {}
        _ => panic!("se esperaba FaltaDirectorio"),
    }

    let args = vec!["--directorio".to_string(), "ruta/relativa".to_string()];
    match analizar_argumentos(&args) {
        Err(ErrorModoRespaldar::DirectorioRelativo) => {}
        _ => panic!("se esperaba DirectorioRelativo"),
    }

    let args = vec!["--desconocido".to_string()];
    match analizar_argumentos(&args) {
        Err(ErrorModoRespaldar::ArgumentoDesconocido(arg)) => {
            assert_eq!(arg, "--desconocido");
        }
        _ => panic!("se esperaba ArgumentoDesconocido"),
    }
}

fn extraer_identificador_de_ronda(linea: &str) -> String {
    let clave = "\"identificador_de_ronda\":\"";
    if let Some(pos) = linea.find(clave) {
        let resto = &linea[pos + clave.len()..];
        if let Some(fin) = resto.find('"') {
            return resto[..fin].to_string();
        }
    }
    String::new()
}

#[tokio::test]
async fn ejecutar_respaldo_exitoso_cinco_bases() {
    let mut sidecar = FakeSidecar::nuevo();
    let id_celula = "celula-respaldo-cli-1";

    let origen_temp = DirectorioTemporal::nuevo("respaldo-cli-origen");
    let (_pools, _repo, _almacen) = abrir_persistencia_con_identidad(origen_temp.ruta());

    let destino_temp = DirectorioTemporal::nuevo("respaldo-cli-destino");
    let destino_path = destino_temp.ruta().to_path_buf();
    let socket_path = sidecar.ruta_socket().clone();
    let origen_path = origen_temp.ruta().to_path_buf();

    let tarea_ejecutar = tokio::spawn(async move {
        ejecutar(
            &socket_path,
            id_celula,
            &origen_path,
            &destino_path,
            Duration::from_secs(5),
        )
        .await
    });

    sidecar.aceptar_y_saludar(id_celula).await;

    // Las DOS órdenes IPC (sqlstore, identidad) llegan ANTES que las tres copias locales
    // (PAT-038); el fake atiende cada una con el acuse de su tipo. El primero DEBE ser sqlstore.
    let primero = sidecar.atender_orden_respaldo(destino_temp.ruta()).await;
    assert_eq!(
        primero, "sqlstore.db",
        "el primer store IPC debe ser el sqlstore"
    );
    let segundo = sidecar.atender_orden_respaldo(destino_temp.ruta()).await;
    assert_eq!(
        segundo, "identidad.db",
        "el segundo store IPC debe ser identidad"
    );

    let resumen = tarea_ejecutar
        .await
        .unwrap()
        .expect("ejecutar debe retornar Ok");

    // CINCO copias, no cuatro: si identidad.db se cae del conjunto, este conteo falla (LES-036).
    assert_eq!(resumen.copias.len(), 5);
    assert!(destino_temp.ruta().join("sqlstore.db").exists());
    assert!(destino_temp.ruta().join("identidad.db").exists());
    assert!(destino_temp.ruta().join("sessions.db").exists());
    assert!(destino_temp.ruta().join("knowledge_live.db").exists());
    assert!(destino_temp.ruta().join("adapter_identity.db").exists());
    // El resumen nombra explícitamente identidad.db entre sus copias.
    assert!(
        resumen
            .copias
            .iter()
            .any(|c| c.nombre_logico == "identidad.db"),
        "el resumen de respaldo debe incluir identidad.db"
    );
}

#[tokio::test]
async fn ejecutar_respaldo_fallido_sqlstore_deja_destino_vacio() {
    let mut sidecar = FakeSidecar::nuevo();
    let id_celula = "celula-respaldo-cli-fallo";

    let origen_temp = DirectorioTemporal::nuevo("respaldo-cli-origen-fallo");
    let (_pools, _repo, _almacen) = abrir_persistencia_con_identidad(origen_temp.ruta());

    let destino_temp = DirectorioTemporal::nuevo("respaldo-cli-destino-fallo");
    let destino_path = destino_temp.ruta().to_path_buf();
    let socket_path = sidecar.ruta_socket().clone();
    let origen_path = origen_temp.ruta().to_path_buf();

    let tarea_ejecutar = tokio::spawn(async move {
        ejecutar(
            &socket_path,
            id_celula,
            &origen_path,
            &destino_path,
            Duration::from_secs(5),
        )
        .await
    });

    sidecar.aceptar_y_saludar(id_celula).await;
    let orden = sidecar.leer_linea().await;
    let ronda_id = extraer_identificador_de_ronda(&orden);

    let acuse = format!(
        "{{\"version\":5,\"tipo\":\"acuse_respaldo_sqlstore\",\"identificador_de_ronda\":\"{ronda_id}\",\"resultado\":\"fallido\",\"ruta_de_la_copia\":\"\",\"bytes\":0,\"motivo\":\"espacio insuficiente en disco\"}}"
    );
    sidecar.enviar_linea(&acuse).await;

    let err = tarea_ejecutar
        .await
        .unwrap()
        .expect_err("ejecutar debe fallar");

    match &err {
        ErrorModoRespaldar::SqlstoreFallido { motivo } => {
            assert_eq!(motivo, "espacio insuficiente en disco");
        }
        _ => panic!("se esperaba SqlstoreFallido"),
    }

    let mensaje = err.to_string();
    assert!(
        mensaje.contains("sqlstore.db"),
        "el mensaje debe nombrar la base que falló: {mensaje}"
    );
    assert!(
        mensaje.contains("espacio insuficiente en disco"),
        "el mensaje debe incluir el motivo reportado por el sidecar: {mensaje}"
    );

    let entradas: Vec<_> = std::fs::read_dir(destino_temp.ruta())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entradas.len(), 0);
}

#[tokio::test]
async fn ejecutar_destino_ocupado_falla_antes_de_ipc() {
    let origen_temp = DirectorioTemporal::nuevo("respaldo-cli-ocupado-origen");
    let (_pools, _repo, _almacen) = abrir_persistencia_con_identidad(origen_temp.ruta());

    let destino_temp = DirectorioTemporal::nuevo("respaldo-cli-ocupado-destino");
    std::fs::write(destino_temp.ruta().join("sessions.db"), b"ocupado").unwrap();

    let socket_inexistente = PathBuf::from("/tmp/socket-no-existente-para-test.sock");
    let res = ejecutar(
        &socket_inexistente,
        "celula-ocupada",
        origen_temp.ruta(),
        destino_temp.ruta(),
        Duration::from_secs(1),
    )
    .await;

    match res {
        Err(ErrorModoRespaldar::Almacen(
            hexcell_storage::ErrorDeAlmacen::DestinoDeRespaldoOcupado { .. },
        )) => {}
        _ => panic!("se esperaba DestinoDeRespaldoOcupado sin intentar IPC"),
    }
}

#[tokio::test]
async fn binario_real_despacha_respaldar_con_exito() {
    let mut sidecar = FakeSidecar::nuevo();
    let socket_path = sidecar.ruta_socket().clone();

    let origen_temp = DirectorioTemporal::nuevo("respaldo-bin-origen");
    let (_pools, _repo, _almacen) = abrir_persistencia_con_identidad(origen_temp.ruta());
    let destino_temp = DirectorioTemporal::nuevo("respaldo-bin-destino");

    let bin_path = env!("CARGO_BIN_EXE_hexcell");
    let id_celula = "celula-bin-real";
    let destino_path = destino_temp.ruta().to_path_buf();

    let mut comando = Command::new(bin_path);
    comando
        .env_clear()
        .env("HEXCELL_ID_CELULA", id_celula)
        .env("HEXCELL_RUTA_DATOS", origen_temp.ruta())
        .env("HEXCELL_SOCKET_IPC", &socket_path)
        .arg("respaldar")
        .arg("--directorio")
        .arg(&destino_path);

    let sidecar_dest_path = destino_path.clone();
    let tarea_sidecar = tokio::spawn(async move {
        sidecar.aceptar_y_saludar(id_celula).await;
        // El binario ordena las dos bases IPC en secuencia: sqlstore y luego identidad.
        sidecar.atender_orden_respaldo(&sidecar_dest_path).await;
        sidecar.atender_orden_respaldo(&sidecar_dest_path).await;
    });

    let salida = tokio::task::spawn_blocking(move || {
        comando
            .output()
            .expect("ejecutar binario hexcell respaldar")
    })
    .await
    .unwrap();
    tarea_sidecar.await.unwrap();

    assert!(
        salida.status.success(),
        "el binario debe terminar con exit code 0; stderr:\n{}",
        String::from_utf8_lossy(&salida.stderr)
    );

    let stdout = String::from_utf8_lossy(&salida.stdout);
    assert!(stdout.contains("respaldo completado exitosamente"));
}

#[test]
fn binario_real_sin_argumento_falla_con_mensaje_espanol() {
    let bin_path = env!("CARGO_BIN_EXE_hexcell");
    let mut comando = Command::new(bin_path);
    comando
        .env_clear()
        .env("HEXCELL_ID_CELULA", "celula-error")
        .env("HEXCELL_RUTA_DATOS", "/tmp/datos")
        .arg("respaldar");

    let salida = comando.output().expect("ejecutar binario sin --directorio");
    assert!(!salida.status.success());

    let stderr = String::from_utf8_lossy(&salida.stderr);
    assert!(stderr.contains("falta el argumento obligatorio --directorio"));
}

#[tokio::test]
async fn binario_real_sidecar_rechaza_respaldo_falla_con_mensaje_espanol() {
    let mut sidecar = FakeSidecar::nuevo();
    let socket_path = sidecar.ruta_socket().clone();

    let origen_temp = DirectorioTemporal::nuevo("respaldo-bin-rechazo-origen");
    let (_pools, _repo, _almacen) = abrir_persistencia_con_identidad(origen_temp.ruta());
    let destino_temp = DirectorioTemporal::nuevo("respaldo-bin-rechazo-destino");

    let bin_path = env!("CARGO_BIN_EXE_hexcell");
    let id_celula = "celula-bin-rechazo";
    let destino_path = destino_temp.ruta().to_path_buf();

    let mut comando = Command::new(bin_path);
    comando
        .env_clear()
        .env("HEXCELL_ID_CELULA", id_celula)
        .env("HEXCELL_RUTA_DATOS", origen_temp.ruta())
        .env("HEXCELL_SOCKET_IPC", &socket_path)
        .arg("respaldar")
        .arg("--directorio")
        .arg(&destino_path);

    let tarea_sidecar = tokio::spawn(async move {
        sidecar.aceptar_y_saludar(id_celula).await;
        let orden = sidecar.leer_linea().await;
        let ronda_id = extraer_identificador_de_ronda(&orden);

        let acuse = format!(
            "{{\"version\":5,\"tipo\":\"acuse_respaldo_sqlstore\",\"identificador_de_ronda\":\"{ronda_id}\",\"resultado\":\"fallido\",\"ruta_de_la_copia\":\"\",\"bytes\":0,\"motivo\":\"sidecar rechazó el respaldo\"}}"
        );
        sidecar.enviar_linea(&acuse).await;
    });

    let salida = tokio::task::spawn_blocking(move || {
        comando
            .output()
            .expect("ejecutar binario hexcell respaldar")
    })
    .await
    .unwrap();
    tarea_sidecar.await.unwrap();

    assert!(
        !salida.status.success(),
        "el binario debe terminar con exit code distinto de 0"
    );

    let stderr = String::from_utf8_lossy(&salida.stderr);
    assert!(
        stderr.contains("sqlstore.db"),
        "stderr debe nombrar la base que falló: {stderr}"
    );
    assert!(
        stderr.contains("sidecar rechazó el respaldo"),
        "stderr debe incluir el motivo reportado por el sidecar: {stderr}"
    );
    assert!(
        stderr.contains("el directorio de destino NO contiene un respaldo válido"),
        "stderr debe advertir que el destino no quedó válido: {stderr}"
    );
    assert!(
        stderr.contains("directorio NUEVO y sin usar"),
        "stderr debe recordar la regla de directorio nuevo: {stderr}"
    );

    let entradas: Vec<_> = std::fs::read_dir(destino_temp.ruta())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entradas.len(), 0);
}
