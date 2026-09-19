//! Servicio de aplicación de la CLI `hexcell-admin`: despacho de subcomandos y modo de
//! simulación. Tercera de tres hijas de la tarea 10 de la etapa A-6. Consume el resultado
//! del análisis de [`crate::argumentos::analizar`] y los dos sumideros tipados de
//! [`crate::salida::Salida`], y devuelve un
//! [`crate::codigo_de_salida::CodigoDeSalida`] sin tocar ningún socket, ningún archivo y
//! sin mutar ningún estado: el comportamiento real de los seis subcomandos pertenece a
//! las tareas 11 a 15 del plan de la etapa A-6.
//!
//! La función [`ejecutar`] es genérica sobre los dos escritores de
//! [`crate::salida::Salida`], de modo que una prueba puede inyectar dos búferes en
//! memoria y asertar tanto el código de salida como los bytes exactos que caen en cada
//! sumidero. En producción, `src/main.rs` construye el `Salida` sobre `stdout` y `stderr`
//! reales a través de `Salida::estandar()`.

use std::io::Write;

use crate::argumentos::{Comando, ErrorDeArgumentos, Invocacion, Subcomando, TEXTO_DE_USO};
use crate::codigo_de_salida::CodigoDeSalida;
use crate::estado_de_celula::EstadoDeCelula;
use crate::salida::Salida;

/// Despacha el resultado del análisis de argumentos contra los dos sumideros de salida y
/// devuelve el código de salida del proceso.
///
/// Tabla de desenlaces, cerrada aquí y en `adr-0036`: error de análisis → `UsoIncorrecto`
/// (2) con el mensaje y el texto de uso por diagnóstico; válido con `--simular` → `Exito`
/// (0) con la línea de simulación por estándar, sin abrir ningún socket ni mutar estado;
/// válido sin `--simular` → `NoImplementadoTodavia` (3) con aviso por diagnóstico;
/// cualquier `io::Error` de los sumideros → `Fallo` (1).
///
/// `ejecutar` no recibe un `ClienteDocker`, ni una ruta de sistema de archivos, ni un
/// reloj y ninguna asa de red: esa es la propiedad que hace que «no hay efecto lateral»
/// sea una consecuencia de la firma y no del resultado de una revisión de código.
pub fn ejecutar<S: Write, D: Write>(
    resultado: Result<Comando, ErrorDeArgumentos>,
    salida: &mut Salida<S, D>,
) -> CodigoDeSalida {
    let comando = match resultado {
        Ok(comando) => comando,
        Err(error) => {
            if salida.diagnostico(&format!("{error}")).is_err() {
                return CodigoDeSalida::Fallo;
            }
            if salida.diagnostico(TEXTO_DE_USO).is_err() {
                return CodigoDeSalida::Fallo;
            }
            return CodigoDeSalida::UsoIncorrecto;
        }
    };

    if let Comando::ConfigRender(invocacion) = comando {
        return ejecutar_renderizado(invocacion, salida);
    }
    let invocacion = match comando {
        Comando::Cell(invocacion) => invocacion,
        Comando::ConfigRender(_) => unreachable!(),
    };
    if invocacion.simular() {
        let linea = linea_de_simulacion(&invocacion);
        match salida.linea(&linea) {
            Ok(()) => CodigoDeSalida::Exito,
            Err(_) => CodigoDeSalida::Fallo,
        }
    } else {
        match salida.diagnostico(&aviso_no_implementado(invocacion.subcomando())) {
            Ok(()) => CodigoDeSalida::NoImplementadoTodavia,
            Err(_) => CodigoDeSalida::Fallo,
        }
    }
}

