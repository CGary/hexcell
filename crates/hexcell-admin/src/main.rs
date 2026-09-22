//! Binario de la CLI central de administración.
//!
//! Raíz de composición de `hexcell-admin`: recoge los argumentos del proceso, los entrega al
//! analizador de [`hexcell_admin::argumentos`], construye el sumidero de salida de producción
//! de [`hexcell_admin::salida`] y los despacha a [`hexcell_admin::comandos::ejecutar`], que
//! devuelve el [`hexcell_admin::codigo_de_salida::CodigoDeSalida`] que el proceso devuelve al
//! sistema operativo a través de `std::process::ExitCode`. Cuando la invocación no es una
//! simulación construye además el [`hexcell_admin::docker::ClienteDocker`], el
//! [`hexcell_admin::docker::InventarioDocker`] y el almacén del plano de control, y despacha
//! por `comandos::ejecutar_con_efectos`.
//!
//! Este archivo no contiene lógica de análisis, ningún `match` sobre subcomandos y ningún
//! texto de mensaje propio: toda cadena y toda regla de despacho vive en los módulos de la
//! biblioteca, donde las pruebas externas de `crates/hexcell-admin/tests/` pueden ejercitarla.
//! El esqueleto de la etapa A-1 (`println!` de talón) desaparece aquí: el cableado real
//! pertenece a la tarea 10-c de la etapa A-6 (HEX-074-c).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hexcell_admin::almacen_plano_de_control::{
    RUTA_POR_OMISION_DEL_ALMACEN, VARIABLE_DE_RUTA_DEL_ALMACEN,
};
use hexcell_admin::argumentos;
use hexcell_admin::ciclo_de_vida::{DatosDeSondeo, TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S};
use hexcell_admin::comandos;
use hexcell_admin::docker::{ClienteDocker, InventarioDocker};
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
        let tiempo_limite = Duration::from_secs(TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S);
        let cliente =
            ClienteDocker::con_tiempo_limite(PathBuf::from("/var/run/docker.sock"), tiempo_limite);
        let inventario =
            InventarioDocker::nuevo(PathBuf::from("/var/run/docker.sock"), tiempo_limite);
        let ruta_almacen = std::env::var(VARIABLE_DE_RUTA_DEL_ALMACEN)
            .unwrap_or_else(|_| RUTA_POR_OMISION_DEL_ALMACEN.to_string());
        let ahora_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        comandos::ejecutar_con_efectos(
            resultado,
            &mut salida,
            &cliente,
            &inventario,
            &ruta_almacen,
            ahora_ms,
            DatosDeSondeo::default(),
        )
    } else {
        comandos::ejecutar(resultado, &mut salida)
    };
    ExitCode::from(codigo)
}
