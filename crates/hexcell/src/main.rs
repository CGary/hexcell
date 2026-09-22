//! Binario del núcleo de una célula: raíz de composición.
//!
//! Lee la configuración de variables de entorno, y si falta algo o no parsea, termina **antes**
//! de vincular cualquier puerto o de arrancar el motor de mensajería, imprimiendo en `stderr` el
//! mensaje que nombra la variable concreta. Esto es lo que hace verificable
//! `[profile.release]`'s `panic = "abort"`: en release un `panic` no deja ningún mensaje
//! utilizable, así que este binario nunca depende de uno para reportar un error de arranque.
//!
//! El mismo criterio gobierna la persistencia: las dos bases de la persistencia dual de FR-05
//! —`sessions.db` y `knowledge_live.db`, ambas derivadas de la ruta de datos ya validada— se
//! abren y se migran **antes** de vincular el servidor de salud. Si eso falla, la célula termina
//! por `stderr` y `ExitCode::FAILURE` sin llegar a anunciarse como viva; ninguna variable de
//! entorno nueva participa en esto, porque las rutas se derivan y los parámetros de SQLite son
//! constantes con nombre en `hexcell-storage`.
//!
//! Con configuración válida: construye el adaptador de canal configurado (hoy solo el simulado;
//! la selección es un `match` estático porque `ChannelAdapter` usa `-> impl Future` y por tanto no
//! es compatible con objetos de trait, `docs/adr/adr-0002-estructura-workspace.md`), levanta el
//! servidor de salud y ejecuta el motor de mensajería, ambos sobre un único runtime
//! `current_thread` porque una célula sirve tráfico bajo y un pool de hilos por célula es la
//! contrapartida equivocada en el hardware objetivo de NFR-01.
//!
//! El estado de sesión del canal se decide **aquí**, en la composición, y no se lee del puerto:
//! `ChannelAdapter` no expone ninguna consulta de sesión y esta tarea no lo reabre para inventarla
//! (el porqué completo está en `crate::preparacion`).
//!
//! # Apagado ordenado, inferencia y registro (HEX-007)
//!
//! El manejador de señales se registra **nada más** analizar la configuración, antes de tocar
//! disco o red, para que un `SIGTERM` que llegara durante el arranque quede capturado en vez de
//! matar el proceso con la acción por defecto del sistema operativo. El registro estructurado se
//! inicializa justo después, para que toda línea posterior lleve ya el identificador de célula.
//! Tras el bucle principal (`tokio::select!` entre el servidor de salud y el motor), se ejecuta el
//! punto de control del WAL sobre ambos pools y el proceso termina siempre con
//! `ExitCode::SUCCESS`: un punto de control que falla se registra, pero no es un fallo de salida,
//! porque un WAL sin consolidar no es pérdida de datos.
//!
//! El evento sintético de arranque (`HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE`) se inyecta **antes**
//! de que `Motor::nuevo` tome posesión del adaptador, así que no hace falta compartirlo por
//! `Arc` ni envolverlo en un delegador: se inyecta a través de
//! `AdaptadorSimulado::inyectar_desde_contacto`, que es quien traduce el contacto sintético a un
//! `IdConversacion` (`adr-0010`) — `main` no construye ninguno. `IdDeduplicacion::nuevo` aparece
//! en este archivo y solo en él, precisamente porque con un canal real el identificador de evento
//! siempre llega ya traducido desde el transporte a través del adaptador.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;