fn ejecutar_renderizado<S: Write, D: Write>(
    invocacion: crate::argumentos::InvocacionRenderizado,
    salida: &mut Salida<S, D>,
) -> CodigoDeSalida {
    let defecto = match std::fs::read_to_string(invocacion.defecto()) {
        Ok(texto) => texto,
        Err(error) => {
            return diagnosticar_fallo(
                salida,
                &format!("no se pudo leer el archivo de defecto: {error}"),
            );
        }
    };
    let superposicion = match std::fs::read_to_string(invocacion.superposicion()) {
        Ok(texto) => texto,
        Err(error) => {
            return diagnosticar_fallo(
                salida,
                &format!("no se pudo leer el archivo de superposición: {error}"),
            );
        }
    };
    let defecto = match crate::renderizado_configuracion::analizar_env(&defecto) {
        Ok(v) => v,
        Err(e) => return diagnosticar_fallo(salida, &e.to_string()),
    };
    let superposicion = match crate::renderizado_configuracion::analizar_env(&superposicion) {
        Ok(v) => v,
        Err(e) => return diagnosticar_fallo(salida, &e.to_string()),
    };
    let combinado = match crate::renderizado_configuracion::combinar(defecto, superposicion) {
        Ok(v) => v,
        Err(e) => return diagnosticar_fallo(salida, &e.to_string()),
    };
    if invocacion.simular() {
        return match salida.linea(&format!(
            "simulación: configuración renderizada con {} claves",
            combinado.len()
        )) {
            Ok(()) => CodigoDeSalida::Exito,
            Err(_) => CodigoDeSalida::Fallo,
        };
    }
    match std::fs::write(
        invocacion.salida(),
        crate::renderizado_configuracion::serializar(&combinado),
    ) {
        Ok(()) => CodigoDeSalida::Exito,
        Err(error) => diagnosticar_fallo(
            salida,
            &format!("no se pudo escribir la configuración renderizada: {error}"),
        ),
    }
}

fn diagnosticar_fallo<S: Write, D: Write>(
    salida: &mut Salida<S, D>,
    mensaje: &str,
) -> CodigoDeSalida {
    let _ = salida.diagnostico(mensaje);
    CodigoDeSalida::Fallo
}

/// Línea en español que describe la acción planificada de una invocación en modo de
/// simulación. Para los cuatro subcomandos que modifican el estado de la célula la línea
/// nombra el estado objetivo a través del `Display` de [`EstadoDeCelula`]; para los dos
/// subcomandos de sólo lectura se limita a nombrar la acción. La taxonomía de estados de
/// sesión del sidecar descrita en `docs/protocolo-ipc-nucleo-sidecar.md` es ajena al plano
/// de control: sus causas de desvinculación no son estados de célula y este crate no las
/// nombra.
fn linea_de_simulacion(invocacion: &Invocacion) -> String {
    let id = invocacion.id().unwrap_or("(sin id)");
    match invocacion.subcomando() {
        Subcomando::Pausar => {
            format!(
                "simulación: cell pause --id {id} -> estado objetivo: {}",
                EstadoDeCelula::Suspendida
            )
        }
        Subcomando::Reanudar => {
            format!(
                "simulación: cell unpause --id {id} -> estado objetivo: {}",
                EstadoDeCelula::EnEjecucion
            )
        }
        Subcomando::Retirar => {
            format!(
                "simulación: cell terminate --id {id} -> estado objetivo: {}",
                EstadoDeCelula::Retirada
            )
        }
        Subcomando::Reemparejar => {
            let motivo = invocacion.motivo().unwrap_or("(sin motivo)");
            format!(
                "simulación: cell rebind --id {id} --motivo \"{motivo}\" -> estado objetivo: {}",
                EstadoDeCelula::Reemparejando
            )
        }
        Subcomando::Listar => "simulación: cell list".to_string(),
        Subcomando::Estado => format!("simulación: cell status --id {id}"),
    }
}

/// Aviso en español que se emite cuando un subcomando válido se invoca sin `--simular`:
/// nombra el subcomando y la tarea del plan de la etapa A-6 a la que pertenece su
/// implementación real.
fn aviso_no_implementado(subcomando: Subcomando) -> String {
    format!(
        "subcomando «{}» todavía no implementado (tareas 11 a 15 de la etapa A-6)",
        subcomando.nombre_en_cli()
    )
}

/// Estado objetivo en el plano de control al que apunta cada subcomando, o `None` para
/// los subcomandos de sólo lectura.
///
/// Función total y pura con coincidencia exhaustiva de seis brazos y ningún brazo por
/// defecto: añadir o quitar una variante de [`Subcomando`] sin extender esta función deja
/// de compilar. No construye ningún `CicloDeVidaDeCelula` ni aplica ninguna transición:
/// el estado actual de la célula es incognoscible sin el almacén de estado del plano de
/// control, diferido a otra tarea de A-6, así que esta función sólo nombra el destino.
pub fn estado_objetivo(subcomando: Subcomando) -> Option<EstadoDeCelula> {
    match subcomando {
        Subcomando::Pausar => Some(EstadoDeCelula::Suspendida),
        Subcomando::Reanudar => Some(EstadoDeCelula::EnEjecucion),
        Subcomando::Retirar => Some(EstadoDeCelula::Retirada),
        Subcomando::Reemparejar => Some(EstadoDeCelula::Reemparejando),
        Subcomando::Listar => None,
        Subcomando::Estado => None,
    }
}
