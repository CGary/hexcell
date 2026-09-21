//! Binario de la CLI central de administración.
//!
//! Raíz de composición de `hexcell-admin`: recoge los argumentos del proceso, los entrega al
//! analizador de [`hexcell_admin::argumentos`], construye el sumidero de salida de producción de
//! [`hexcell_admin::salida`] y los despacha a [`hexcell_admin::comandos::ejecutar`], que
//! devuelve el [`hexcell_admin::codigo_de_salida::CodigoDeSalida`] que el proceso devuelve al
//! sistema operativo a través de `std::process::ExitCode`. Cuando la invocación no es una
//! simulación construye además el [`hexcell_admin::docker::ClienteDocker`] y despacha por
//! `comandos::ejecutar_con_efectos`.
//!
//! Este archivo no contiene lógica de análisis, ningún `match` sobre subcomandos y ningún texto
//! de mensaje propio: toda cadena y toda regla de despacho vive en los módulos de la biblioteca,
//! donde las pruebas externas de `crates/hexcell-admin/tests/` pueden ejercitarla. El esqueleto
//! de la etapa A-1 (`println!` de talón) desaparece aquí: el cableado real pertenece a la tarea
//! 10-c de la etapa A-6 (HEX-074-c).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use hexcell_admin::argumentos;
use hexcell_admin::ciclo_de_vida::{DatosDeSondeo, TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S};
use hexcell_admin::comandos;
use hexcell_admin::docker::ClienteDocker;
use hexcell_admin::salida::Salida;

fn main() -> ExitCode {
    let argumentos_del_proceso: Vec<String> = std::env::args().collect();
    let resto = if argumentos_del_proceso.is_empty() {
        &[][..]
    } else {
        &argumentos_del_proceso[1..]
    };
    let resultado = argumentos::analizar(resto);
    let mut salida = Salida::estandar();
    let necesita_docker = matches!(&resultado, Ok(invocacion) if !invocacion.simular());
    let codigo = if necesita_docker {
        let cliente = ClienteDocker::con_tiempo_limite(
            PathBuf::from("/var/run/docker.sock"),
            Duration::from_secs(TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S),
        );
        comandos::ejecutar_con_efectos(resultado, &mut salida, &cliente, DatosDeSondeo::default())
    } else {
        comandos::ejecutar(resultado, &mut salida)
    };
    ExitCode::from(codigo)
}