use hexcell::admin::{
    CierreDeSesion, EstadoDeAdmin, PLAZO_DE_CIERRE_DE_SESION, RegistroDeCierreDeSesion,
    servir_servicios_http,
};
use hexcell::alertas::EmisorDeAlertas;
use hexcell::apagado::Apagado;
use hexcell::concurrencia::LimitadorDeConcurrencia;
use hexcell::configuracion::{
    CanalSeleccionado, Configuracion, ConfiguracionDeEmbeddingsSegunProveedor, EntornoDelProceso,
};
use hexcell::embeddings::{
    ProveedorDeEmbeddingsDeCelula, ProveedorDeEmbeddingsSimulado, ServicioDeEmbeddings,
};
use hexcell::emparejar;
use hexcell::inferencia::{ProveedorDeCelula, ProveedorSimulado};
use hexcell::metricas::{
    INTERVALO_DE_INSTANTANEA, RegistroDeMetricas, emitir_instantanea, tomar_instantanea,
};
use hexcell::motor::Motor;
use hexcell::notificacion::SumideroDeCelula;
use hexcell::preparacion::SesionDelCanal;
use hexcell::procesador::ProcesadorDeInferencia;
use hexcell::proveedor_embeddings::ProveedorDeEmbeddingsOpenRouter;
use hexcell::proveedor_embeddings_gemini::ProveedorDeEmbeddingsGemini;
use hexcell::proveedor_openai::ProveedorOpenAi;
use hexcell::registro::{self, EntradaDeRegistro, NivelDeRegistro};
use hexcell::salud::EstadoDeSalud;
use hexcell_canal_simulado::{AdaptadorSimulado, RelojDelSistema};
use hexcell_canal_whatsmeow::{AdaptadorWhatsmeow, Retroceso};
use hexcell_core::identidad::IdDeduplicacion;
use hexcell_storage::{
    AlmacenDeIdentidad, GestorDePools, RepositorioDeSesiones, ResumenDePuntoDeControl,
};

