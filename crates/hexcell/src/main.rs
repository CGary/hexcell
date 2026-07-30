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
//! Sin manejo de señales: el apagado ordenado y el drenaje en vuelo son una etapa posterior del
//! plan (HEX-008, tarea 12).

use std::process::ExitCode;
use std::sync::Arc;

use hexcell::configuracion::{CanalSeleccionado, Configuracion};
use hexcell::motor::Motor;
use hexcell::preparacion::SesionDelCanal;
use hexcell::procesador::ProcesadorDeEco;
use hexcell::salud::{EstadoDeSalud, servir_salud};
use hexcell_canal_simulado::{AdaptadorSimulado, RelojDelSistema};
use hexcell_storage::{GestorDePools, RepositorioDeSesiones};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let configuracion = match Configuracion::desde_entorno() {
        Ok(configuracion) => configuracion,
        Err(error) => {
            eprintln!("hexcell: error de configuración: {error}");
            return ExitCode::FAILURE;
        }
    };

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

    let repositorio = Arc::new(RepositorioDeSesiones::nuevo(Arc::clone(&pools)));
    let estado_de_salud = Arc::new(EstadoDeSalud::nuevo(
        Arc::clone(&pools),
        SesionDelCanal::siempre_activa(),
    ));

    let (direccion_salud, servidor_salud) =
        match servir_salud(configuracion.direccion_salud, estado_de_salud).await {
            Ok(vinculado) => vinculado,
            Err(error) => {
                eprintln!(
                    "hexcell: no se pudo vincular el servidor de salud en {}: {error}",
                    configuracion.direccion_salud
                );
                return ExitCode::FAILURE;
            }
        };
    println!("hexcell: servidor de salud escuchando en {direccion_salud}");

    match configuracion.canal {
        CanalSeleccionado::Simulado => {
            println!("hexcell: canal configurado: simulado");
            let reloj = Arc::new(RelojDelSistema);
            let (adaptador, receptor_eventos) =
                AdaptadorSimulado::nuevo(reloj, configuracion.capacidad_cola);
            let mut motor = Motor::nuevo(
                adaptador,
                ProcesadorDeEco,
                receptor_eventos,
                configuracion.ventana_deduplicacion,
                repositorio,
            );

            tokio::select! {
                () = servidor_salud => {}
                () = motor.ejecutar() => {}
            }
        }
    }

    ExitCode::SUCCESS
}
