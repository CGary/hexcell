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

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use crate::almacen_plano_de_control::{AlmacenDelPlanoDeControl, MOTIVO_DE_ALTA_IMPLICITA};
use crate::argumentos::{Comando, ErrorDeArgumentos, Invocacion, Subcomando, TEXTO_DE_USO};
use crate::ciclo_de_vida::{
    self, DatosDeSondeo, Disponibilidad, LIMITE_DE_SONDEO_DE_ESTADO_S, NombresDeCelula,
};
use crate::codigo_de_salida::CodigoDeSalida;
use crate::docker::{ClienteDocker, ErrorDeClienteDocker, InventarioDocker};
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

/// Ejecuta los subcomandos que ya tienen efectos Docker.
pub fn ejecutar_con_efectos<S: Write, D: Write>(
    resultado: Result<Comando, ErrorDeArgumentos>,
    salida: &mut Salida<S, D>,
    cliente: &ClienteDocker,
    inventario: &InventarioDocker,
    ruta_almacen: &str,
    ahora_ms: i64,
    datos: ciclo_de_vida::DatosDeSondeo,
) -> CodigoDeSalida {
    let comando = match resultado {
        Ok(comando) => comando,
        Err(error) => return ejecutar(Err(error), salida),
    };
    if comando.simular() {
        return ejecutar(Ok(comando), salida);
    }
    // El grupo `config render` no toca Docker: sus únicos efectos son de sistema de archivos y
    // viven en `ejecutar_renderizado`. Se delega sin construir ni consumir el `ClienteDocker`.
    let invocacion = match comando {
        Comando::Cell(invocacion) => invocacion,
        otro @ Comando::ConfigRender(_) => return ejecutar(Ok(otro), salida),
    };
    // El despacho por subcomando va ANTES de exigir `--id`: `cell list` nunca lo admite, y si el
    // `id` se exigiera primero, `cell list` sin `--simular` devolvería `Fallo` en vez de
    // `NoImplementadoTodavia`, rompiendo AC-6 para el único subcomando sin identificador.
    match invocacion.subcomando() {
        Subcomando::Retirar | Subcomando::Reemparejar => {
            return ejecutar(Ok(Comando::Cell(invocacion)), salida);
        }
        Subcomando::Listar => {
            return ejecutar_listado(salida, inventario, ruta_almacen);
        }
        Subcomando::Estado => {
            let id = match invocacion.id() {
                Some(id) => id,
                None => return CodigoDeSalida::Fallo,
            };
            return ejecutar_estado(salida, cliente, inventario, ruta_almacen, id, &datos);
        }
        Subcomando::Pausar | Subcomando::Reanudar => {}
    }
    let id = match invocacion.id() {
        Some(id) => id,
        None => return CodigoDeSalida::Fallo,
    };
    let estado_objetivo = if invocacion.subcomando() == Subcomando::Pausar {
        EstadoDeCelula::Suspendida
    } else {
        EstadoDeCelula::EnEjecucion
    };
    // Abrir el almacén y validar la transición ANTES de cualquier petición Docker.
    let almacen = match AlmacenDelPlanoDeControl::abrir(Path::new(ruta_almacen)) {
        Ok(a) => a,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    let fila_existente = match almacen.leer_estado(id) {
        Ok(fila) => fila,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    if let Some(fila) = &fila_existente
        && let Err(transicion) = fila.estado.transitar(estado_objetivo)
    {
        return diagnosticar_fallo(salida, &transicion.to_string());
    }
    // Sólo ahora se emiten las peticiones Docker.
    let nombres = NombresDeCelula::nueva(id);
    let resultado_docker = if invocacion.subcomando() == Subcomando::Pausar {
        ciclo_de_vida::pausar(cliente, &nombres)
    } else {
        ciclo_de_vida::reanudar(cliente, &nombres, &datos)
    };
    match resultado_docker {
        Ok(()) => {
            // La transición se persiste sólo después de que Docker tuvo éxito.
            let motivo = if fila_existente.is_some() {
                ""
            } else {
                MOTIVO_DE_ALTA_IMPLICITA
            };
            if let Err(error) = almacen.registrar_transicion(
                id,
                fila_existente.as_ref().map(|f| f.estado),
                estado_objetivo,
                motivo,
                ahora_ms,
            ) {
                return diagnosticar_fallo(salida, &error.to_string());
            }
            match salida.linea(&format!(
                "cell {} completado para «{id}»",
                invocacion.subcomando().nombre_en_cli()
            )) {
                Ok(()) => CodigoDeSalida::Exito,
                Err(_) => CodigoDeSalida::Fallo,
            }
        }
        Err(error) => diagnosticar_fallo(salida, &error.to_string()),
    }
}

/// Cruza las tres fuentes (almacén, Docker, sonda de salud) para reportar el estado de una
/// célula y las discrepancias detectadas.
///
/// **Sólo lectura por construcción:** el almacén se abre con
/// [`AlmacenDelPlanoDeControl::abrir_solo_lectura`], que ni crea el archivo ni migra, así que un
/// `HEXCELL_ADMIN_ALMACEN` apuntando a una ruta nueva devuelve `Fallo` con diagnóstico en vez de
/// dejar una base recién creada detrás de una consulta.
///
/// **Fuente que falla frente a discrepancia:** una discrepancia es un desacuerdo entre fuentes
/// que todas respondieron; un `inspect` que falla con algo distinto de `NoEncontrado` significa
/// que Docker NO respondió, y entonces no se sabe si los contenedores existen. Reportar DISC-04
/// o DISC-05 ahí sería afirmar como observado lo que no se observó, así que se emite un
/// diagnóstico que nombra la fuente y se devuelve `Fallo` sin inventar ningún código.
///
/// **Peticiones Docker:** con ambos contenedores corriendo se emiten SIETE —los dos `inspect` de
/// este comando más las cinco de [`ciclo_de_vida::sondear_disponibilidad`], que reinspecciona el
/// núcleo para leer su red y su puerto—; con cualquiera detenido o ausente, la sonda no se lanza
/// y sólo se emiten los dos `inspect`.
fn ejecutar_estado<S: Write, D: Write>(
    salida: &mut Salida<S, D>,
    cliente: &ClienteDocker,
    _inventario: &InventarioDocker,
    ruta_almacen: &str,
    id: &str,
    datos: &DatosDeSondeo,
) -> CodigoDeSalida {
    let almacen = match AlmacenDelPlanoDeControl::abrir_solo_lectura(Path::new(ruta_almacen)) {
        Ok(a) => a,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    let fila = match almacen.leer_estado(id) {
        Ok(fila) => fila,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    let nombres = NombresDeCelula::nueva(id);
    let inspeccion_nucleo = cliente.inspeccionar_contenedor(&nombres.nucleo);
    let inspeccion_sidecar = cliente.inspeccionar_contenedor(&nombres.sidecar);
    // Una fuente que falla NO es una discrepancia: se nombra y se aborta. Sólo `NoEncontrado`
    // es una observación («el contenedor no existe»); todo lo demás es «Docker no contestó».
    for (nombre, inspeccion) in [
        (&nombres.nucleo, &inspeccion_nucleo),
        (&nombres.sidecar, &inspeccion_sidecar),
    ] {
        if let Err(error) = inspeccion
            && !matches!(error, ErrorDeClienteDocker::NoEncontrado)
        {
            return diagnosticar_fallo(
                salida,
                &format!("{TEXTO_DE_FUENTE_DOCKER_FALLIDA} «{nombre}»: {error}"),
            );
        }
    }
    // Pasada la guarda, cada inspección es un éxito o un `NoEncontrado`: la ausencia de estado
    // significa contenedor ausente y nada más.
    let estado_nucleo = estado_de_inspeccion(&inspeccion_nucleo);
    let estado_sidecar = estado_de_inspeccion(&inspeccion_sidecar);
    let datos_de_sondeo = DatosDeSondeo {
        imagen: datos.imagen.clone(),
        limite_segundos: LIMITE_DE_SONDEO_DE_ESTADO_S,
    };
    let disponibilidad = if estado_nucleo.as_deref() == Some("running")
        && estado_sidecar.as_deref() == Some("running")
    {
        ciclo_de_vida::sondear_disponibilidad(cliente, &nombres, &datos_de_sondeo)
    } else {
        Disponibilidad::Inalcanzable
    };
    let sustituciones = match almacen.leer_sustituciones(id) {
        Ok(s) => s,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    // Reportar los cinco campos principales.
    let estado_almacen = fila
        .as_ref()
        .map(|f| crate::almacen_plano_de_control::etiqueta_persistida(f.estado))
        .unwrap_or("sin_fila");
    let _ = salida.linea(&format!("estado: {estado_almacen}"));
    let _ = salida.linea(&format!(
        "docker nucleo: {}",
        estado_nucleo.as_deref().unwrap_or("ausente")
    ));
    let _ = salida.linea(&format!(
        "docker sidecar: {}",
        estado_sidecar.as_deref().unwrap_or("ausente")
    ));
    let _ = salida.linea(&format!(
        "salud: {}",
        etiqueta_disponibilidad(disponibilidad)
    ));
    if sustituciones.is_empty() {
        let _ = salida.linea("sustituciones: (ninguna)");
    } else {
        let _ = salida.linea("sustituciones:");
        for s in &sustituciones {
            let _ = salida.linea(&format!("  - {} ({} ms)", s.motivo, s.registrado_ms));
        }
    }
    // Calcular y reportar discrepancias.
    let mut discrepancias = Vec::new();
    let nucleo_corriendo = estado_nucleo.as_deref() == Some("running");
    let sidecar_corriendo = estado_sidecar.as_deref() == Some("running");
    // Los dos indicadores son complementarios exactos y sus nombres dicen lo que calculan:
    // DISC-04 exige que Docker no conozca NINGUNO de los dos contenedores, y DISC-05 que
    // conozca AL MENOS uno. Un par a medias sin fila en el almacén sigue siendo DISC-05.
    let ambos_ausentes = estado_nucleo.is_none() && estado_sidecar.is_none();
    let alguno_presente = !ambos_ausentes;
    // DISC-01: almacén dice en_ejecucion pero algún contenedor no está corriendo.
    if let Some(fila) = &fila
        && fila.estado == EstadoDeCelula::EnEjecucion
        && (!nucleo_corriendo || !sidecar_corriendo)
    {
        discrepancias
            .push("DISC-01: el almacén indica en_ejecucion pero un contenedor no está corriendo");
    }
    // DISC-02: almacén dice suspendida pero algún contenedor está corriendo.
    if let Some(fila) = &fila
        && fila.estado == EstadoDeCelula::Suspendida
        && (nucleo_corriendo || sidecar_corriendo)
    {
        discrepancias
            .push("DISC-02: el almacén indica suspendida pero un contenedor está corriendo");
    }
    // DISC-03: ambos contenedores corriendo pero la salud no está lista.
    if nucleo_corriendo && sidecar_corriendo && disponibilidad != Disponibilidad::Listo {
        discrepancias
            .push("DISC-03: los contenedores corren pero /health/ready no confirma disponibilidad");
    }
    // DISC-04: hay fila en el almacén pero los contenedores no existen en Docker.
    if fila.is_some() && ambos_ausentes {
        discrepancias
            .push("DISC-04: el almacén tiene una fila pero los contenedores no existen en Docker");
    }
    // DISC-05: los contenedores existen en Docker pero no hay fila en el almacén.
    if fila.is_none() && alguno_presente {
        discrepancias
            .push("DISC-05: los contenedores existen en Docker pero el almacén no tiene fila");
    }
    if discrepancias.is_empty() {
        CodigoDeSalida::Exito
    } else {
        for d in &discrepancias {
            let _ = salida.diagnostico(d);
        }
        CodigoDeSalida::Fallo
    }
}

/// Lista las células conocidas: la unión de las filas del almacén y los pares de contenedores
/// de Docker.
fn ejecutar_listado<S: Write, D: Write>(
    salida: &mut Salida<S, D>,
    inventario: &InventarioDocker,
    ruta_almacen: &str,
) -> CodigoDeSalida {
    // Sólo lectura por construcción, igual que `ejecutar_estado`: ver su documentación.
    let almacen = match AlmacenDelPlanoDeControl::abrir_solo_lectura(Path::new(ruta_almacen)) {
        Ok(a) => a,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    let filas = match almacen.listar_celulas() {
        Ok(f) => f,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    let contenedores = match inventario.listar_contenedores() {
        Ok(c) => c,
        Err(error) => {
            return diagnosticar_fallo(salida, &error.to_string());
        }
    };
    // Construir el conjunto de ids conocidos desde Docker.
    let mut ids_docker: BTreeSet<String> = BTreeSet::new();
    let mut estado_nucleo: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut estado_sidecar: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for c in &contenedores {
        if let Some(id) = c.nombre.strip_suffix("-nucleo") {
            ids_docker.insert(id.to_string());
            estado_nucleo.insert(id.to_string(), c.estado.clone());
        } else if let Some(id) = c.nombre.strip_suffix("-sidecar") {
            ids_docker.insert(id.to_string());
            estado_sidecar.insert(id.to_string(), c.estado.clone());
        }
    }
    // Unión de ids del almacén y de Docker.
    let mut todos_ids: BTreeSet<String> = BTreeSet::new();
    for f in &filas {
        todos_ids.insert(f.id.clone());
    }
    for id in &ids_docker {
        todos_ids.insert(id.clone());
    }
    // Reportar cada célula.
    for id in &todos_ids {
        let fila = filas.iter().find(|f| f.id == *id);
        let estado_almacen = fila
            .map(|f| crate::almacen_plano_de_control::etiqueta_persistida(f.estado))
            .unwrap_or("sin_fila");
        let est_nucleo = estado_nucleo
            .get(id)
            .map(|s| s.as_str())
            .unwrap_or("ausente");
        let est_sidecar = estado_sidecar
            .get(id)
            .map(|s| s.as_str())
            .unwrap_or("ausente");
        let _ = salida.linea(&format!(
            "{id} estado={estado_almacen} nucleo={est_nucleo} sidecar={est_sidecar}"
        ));
    }
    CodigoDeSalida::Exito
}

/// Prefijo del diagnóstico que nombra la fuente Docker cuando es ella la que falla.
///
/// Constante para que la prueba de fuente fallida pueda exigir el texto sin copiarlo, y para
/// que quede a la vista que ningún código `DISC-0N` aparece en ese camino.
pub const TEXTO_DE_FUENTE_DOCKER_FALLIDA: &str = "la fuente Docker falló al inspeccionar";

/// Extrae el estado de un contenedor de su inspección, o `None` si el contenedor no existe.
///
/// No existe ningún centinela de error: la única forma de `Err` que llega aquí es
/// `NoEncontrado`, porque `ejecutar_estado` aborta antes con un diagnóstico para cualquier
/// otra. Devolver un `Some("error")` haría que un fallo de transporte se leyera como un estado
/// de contenedor observado, que es justamente lo que produce un DISC-04 o un DISC-05 falso.
fn estado_de_inspeccion(
    resultado: &Result<serde_json::Value, ErrorDeClienteDocker>,
) -> Option<String> {
    match resultado {
        Ok(valor) => valor
            .pointer("/State/Status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase()),
        Err(_) => None,
    }
}

/// Traduce la disponibilidad a la etiqueta que se muestra en `cell status`.
fn etiqueta_disponibilidad(disponibilidad: Disponibilidad) -> &'static str {
    match disponibilidad {
        Disponibilidad::Listo => "listo",
        Disponibilidad::NoListo => "no_listo",
        Disponibilidad::Inalcanzable => "inalcanzable",
    }
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