/// Contacto sintético que recibe el evento de arranque cuando
/// `HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE` está presente.
const CONTACTO_DEL_EVENTO_DE_ARRANQUE: &str = "arranque-simulado";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let argumentos: Vec<String> = std::env::args().collect();
    // Raíz de composición: es aquí, y solo aquí, donde se elige que la configuración salga del
    // entorno real del proceso. Todo lo que hay por debajo recibe la fuente como parámetro.
    let fuente = EntornoDelProceso;
    if argumentos.get(1).map(String::as_str) == Some("emparejar") {
        return emparejar::ejecutar_cli(&argumentos[2..], &fuente).await;
    }
    if argumentos.get(1).map(String::as_str) == Some("respaldar") {
        return hexcell::respaldar::ejecutar_cli(&argumentos[2..], &fuente).await;
    }

    let configuracion = match Configuracion::desde_entorno() {
        Ok(configuracion) => configuracion,
        Err(error) => {
            eprintln!("hexcell: error de configuración: {error}");
            return ExitCode::FAILURE;
        }
    };

    let (_apagado, senal_de_apagado) = match Apagado::instalar(configuracion.limite_de_drenaje) {
        Ok(instalado) => instalado,
        Err(error) => {
            eprintln!("hexcell: no se pudo instalar el manejador de señales: {error}");
            return ExitCode::FAILURE;
        }
    };

    registro::inicializar(configuracion.id_celula.clone());

    println!(
        "hexcell: célula {} arrancando; ruta de datos {}",
        configuracion.id_celula,
        configuracion.ruta_datos.display()
    );

    let pools = match GestorDePools::abrir(&configuracion.ruta_datos) {
        Ok(pools) => Arc::new(pools),
        Err(error) => {
            eprintln!(
                "hexcell: no se pudo abrir la persistencia en {}: {error}",
                configuracion.ruta_datos.display()
            );
            return ExitCode::FAILURE;
        }
    };
    println!("hexcell: persistencia dual abierta y migrada");

    // Almacén de identidad del adaptador (adr-0010, puntos 5 y 6): propio del adaptador y no del
    // gestor de pools del núcleo, con la misma disciplina de fallo que las dos bases anteriores.
    // Se abre aquí, en la composición, para que main —y no GestorDePools— sea quien decide su
    // dueño; ruta derivada de la misma ruta de datos ya validada, sin variable de entorno nueva.
    let almacen_de_identidad = match AlmacenDeIdentidad::abrir(&configuracion.ruta_datos) {
        Ok(almacen) => Arc::new(almacen),
        Err(error) => {
            eprintln!(
                "hexcell: no se pudo abrir el almacén de identidad del adaptador en {}: {error}",
                configuracion.ruta_datos.display()
            );
            return ExitCode::FAILURE;
        }
    };
    println!("hexcell: almacén de identidad del adaptador abierto y migrado");

    let repositorio = Arc::new(RepositorioDeSesiones::nuevo(Arc::clone(&pools)));

    let limitador = LimitadorDeConcurrencia::nuevo(configuracion.limite_de_concurrencia);
    let metricas = Arc::new(RegistroDeMetricas::nuevo());

    let sumidero = SumideroDeCelula::desde_configuracion(configuracion.notificaciones.clone());
    let emisor_alertas = Arc::new(EmisorDeAlertas::nuevo(
        configuracion.umbrales_de_alerta.clone(),
        sumidero,
    ));

    let _metricas_task = {
        let metricas = Arc::clone(&metricas);
        let limitador = limitador.clone();
        let repositorio = Arc::clone(&repositorio);
        let emisor = Arc::clone(&emisor_alertas);
        tokio::spawn(async move {
            let mut intervalo = tokio::time::interval(INTERVALO_DE_INSTANTANEA);
            loop {
                intervalo.tick().await;
                if let Ok(instantanea) = tomar_instantanea(&metricas, &limitador, &repositorio) {
                    emitir_instantanea(&instantanea);
                    let rechazos = hexcell_core::canal::rechazos_de_construccion();
                    emisor
                        .evaluar_y_emitir_instantanea(&instantanea, rechazos)
                        .await;
                }
            }
        })
    };

    if configuracion.presupuesto_inicial_unidades > 0 {
        match repositorio.presupuesto_sin_iniciar() {
            Ok(true) => {
                if let Err(error) = repositorio.aportar_presupuesto(
                    configuracion.presupuesto_inicial_unidades,
                    std::time::SystemTime::now(),
                ) {
                    eprintln!("hexcell: no se pudo aportar el presupuesto inicial: {error}");
                }
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!("hexcell: error al consultar estado de presupuesto inicial: {error}");
            }
        }
    }

    let receptor_apagado = senal_de_apagado.observador();
    let debe_apagar = move || *receptor_apagado.borrow();

    let estado_de_salud = Arc::new(EstadoDeSalud::nuevo(
        Arc::clone(&pools),
        SesionDelCanal::siempre_activa(),
    ));
    let estado_de_admin = Arc::new(EstadoDeAdmin::nuevo());

    let proveedor_embeddings = match &configuracion.embeddings {
        Some(ConfiguracionDeEmbeddingsSegunProveedor::OpenRouter(cfg)) => {
            let p = ProveedorDeEmbeddingsOpenRouter::nuevo(cfg.clone());
            ProveedorDeEmbeddingsDeCelula::OpenRouter(Box::new(p))
        }
        Some(ConfiguracionDeEmbeddingsSegunProveedor::Gemini(cfg)) => {
            let p = ProveedorDeEmbeddingsGemini::nuevo(cfg.clone());
            ProveedorDeEmbeddingsDeCelula::Gemini(Box::new(p))
        }
        None => ProveedorDeEmbeddingsDeCelula::Simulado(ProveedorDeEmbeddingsSimulado::nuevo()),
    };
    let servicio_embeddings = Arc::new(ServicioDeEmbeddings::nuevo(
        proveedor_embeddings,
        Arc::clone(&repositorio),
    ));

    // Un solo futuro para las dos superficies HTTP: cada `tokio::select!` de más abajo enumera sus
    // ramas a mano, una por canal, y dos futuros independientes se podrían enumerar en uno y
    // olvidar en el otro, dejando el endpoint inexistente en ese canal sin que nada fallara.
    //
    // El registro de cierre de sesión se crea aquí y se pasa al futuro combinado; la raíz de
    // composición lo rellena tarde, desde la rama del `match` sobre `CanalSeleccionado`, porque
    // el futuro se construye antes de conocer el canal.
    let registro_cierre: RegistroDeCierreDeSesion = std::sync::Arc::new(std::sync::OnceLock::new());
    let ((direccion_salud, direccion_admin), servidores_http) = match servir_servicios_http(
        configuracion.direccion_salud,
        estado_de_salud,
        configuracion.direccion_admin,
        configuracion.limite_de_cuerpo_admin,
        estado_de_admin,
        servicio_embeddings,
        configuracion.ruta_datos.clone(),
        debe_apagar,
        Arc::clone(&registro_cierre),
        PLAZO_DE_CIERRE_DE_SESION,
    )
    .await
    {
        Ok(vinculados) => vinculados,
        Err(error) => {
            eprintln!("hexcell: no se pudo vincular el {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("hexcell: servidor de salud escuchando en {direccion_salud}");
    registro::emitir(
        EntradaDeRegistro::nueva(NivelDeRegistro::Info, "salud_vinculada")
            .con_detalle(direccion_salud.to_string()),
    );
    println!("hexcell: servidor de administración escuchando en {direccion_admin}");
    registro::emitir(
        EntradaDeRegistro::nueva(NivelDeRegistro::Info, "admin_vinculada")
            .con_detalle(direccion_admin.to_string()),
    );

    let proveedor = match &configuracion.inferencia {
        Some(cfg_inferencia) => {
            let proveedor_openai = ProveedorOpenAi::nuevo(cfg_inferencia.clone());
            ProveedorDeCelula::OpenAi(Box::new(proveedor_openai))
        }
        None => {
            let simulado = if configuracion.proveedor_de_inferencia_falla {
                ProveedorSimulado::que_falla()
            } else {
                ProveedorSimulado::con_latencia(configuracion.latencia_inferencia_simulada)
            };
            ProveedorDeCelula::Simulado(simulado)
        }
    };

    match configuracion.canal {
        CanalSeleccionado::Simulado => {
            println!("hexcell: canal configurado: simulado");
            let reloj = Arc::new(RelojDelSistema);
            let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo_con_almacen(
                reloj,
                configuracion.capacidad_cola,
                Arc::clone(&almacen_de_identidad),
            );

            // El canal simulado no vincula ningún dispositivo: no hay sesión que cerrar.
            // La política ratificada (R5, 2026-09-22) es «no hay sesión = completado», con
            // motivo `canal_sin_sesion`. El adaptador simulado no implementa
            // `CicloDeVidaSesion` a propósito.
            let _ = CierreDeSesion::SinSesion.registrar(&registro_cierre);

            if let Some(contenido) = configuracion.evento_simulado_de_arranque.clone() {
                // Único lugar de `crates/hexcell/src/` donde se construye un `IdDeduplicacion`:
                // con un canal real, ese identificador siempre llega ya traducido por el
                // adaptador desde el transporte. Aquí no hay transporte, así que este evento
                // sintético necesita uno propio.
                let deduplicacion = IdDeduplicacion::nuevo("evento-simulado-de-arranque");
                if let Err(error) = adaptador
                    .inyectar_desde_contacto(
                        CONTACTO_DEL_EVENTO_DE_ARRANQUE,
                        contenido,
                        deduplicacion,
                    )
                    .await
                {
                    eprintln!(
                        "hexcell: no se pudo inyectar el evento simulado de arranque: {error}"
                    );
                }
            }

            let procesador =
                ProcesadorDeInferencia::nuevo(proveedor.clone(), Arc::clone(&repositorio));
            let mut motor = Motor::nuevo(
                adaptador,
                procesador,
                receptor_eventos,
                configuracion.ventana_deduplicacion,
                repositorio,
            )
            .con_configuracion_gcra(configuracion.configuracion_gcra.clone())
            .con_limite_de_concurrencia(limitador.clone())
            .con_metricas(metricas.clone());

            tokio::select! {
                () = servidores_http => {}
                () = motor.ejecutar(senal_de_apagado) => {}
            }
        }
        CanalSeleccionado::Whatsmeow => {
            println!("hexcell: canal configurado: whatsmeow");
            let (adaptador, receptor_eventos) = AdaptadorWhatsmeow::nuevo(
                configuracion.ruta_socket_ipc.clone(),
                configuracion.id_celula.clone(),
                configuracion.capacidad_cola,
                Retroceso::por_omision(),
            );
            adaptador.arrancar();

            // Asa de sesión para el cierre: se toma ANTES de que `Motor::nuevo` consuma el
            // adaptador, siguiendo el precedente de `contadores_de_acuse()` y
            // `suscribir_estado_con_expiracion()`. El motivo «cell terminate» identifica el
            // cierre ordenado por el operador, distinguiéndolo del cierre por trait (motivo
            // vacío) que usa el sub-trait `CicloDeVidaSesion`.
            let asa = adaptador.asa_de_sesion("cell terminate");
            let _ = CierreDeSesion::con_sesion(asa).registrar(&registro_cierre);

            let mut receptor_estado_alertas = adaptador.suscribir_estado_con_expiracion();
            let contadores = adaptador.contadores_de_acuse().clone();
            let emisor = Arc::clone(&emisor_alertas);

            // Observador del estado de sesión para las alertas AC-2, AC-3 y AC-4.
            //
            // Reacciona a los cambios del par (estado, expiración) —que el adaptador publica en un
            // único envío, sin ventana entre ambos— y además **reevalúa periódicamente el último
            // estado observado**. La reevaluación periódica no es un adorno: la condición AC-4
            // («el sidecar no reconecta pasada la ventana configurada») es temporal, y el sidecar
            // emite `reconectando` una sola vez por desconexión. Sin un disparo por reloj, un
            // estado `Reconectando` persistente no volvería a evaluarse nunca y la ventana jamás
            // se cruzaría. La regla de «exactamente una» vive en el evaluador, así que reevaluar
            // una condición ya alertada no produce una segunda notificación.
            let _alertas_sesion_task = tokio::spawn(async move {
                let mut reevaluacion = tokio::time::interval(INTERVALO_DE_INSTANTANEA);
                loop {
                    tokio::select! {
                        resultado = receptor_estado_alertas.changed() => {
                            if resultado.is_err() {
                                break;
                            }
                        }
                        _ = reevaluacion.tick() => {}
                    }
                    let (estado, expira_en) = *receptor_estado_alertas.borrow();
                    emisor
                        .evaluar_y_emitir_estado(estado, expira_en, SystemTime::now())
                        .await;
                }
            });

            let _alertas_acuses_task = {
                let emisor = Arc::clone(&emisor_alertas);
                tokio::spawn(async move {
                    let mut intervalo = tokio::time::interval(INTERVALO_DE_INSTANTANEA);
                    loop {
                        intervalo.tick().await;
                        let instantanea_de_contadores = contadores.instantanea().await;
                        emisor
                            .evaluar_y_emitir_acuses(&instantanea_de_contadores)
                            .await;
                    }
                })
            };

            let procesador = ProcesadorDeInferencia::nuevo(proveedor, Arc::clone(&repositorio));
            let mut motor = Motor::nuevo(
                adaptador,
                procesador,
                receptor_eventos,
                configuracion.ventana_deduplicacion,
                repositorio,
            )
            .con_configuracion_gcra(configuracion.configuracion_gcra.clone())
            .con_limite_de_concurrencia(limitador.clone())
            .con_metricas(metricas.clone());

            tokio::select! {
                () = servidores_http => {}
                () = motor.ejecutar(senal_de_apagado) => {}
            }
        }
    }

    emitir_punto_de_control(pools.punto_de_control_de_wal());

    ExitCode::SUCCESS
}

/// Registra el resultado del punto de control del WAL de apagado.
fn emitir_punto_de_control(resumen: ResumenDePuntoDeControl) {
    let nivel = if resumen.ocupado {
        NivelDeRegistro::Aviso
    } else {
        NivelDeRegistro::Info
    };
    registro::emitir(
        EntradaDeRegistro::nueva(nivel, "punto_de_control_wal").con_detalle(format!(
            "ocupado={} wal_sesiones_bytes={}",
            resumen.ocupado, resumen.tamano_wal_de_sesiones_bytes
        )),
    );
}
