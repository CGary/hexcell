//! Operaciones Docker del ciclo de vida de una célula.
//!
//! Además de `pausar`, `reanudar` y `retirar` (tareas 11 y 12), este módulo aloja desde HEX-085-b
//! los servicios de la secuencia de `cell rebind` (tarea 13 de A-6): los tipos de valor que
//! representan las respuestas de las rutas administrativas del núcleo, el guión de petición HTTP
//! (`wget` de disparo único), la consulta por contenedor hermano (crear, iniciar, esperar, leer
//! registros, eliminar siempre) y los servicios de fase que la tarea 15 de `comandos.rs` compone.

use std::fmt;

use crate::docker::{
    ClienteDocker, ErrorDeClienteDocker, OpcionesDeContenedor, ResultadoDeArranque,
};

/// Ruta de montaje del volumen de datos de la célula, hardcoded en un único lugar.
///
/// El nombre del volumen se lee de `Mounts[].Name` de la inspección del núcleo, pero el punto de
/// montaje es una constante: la plantilla de célula lo fija en la ruta de datos y ninguna otra
/// ruta cuenta como volumen de datos.
const RUTA_DE_DATOS_DE_CELULA: &str = "/var/lib/hexcell";

/// Intervalo entre intentos de la sonda, en milisegundos.
pub const CADENCIA_DE_SONDEO_MS: u64 = 100;
/// Tiempo máximo que se concede a la sonda.
pub const LIMITE_DE_SONDEO_S: u64 = 60;
/// Tiempo máximo de la sonda corta que usa `cell status`: distinto de [`LIMITE_DE_SONDEO_S`]
/// para que una consulta de estado sobre una célula detenida no bloquee al operador durante
/// un minuto completo.
pub const LIMITE_DE_SONDEO_DE_ESTADO_S: u64 = 5;
/// Imagen mínima que contiene el intérprete y `wget`.
pub const IMAGEN_DE_SONDA_POR_OMISION: &str = "alpine:3";
/// Holgura que se concede al cliente Docker por encima del límite de la sonda.
pub const HOLGURA_DEL_CLIENTE_DOCKER_S: u64 = 10;
/// Tiempo límite de lectura del cliente Docker que atiende `POST /containers/{id}/wait`.
///
/// Esa llamada bloquea durante TODA la vida de la sonda, así que el límite del cliente tiene que
/// ser estrictamente mayor que el de la sonda: con uno menor la espera abortaría con
/// `TiempoDeEsperaAgotado` antes de conocer el veredicto real y `cell unpause` fallaría por una
/// razón inventada. La aserción de abajo convierte esa relación en una condición de compilación:
/// si alguien invierte el signo de la holgura, el crate deja de compilar.
pub const TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S: u64 =
    LIMITE_DE_SONDEO_S + HOLGURA_DEL_CLIENTE_DOCKER_S;
const _: () = assert!(TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S > LIMITE_DE_SONDEO_S);

/// Límite de segundos que la sonda de emparejamiento se queda esperando la respuesta del núcleo.
///
/// `POST /containers/{id}/wait` bloquea durante TODA la vida de la sonda, así que el `-T` de
/// `wget` debe ser lo bastante alto para que la sonda vea el 200 del núcleo (cuya ruta de
/// emparejamiento espera hasta 30 s) pero lo bastante bajo para que el cliente no se quede
/// bloqueado si el núcleo no responde. Producción: 40 s. Las pruebas inyectan 1 s.
pub const LIMITE_DE_EMPAREJAMIENTO_SONDA_S: u64 = 40;
const _: () = assert!(LIMITE_DE_EMPAREJAMIENTO_SONDA_S > 30);
const _: () = assert!(LIMITE_DE_EMPAREJAMIENTO_SONDA_S < TIEMPO_LIMITE_DEL_CLIENTE_DOCKER_S);

/// Plazos configurables de la secuencia de `cell rebind`, inyectados para que los bucles de
/// reintento sean deterministas en las pruebas (cadencia en milisegundos, no en segundos).
///
/// La cadencia y losintentos sólo se usan para contar iteraciones: los tests los fijan en
/// valores minúsculos (1 ms, 2 intentos) y nunca duermen segundos; producción usa
/// [`Self::por_omision`] (2 s, 30, 30, 120 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlazosDeReemparejamiento {
    /// Intervalo entre reintentos, en milisegundos.
    pub cadencia_ms: u64,
    /// Número máximo de intentos de la sonda de pausa en el arranque posterior al rm.
    pub intentos_de_pausa: u64,
    /// Número máximo de intentos de la sonda de emparejamiento.
    pub intentos_de_emparejamiento: u64,
    /// Tope del presupuesto de confirmación de emparejamiento, en segundos.
    pub tope_de_confirmacion_s: u64,
}

impl PlazosDeReemparejamiento {
    /// Valores por omisión de producción: cadencia de 2 s, pausa y emparejamiento con hasta 30
    /// reintentos cada uno (60 s a 2 s), y tope de confirmación de 120 s.
    pub fn por_omision() -> Self {
        Self {
            cadencia_ms: 2000,
            intentos_de_pausa: 30,
            intentos_de_emparejamiento: 30,
            tope_de_confirmacion_s: 120,
        }
    }
}

/// Nombres Docker derivados de la identidad de la célula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NombresDeCelula {
    /// Nombre del contenedor del núcleo.
    pub nucleo: String,
    /// Nombre del contenedor del sidecar.
    pub sidecar: String,
}

impl NombresDeCelula {
    /// Construye los nombres fijados por la plantilla de célula.
    pub fn nueva(id: &str) -> Self {
        Self {
            nucleo: format!("{id}-nucleo"),
            sidecar: format!("{id}-sidecar"),
        }
    }
}

/// Configuración opcional de la sonda de disponibilidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosDeSondeo {
    /// Imagen que hospedará la sonda hermana.
    pub imagen: String,
    /// Límite de espera expresado en segundos.
    pub limite_segundos: u64,
}

impl Default for DatosDeSondeo {
    fn default() -> Self {
        Self {
            imagen: std::env::var("HEXCELL_IMAGEN_SONDA")
                .unwrap_or_else(|_| IMAGEN_DE_SONDA_POR_OMISION.to_string()),
            limite_segundos: LIMITE_DE_SONDEO_S,
        }
    }
}

/// Fallo de una operación del ciclo de vida.
#[derive(Debug)]
pub enum ErrorDeCicloDeVida {
    /// Fallo devuelto por Docker.
    Docker(ErrorDeClienteDocker),
    /// La inspección no contiene la configuración necesaria.
    Configuracion(String),
    /// La sonda no confirmó disponibilidad a tiempo.
    TiempoDeSondeoAgotado { limite_segundos: u64 },
    /// La imagen auxiliar no está disponible en el demonio.
    ImagenDeSondaNoEncontrada { imagen: String },
    /// La célula no existe: falta el núcleo o el sidecar.
    CelulaNoEncontrada,
    /// La célula está pausada: el núcleo no está en ejecución.
    CelulaPausada,
    /// El cierre de sesión falló: la sonda de cierre salió con código distinto de cero.
    CierreDeSesionFallido { codigo: i64 },
    /// `POST /admin/envio/pausa` respondió `fallido` antes del paso destructivo (tarea 13, D5.3).
    PausaDeEnvioFallida { motivo: String },
    /// `POST /admin/sesion/emparejamiento` respondió `fallido` con un motivo distinto de
    /// `sin_conexion` (tarea 13, D5.8).
    EmparejamientoFallido { motivo: String },
    /// El código de emparejamiento expiró antes de que la sesión llegara a `activa` (tarea 13,
    /// D5.9). La célula queda en `Reemparejando` para reanudar.
    CodigoExpirado,
    /// No se pudo descartar `sqlstore.db` mediante el contenedor hermano (tarea 13, D5.6).
    DescarteDeSqlstoreFallido { motivo: String },
    /// El cuerpo de la sonda hermana no es JSON válido o no tiene los campos esperados.
    CuerpoDeSondaIlegible { motivo: String },
    /// El núcleo de la célula no está en ejecución al iniciar `cell rebind` desde un estado
    /// distinto de `Reemparejando` (no aplicable al resume, que lo gestiona el llamador).
    NucleoNoCorriendo,
}

impl fmt::Display for ErrorDeCicloDeVida {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Docker(error) => write!(f, "fallo de Docker: {error}"),
            Self::Configuracion(motivo) => {
                write!(f, "configuración de la célula inválida: {motivo}")
            }
            Self::TiempoDeSondeoAgotado { limite_segundos } => write!(
                f,
                "la célula no alcanzó /health/ready: se agotó el límite de {limite_segundos} segundos"
            ),
            Self::ImagenDeSondaNoEncontrada { imagen } => write!(
                f,
                "la imagen de sonda «{imagen}» no existe en Docker; hay que traerla antes de reanudar"
            ),
            Self::CelulaNoEncontrada => write!(f, "célula no encontrada"),
            Self::CelulaPausada => write!(
                f,
                "la célula está pausada: ejecute cell unpause antes de cell terminate"
            ),
            Self::CierreDeSesionFallido { codigo } => write!(
                f,
                "el cierre de sesión falló: la sonda de cierre salió con código {codigo}"
            ),
            Self::PausaDeEnvioFallida { motivo } => {
                write!(f, "la pausa de envío falló: {motivo}")
            }
            Self::EmparejamientoFallido { motivo } => {
                write!(f, "el emparejamiento falló: {motivo}")
            }
            Self::CodigoExpirado => {
                write!(f, "código expirado; repita cell rebind")
            }
            Self::DescarteDeSqlstoreFallido { motivo } => {
                write!(f, "no se pudo descartar sqlstore.db: {motivo}")
            }
            Self::CuerpoDeSondaIlegible { motivo } => {
                write!(f, "no se pudo leer la respuesta de la sonda: {motivo}")
            }
            Self::NucleoNoCorriendo => {
                write!(
                    f,
                    "el núcleo de la célula no está en ejecución; «cell rebind» exige un núcleo activo"
                )
            }
        }
    }
}

impl std::error::Error for ErrorDeCicloDeVida {}

impl From<ErrorDeClienteDocker> for ErrorDeCicloDeVida {
    fn from(error: ErrorDeClienteDocker) -> Self {
        Self::Docker(error)
    }
}

/// Detiene primero el sidecar y después el núcleo, sin fijar el plazo desde la CLI.
///
/// Ninguna de las dos paradas envía el parámetro `t`: el plazo de gracia lo fija el
/// `stop_grace_period` de la plantilla de célula, que queda como única fuente de verdad. El
/// sidecar se detiene CON gracia igual que el núcleo, porque tiene que cerrar su websocket
/// saliente y dejar su almacén consistente. El invariante de que nada sale durante la pausa lo
/// sostiene el ORDEN, no ninguna bandera de estado: sin sidecar no queda canal por el que el
/// núcleo pueda enviar mientras drena.
pub fn pausar(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
) -> Result<(), ErrorDeCicloDeVida> {
    detener_ambos_sin_plazo(cliente, nombres)
}

/// Detiene primero el sidecar y después el núcleo, sin fijar el plazo desde la CLI.
///
/// Función compartida entre `pausar` y `retirar`: el orden (sidecar primero, núcleo después) y la
/// ausencia del parámetro `t` son invariantes de ambas operaciones. El plazo de gracia lo fija el
/// `stop_grace_period` de la plantilla de célula, única fuente de verdad.
fn detener_ambos_sin_plazo(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
) -> Result<(), ErrorDeCicloDeVida> {
    cliente.detener_contenedor_sin_plazo(&nombres.sidecar)?;
    cliente.detener_contenedor_sin_plazo(&nombres.nucleo)?;
    Ok(())
}

/// Lee la red del núcleo desde su inspección: la primera clave de `NetworkSettings.Networks`.
fn red_de_inspeccion(inspeccion: &serde_json::Value) -> Result<String, ErrorDeCicloDeVida> {
    inspeccion
        .pointer("/NetworkSettings/Networks")
        .and_then(serde_json::Value::as_object)
        .and_then(|redes| redes.keys().next())
        .cloned()
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion("el núcleo no declara una red".to_string())
        })
}

/// Lee el puerto de una variable de entorno del núcleo, parametrizado por el nombre de la variable.
///
/// La variable se busca en `Config.Env` y se extrae el puerto tras el último `:`. Hoy se usa para
/// `HEXCELL_DIRECCION_SALUD` (reanudar) y `HEXCELL_DIRECCION_ADMIN` (retirar).
fn puerto_de_inspeccion(
    inspeccion: &serde_json::Value,
    variable: &str,
) -> Result<String, ErrorDeCicloDeVida> {
    let direccion = inspeccion
        .pointer("/Config/Env")
        .and_then(serde_json::Value::as_array)
        .and_then(|variables| {
            variables.iter().find_map(|variable_valor| {
                let prefijo = format!("{variable}=");
                variable_valor
                    .as_str()?
                    .strip_prefix(&prefijo)
                    .map(str::to_string)
            })
        })
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion(format!("falta {variable} en el núcleo"))
        })?;
    direccion
        .rsplit_once(':')
        .map(|(_, puerto)| puerto.to_string())
        .filter(|puerto| !puerto.is_empty())
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion(format!("{variable} no contiene un puerto"))
        })
}

/// Lee el nombre del volumen de datos del núcleo desde su inspección.
///
/// Busca en `Mounts[]` la entrada cuyo `Destination` es la ruta de datos de la célula y devuelve
/// su `Name`. El nombre NUNCA se deriva del `--id` ni del identificador del contenedor: viene de
/// la propia inspección de Docker.
fn volumen_de_inspeccion(inspeccion: &serde_json::Value) -> Result<String, ErrorDeCicloDeVida> {
    inspeccion
        .pointer("/Mounts")
        .and_then(serde_json::Value::as_array)
        .and_then(|montajes| {
            montajes.iter().find_map(|montaje| {
                let destino = montaje.get("Destination")?.as_str()?;
                if destino == RUTA_DE_DATOS_DE_CELULA {
                    montaje.get("Name")?.as_str().map(str::to_string)
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion(format!(
                "el núcleo no monta un volumen en {RUTA_DE_DATOS_DE_CELULA}"
            ))
        })
}

/// Arranca la célula y espera la disponibilidad mediante un contenedor hermano.
pub fn reanudar(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeSondeo,
) -> Result<(), ErrorDeCicloDeVida> {
    cliente.iniciar_contenedor(&nombres.nucleo)?;
    cliente.iniciar_contenedor(&nombres.sidecar)?;

    let inspeccion = cliente.inspeccionar_contenedor(&nombres.nucleo)?;
    let red = red_de_inspeccion(&inspeccion)?;
    let puerto = puerto_de_inspeccion(&inspeccion, "HEXCELL_DIRECCION_SALUD")?;
    let url = format!("http://{}:{}/health/ready", nombres.nucleo, puerto);
    let opciones = OpcionesDeContenedor {
        red,
        cmd: guion_de_sonda_con_limite(&url, datos.limite_segundos),
    };
    let sonda = match cliente.crear_e_iniciar_contenedor_con_opciones(&datos.imagen, opciones) {
        Ok(resultado) => match resultado {
            crate::docker::ResultadoDeArranque::Iniciado { id_contenedor }
            | crate::docker::ResultadoDeArranque::YaEnEjecucion { id_contenedor } => id_contenedor,
        },
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada {
                imagen: datos.imagen.clone(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let espera = cliente.esperar_contenedor(&sonda);
    let limpieza = cliente.eliminar_contenedor(&sonda);
    let codigo = match (espera, limpieza) {
        (Ok(codigo), Ok(())) => codigo,
        (Err(error), Ok(())) => return Err(error.into()),
        (Ok(_), Err(error)) => return Err(error.into()),
        (Err(error), Err(_)) => return Err(error.into()),
    };
    if codigo == 0 {
        Ok(())
    } else {
        Err(ErrorDeCicloDeVida::TiempoDeSondeoAgotado {
            limite_segundos: datos.limite_segundos,
        })
    }
}

/// Veredicto corto de la sonda de disponibilidad que usa `cell status`.
///
/// Tres variantes sin datos: `cell status` sólo necesita saber si la célula está lista, no
/// lista o es inalcanzable; el detalle del fallo vive en las discrepancias que el comando
/// reporta aparte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Disponibilidad {
    /// La sonda confirmó `/health/ready` con código 0.
    Listo,
    /// La sonda se ejecutó pero agotó su límite sin confirmar disponibilidad.
    NoListo,
    /// La sonda no se pudo crear o ejecutar (demonio inalcanzable, imagen ausente, etc.).
    Inalcanzable,
}

/// Sondéa la disponibilidad de la célula con un límite corto y devuelve el veredicto.
///
/// A diferencia de [`reanudar`], esta función no propaga errores: cualquier fallo de Docker
/// se traduce en [`Disponibilidad::Inalcanzable`], porque `cell status` necesita un veredicto
/// de tres estados, no un `Result`. El límite corto ([`LIMITE_DE_SONDEO_DE_ESTADO_S`]) evita
/// que una consulta sobre una célula detenida bloquee al operador durante un minuto.
pub fn sondear_disponibilidad(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeSondeo,
) -> Disponibilidad {
    let inspeccion = match cliente.inspeccionar_contenedor(&nombres.nucleo) {
        Ok(valor) => valor,
        Err(_) => return Disponibilidad::Inalcanzable,
    };
    let red = match inspeccion
        .pointer("/NetworkSettings/Networks")
        .and_then(serde_json::Value::as_object)
        .and_then(|redes| redes.keys().next())
        .cloned()
    {
        Some(r) => r,
        None => return Disponibilidad::Inalcanzable,
    };
    let direccion = match inspeccion
        .pointer("/Config/Env")
        .and_then(serde_json::Value::as_array)
        .and_then(|variables| {
            variables.iter().find_map(|variable| {
                variable
                    .as_str()?
                    .strip_prefix("HEXCELL_DIRECCION_SALUD=")
                    .map(str::to_string)
            })
        }) {
        Some(d) => d,
        None => return Disponibilidad::Inalcanzable,
    };
    let puerto = match direccion
        .rsplit_once(':')
        .map(|(_, p)| p)
        .filter(|p| !p.is_empty())
    {
        Some(p) => p,
        None => return Disponibilidad::Inalcanzable,
    };
    let url = format!("http://{}:{}/health/ready", nombres.nucleo, puerto);
    let opciones = OpcionesDeContenedor {
        red,
        cmd: guion_de_sonda_con_limite(&url, datos.limite_segundos),
    };
    let sonda = match cliente.crear_e_iniciar_contenedor_con_opciones(&datos.imagen, opciones) {
        Ok(crate::docker::ResultadoDeArranque::Iniciado { id_contenedor })
        | Ok(crate::docker::ResultadoDeArranque::YaEnEjecucion { id_contenedor }) => id_contenedor,
        Err(_) => return Disponibilidad::Inalcanzable,
    };
    let espera = cliente.esperar_contenedor(&sonda);
    let limpieza = cliente.eliminar_contenedor(&sonda);
    match (espera, limpieza) {
        (Ok(codigo), _) => {
            if codigo == 0 {
                Disponibilidad::Listo
            } else {
                Disponibilidad::NoListo
            }
        }
        (Err(_), _) => Disponibilidad::Inalcanzable,
    }
}

/// Produce el comando que ejecuta la sonda dentro de la red de la célula.
pub fn guion_de_sonda(url: &str) -> Vec<String> {
    guion_de_sonda_con_limite(url, LIMITE_DE_SONDEO_S)
}

/// Produce el comando que ejecuta la sonda con un límite explícito.
///
/// Pública para que `comandos::ejecutar_estado` pueda construir una sonda corta con
/// [`LIMITE_DE_SONDEO_DE_ESTADO_S`] sin duplicar la plantilla.
pub fn guion_de_sonda_con_limite(url: &str, limite_segundos: u64) -> Vec<String> {
    let intentos = limite_segundos.saturating_mul(1000 / CADENCIA_DE_SONDEO_MS);
    let espera = cadencia_en_segundos(CADENCIA_DE_SONDEO_MS);
    let guion = format!(
        "i=0; while [ \"$i\" -lt {intentos} ]; do if wget -q -O /dev/null \"{url}\"; then exit 0; fi; i=$((i+1)); sleep {espera}; done; exit 1"
    );
    vec!["/bin/sh".to_string(), "-c".to_string(), guion]
}

/// Traduce la cadencia en milisegundos al argumento decimal que entiende el `sleep` de BusyBox,
/// que no admite milisegundos.
///
/// Es lo que hace de [`CADENCIA_DE_SONDEO_MS`] la ÚNICA fuente de la cadencia: el número de
/// iteraciones y la espera de cada una salen de la misma constante, de modo que no pueden
/// separarse en silencio. No usa coma flotante y no colapsa dos cadencias distintas en el mismo
/// texto: 100 ms da `0.1` y 200 ms da `0.2`.
fn cadencia_en_segundos(milisegundos: u64) -> String {
    let enteros = milisegundos / 1000;
    let resto = milisegundos % 1000;
    if resto == 0 {
        return enteros.to_string();
    }
    let fraccion = format!("{resto:03}");
    format!("{enteros}.{}", fraccion.trim_end_matches('0'))
}

/// Produce el comando que ejecuta el cierre de sesión dentro de la red de la célula.
///
/// A diferencia de [`guion_de_sonda`], este es un disparo único sin bucle: `wget -q -O - --post-data ''`
/// contra la ruta de cierre. El código de salida de `wget` es el veredicto: 0 si la ruta respondió
/// con 2xx, distinto de cero en caso contrario (502, 504, conexión reseteada, etc.).
pub fn guion_de_cierre_de_sesion(url: &str) -> Vec<String> {
    vec![
        "wget".to_string(),
        "-q".to_string(),
        "-O".to_string(),
        "-".to_string(),
        "--post-data".to_string(),
        "".to_string(),
        url.to_string(),
    ]
}

/// Retira definitivamente una célula: cierra la sesión, detiene ambos contenedores y elimina
/// los contenedores y el volumen de datos.
///
/// Secuencia de seis pasos, abortando al primer fallo:
/// 1. Inspeccionar el núcleo; si no existe, `CelulaNoEncontrada`; si no está en ejecución,
///    `CelulaPausada`.
/// 2. Inspeccionar el sidecar; si no existe, `CelulaNoEncontrada`.
/// 3. Resolver red, puerto de admin y nombre del volumen desde la inspección del núcleo.
/// 4. POST `/admin/sesion/cierre` mediante un contenedor hermano; si falla, abortar sin destruir
///    nada (salvo la propia sonda de cierre).
/// 5. Detener sidecar y núcleo (sin plazo explícito, rige el `stop_grace_period`).
/// 6. Eliminar sidecar, núcleo y volumen; devolver el nombre del volumen eliminado.
pub fn retirar(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeSondeo,
) -> Result<String, ErrorDeCicloDeVida> {
    // Paso 1: inspeccionar el núcleo.
    let inspeccion_nucleo = match cliente.inspeccionar_contenedor(&nombres.nucleo) {
        Ok(inspeccion) => inspeccion,
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::CelulaNoEncontrada);
        }
        Err(error) => return Err(error.into()),
    };

    // Verificar que el núcleo está en ejecución.
    let estado = inspeccion_nucleo
        .pointer("/State/Status")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion("el núcleo no declara su estado".to_string())
        })?;
    if estado != "running" {
        return Err(ErrorDeCicloDeVida::CelulaPausada);
    }

    // Paso 2: inspeccionar el sidecar.
    match cliente.inspeccionar_contenedor(&nombres.sidecar) {
        Ok(_) => {}
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::CelulaNoEncontrada);
        }
        Err(error) => return Err(error.into()),
    }

    // Paso 3: resolver red, puerto y volumen desde la inspección del núcleo.
    let red = red_de_inspeccion(&inspeccion_nucleo)?;
    let puerto = puerto_de_inspeccion(&inspeccion_nucleo, "HEXCELL_DIRECCION_ADMIN")?;
    let volumen = volumen_de_inspeccion(&inspeccion_nucleo)?;

    // Paso 4: POST /admin/sesion/cierre mediante un contenedor hermano.
    let url = format!("http://{}:{}/admin/sesion/cierre", nombres.nucleo, puerto);
    let opciones = OpcionesDeContenedor {
        red,
        cmd: guion_de_cierre_de_sesion(&url),
    };
    let sonda = match cliente.crear_e_iniciar_contenedor_con_opciones(&datos.imagen, opciones) {
        Ok(resultado) => match resultado {
            crate::docker::ResultadoDeArranque::Iniciado { id_contenedor }
            | crate::docker::ResultadoDeArranque::YaEnEjecucion { id_contenedor } => id_contenedor,
        },
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada {
                imagen: datos.imagen.clone(),
            });
        }
        Err(error) => return Err(error.into()),
    };

    // Esperar y limpiar la sonda de cierre en ambos caminos (éxito y fallo).
    let espera = cliente.esperar_contenedor(&sonda);
    let limpieza = cliente.eliminar_contenedor(&sonda);
    let codigo = match (espera, limpieza) {
        (Ok(codigo), Ok(())) => codigo,
        (Err(error), Ok(())) => return Err(error.into()),
        (Ok(_), Err(error)) => return Err(error.into()),
        (Err(error), Err(_)) => return Err(error.into()),
    };

    // Si el cierre de sesión falló, abortar sin destruir nada.
    if codigo != 0 {
        return Err(ErrorDeCicloDeVida::CierreDeSesionFallido { codigo });
    }

    // Paso 5: detener sidecar y núcleo.
    detener_ambos_sin_plazo(cliente, nombres)?;

    // Paso 6: eliminar sidecar, núcleo y volumen.
    cliente.eliminar_contenedor(&nombres.sidecar)?;
    cliente.eliminar_contenedor(&nombres.nucleo)?;
    cliente.eliminar_volumen(&volumen)?;

    Ok(volumen)
}

// ============================================================================
// Tipos de valor y helpers para `cell rebind` (tarea 13 de A-6, HEX-085-b).
// ============================================================================

/// Información resuelta de los contenedores de una célula para las sondas HTTP de `cell rebind`:
/// red, puerto de admin y nombre del volumen de datos, todos leídos de la inspección del núcleo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosDeCelulaParaRebind {
    /// Red Docker a la que se conectan las sondas hermanas.
    pub red: String,
    /// Puerto de la escucha administrativa del núcleo, leído de `HEXCELL_DIRECCION_ADMIN`.
    pub puerto_admin: String,
    /// Nombre del volumen de datos, leído de `Mounts[].Name` del núcleo.
    pub volumen: String,
}

/// Resuelve la red, el puerto de admin y el volumen de datos de una célula inspeccionando el núcleo
/// y verificando que el sidecar existe. Es el helper compartido de los pasos 2-3 de D5.
///
/// Devuelve [`ErrorDeCicloDeVida::CelulaNoEncontrada`] si falta el núcleo o el sidecar, y
/// [`ErrorDeCicloDeVida::NucleoNoCorriendo`] si el núcleo no está en ejecución (el rebind desde
/// `EnEjecución` lo exige; el resume desde `Reemparejando` lo gestiona el llamador).
pub fn resolver_datos_de_celula_para_rebind(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
) -> Result<DatosDeCelulaParaRebind, ErrorDeCicloDeVida> {
    let inspeccion_nucleo = match cliente.inspeccionar_contenedor(&nombres.nucleo) {
        Ok(inspeccion) => inspeccion,
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::CelulaNoEncontrada);
        }
        Err(error) => return Err(error.into()),
    };
    match cliente.inspeccionar_contenedor(&nombres.sidecar) {
        Ok(_) => {}
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::CelulaNoEncontrada);
        }
        Err(error) => return Err(error.into()),
    }
    let red = red_de_inspeccion(&inspeccion_nucleo)?;
    let puerto_admin = puerto_de_inspeccion(&inspeccion_nucleo, "HEXCELL_DIRECCION_ADMIN")?;
    let volumen = volumen_de_inspeccion(&inspeccion_nucleo)?;
    Ok(DatosDeCelulaParaRebind {
        red,
        puerto_admin,
        volumen,
    })
}

/// Comando de disparo único que ejecuta una petición HTTP desde un contenedor hermano.
///
/// A diferencia de [`guion_de_sonda_con_limite`] (bucle con `sleep`), este es un único
/// `wget -q -O - -T <limite>` con `--post-data <cuerpo>` opcional: la sonda se limita a leer una
/// respuesta y devolverla por su salida estándar. El código de salida de `wget` es el veredicto
/// sólo cuando el llamador decide consumirlo (p. ej., la sonda de cierre de sesión); para las
/// sondas que leen cuerpo, el código es 0 si el núcleo respondió 2xx.
///
/// `limite` es el `-T` de `wget` en segundos. Debe estar por encima del plazo de la ruta del
/// núcleo (30 s para emparejamiento) y por debajo del tiempo límite del cliente Docker.
pub fn guion_de_peticion_http(url: &str, cuerpo: Option<&str>, limite: u64) -> Vec<String> {
    let limite = limite.to_string();
    match cuerpo {
        Some(cuerpo) => vec![
            "wget".to_string(),
            "-q".to_string(),
            "-O".to_string(),
            "-".to_string(),
            "-T".to_string(),
            limite,
            "--post-data".to_string(),
            cuerpo.to_string(),
            url.to_string(),
        ],
        None => vec![
            "wget".to_string(),
            "-q".to_string(),
            "-O".to_string(),
            "-".to_string(),
            "-T".to_string(),
            limite,
            url.to_string(),
        ],
    }
}

/// Crea un contenedor hermano, lo inicia, espera su código de salida, lee su salida estándar y lo
/// elimina siempre (también si el arranque o la espera fallan). Devuelve los bytes de salida
/// estándar para que el los analice el llamador.
///
/// La eliminación se ejecuta siempre, también en los caminos de error, para que ninguna sonda
/// quede huérfana en el demonio.
fn consultar_por_hermano(
    cliente: &ClienteDocker,
    _nombres: &NombresDeCelula,
    imagen: &str,
    cmd: Vec<String>,
    red: &str,
) -> Result<Vec<u8>, ErrorDeCicloDeVida> {
    let opciones = OpcionesDeContenedor {
        red: red.to_string(),
        cmd,
    };
    let sonda = match cliente.crear_e_iniciar_contenedor_con_opciones(imagen, opciones) {
        Ok(ResultadoDeArranque::Iniciado { id_contenedor })
        | Ok(ResultadoDeArranque::YaEnEjecucion { id_contenedor }) => id_contenedor,
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada {
                imagen: imagen.to_string(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let espera = cliente.esperar_contenedor(&sonda);
    let lectura = cliente.leer_salida_estandar(&sonda);
    let limpieza = cliente.eliminar_contenedor(&sonda);
    if let Err(error) = limpieza {
        return Err(error.into());
    }
    if let Err(error) = espera {
        return Err(error.into());
    }
    lectura.map_err(ErrorDeCicloDeVida::from)
}

/// Desenlace de `POST /admin/envio/pausa` (D2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DesenlaceDePausa {
    /// `{"resultado":"aplicado","accion":"pausar"|"reanudar"}`.
    Aplicado { accion: String },
    /// `{"resultado":"fallido","accion":"...","motivo":"..."}`.
    Fallido { motivo: String },
    /// `{"resultado":"canal_sin_sesion"}`.
    CanalSinSesion,
}

/// Parsea la respuesta JSON de `POST /admin/envio/pausa` a un [`DesenlaceDePausa`].
fn desenlace_de_pausa(desde: &[u8]) -> Result<DesenlaceDePausa, ErrorDeCicloDeVida> {
    let valor: serde_json::Value =
        serde_json::from_slice(desde).map_err(|_| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de pausa no es JSON válido".to_string(),
        })?;
    let resultado = valor
        .get("resultado")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de pausa no lleva resultado".to_string(),
        })?;
    match resultado {
        "aplicado" => {
            let accion = valor
                .get("accion")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(DesenlaceDePausa::Aplicado { accion })
        }
        "fallido" => {
            let motivo = valor
                .get("motivo")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(DesenlaceDePausa::Fallido { motivo })
        }
        "canal_sin_sesion" => Ok(DesenlaceDePausa::CanalSinSesion),
        otro => Err(ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: format!("resultado de pausa inesperado: {otro}"),
        }),
    }
}

/// Desenlace de `POST /admin/sesion/emparejamiento` (D2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DesenlaceDeEmparejamiento {
    /// `{"resultado":"codigo","metodo":"qr"|"codigo_de_vinculacion","valor":"...","expira_en_ms":N}`.
    Codigo {
        metodo: String,
        valor: String,
        expira_en_ms: i64,
    },
    /// `{"resultado":"fallido","motivo":"sin_conexion"|"ya_emparejada"|...}`.
    Fallido { motivo: String },
    /// `{"resultado":"canal_sin_sesion"}`.
    CanalSinSesion,
}

/// Parsea la respuesta JSON de `POST /admin/sesion/emparejamiento` a un [`DesenlaceDeEmparejamiento`].
fn desenlace_de_emparejamiento(
    desde: &[u8],
) -> Result<DesenlaceDeEmparejamiento, ErrorDeCicloDeVida> {
    let valor: serde_json::Value =
        serde_json::from_slice(desde).map_err(|_| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de emparejamiento no es JSON válido".to_string(),
        })?;
    let resultado = valor
        .get("resultado")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de emparejamiento no lleva resultado".to_string(),
        })?;
    match resultado {
        "codigo" => {
            let metodo = valor
                .get("metodo")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let valor_codigo = valor
                .get("valor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let expira_en_ms = valor
                .get("expira_en_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            Ok(DesenlaceDeEmparejamiento::Codigo {
                metodo,
                valor: valor_codigo,
                expira_en_ms,
            })
        }
        "fallido" => {
            let motivo = valor
                .get("motivo")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(DesenlaceDeEmparejamiento::Fallido { motivo })
        }
        "canal_sin_sesion" => Ok(DesenlaceDeEmparejamiento::CanalSinSesion),
        otro => Err(ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: format!("resultado de emparejamiento inesperado: {otro}"),
        }),
    }
}

/// Estado de sesión devuelto por `GET /admin/sesion` (D2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EstadoDeSesion {
    Activa,
    Reconectando,
    Desvinculada,
    Pausada,
    CanalSinSesion,
}

impl EstadoDeSesion {
    /// ¿Es el estado `activa`?
    pub fn es_activa(self) -> bool {
        matches!(self, Self::Activa)
    }
}

/// Parsea la respuesta JSON de `GET /admin/sesion` a un [`EstadoDeSesion`].
fn estado_de_sesion(desde: &[u8]) -> Result<EstadoDeSesion, ErrorDeCicloDeVida> {
    let valor: serde_json::Value =
        serde_json::from_slice(desde).map_err(|_| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de sesión no es JSON válido".to_string(),
        })?;
    let estado = valor
        .get("estado")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de sesión no lleva estado".to_string(),
        })?;
    match estado {
        "activa" => Ok(EstadoDeSesion::Activa),
        "reconectando" => Ok(EstadoDeSesion::Reconectando),
        "desvinculada" => Ok(EstadoDeSesion::Desvinculada),
        "pausada" => Ok(EstadoDeSesion::Pausada),
        "canal_sin_sesion" => Ok(EstadoDeSesion::CanalSinSesion),
        otro => Err(ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: format!("estado de sesión inesperado: {otro}"),
        }),
    }
}

// ============================================================================
// Servicios de fase de `cell rebind` (tarea 13 de A-6, HEX-085-b).
//
// Cada función corresponde a una fase de la secuencia D5 documentada en el plan. La orquestación
// que las compone en el orden correcto, maneja reanudación desde `Reemparejando` y decide cuándo
// persistir transiciones vive en `comandos::ejecutar_reemparejamiento`.
// ============================================================================

/// Fase «preparar reemparejamiento» (pasos 2-4 de D5): resuelve los datos de la célula, verifica
/// que el núcleo está corriendo, envía `POST /admin/envio/pausa pausar` y, si tiene éxito, intenta
/// el cierre de sesión como mejor esfuerzo.
///
/// Devuelve una advertencia de cierre de sesión (cadena no vacía) cuando el cierre falla, para que
/// el llamador la escriba por diagnóstico sin abortar la secuencia.
///
/// Contrato de fallo:
/// * `fallido` en la pausa → [`ErrorDeCicloDeVida::PausaDeEnvioFallida`] y la secuencia aborta.
/// * `canal_sin_sesion` en la pausa → éxito (no hay nada que cerrar).
/// * núcleo no corriendo → [`ErrorDeCicloDeVida::NucleoNoCorriendo`].
pub fn preparar_reemparejamiento(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeCelulaParaRebind,
    imagen: &str,
    limite_http: u64,
) -> Result<Option<String>, ErrorDeCicloDeVida> {
    // Verificar que el núcleo está en ejecución.
    let inspeccion_nucleo = match cliente.inspeccionar_contenedor(&nombres.nucleo) {
        Ok(inspeccion) => inspeccion,
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::CelulaNoEncontrada);
        }
        Err(error) => return Err(error.into()),
    };
    let estado = inspeccion_nucleo
        .pointer("/State/Status")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ErrorDeCicloDeVida::Configuracion("el núcleo no declara su estado".to_string())
        })?;
    if estado != "running" {
        return Err(ErrorDeCicloDeVida::NucleoNoCorriendo);
    }

    // Paso 3: POST /admin/envio/pausa {"accion":"pausar"}.
    let url_pausa = format!(
        "http://{}:{}/admin/envio/pausa",
        nombres.nucleo, datos.puerto_admin
    );
    let cuerpo_pausa = serde_json::json!({ "accion": "pausar" }).to_string();
    let respuesta_pausa = consultar_por_hermano(
        cliente,
        nombres,
        imagen,
        guion_de_peticion_http(&url_pausa, Some(&cuerpo_pausa), limite_http),
        &datos.red,
    )?;
    match desenlace_de_pausa(&respuesta_pausa)? {
        DesenlaceDePausa::Aplicado { .. } => {}
        DesenlaceDePausa::Fallido { motivo } => {
            return Err(ErrorDeCicloDeVida::PausaDeEnvioFallida { motivo });
        }
        DesenlaceDePausa::CanalSinSesion => {}
    }

    // Paso 4: POST /admin/sesion/cierre como mejor esfuerzo.
    let url_cierre = format!(
        "http://{}:{}/admin/sesion/cierre",
        nombres.nucleo, datos.puerto_admin
    );
    let opciones = OpcionesDeContenedor {
        red: datos.red.clone(),
        cmd: guion_de_cierre_de_sesion(&url_cierre),
    };
    let sonda_de_cierre = match cliente.crear_e_iniciar_contenedor_con_opciones(imagen, opciones) {
        Ok(ResultadoDeArranque::Iniciado { id_contenedor })
        | Ok(ResultadoDeArranque::YaEnEjecucion { id_contenedor }) => id_contenedor,
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada {
                imagen: imagen.to_string(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let espera_cierre = cliente.esperar_contenedor(&sonda_de_cierre);
    let limpieza_cierre = cliente.eliminar_contenedor(&sonda_de_cierre);
    let codigo_cierre = match (espera_cierre, limpieza_cierre) {
        (Ok(codigo), Ok(())) => codigo,
        (Err(error), _) => return Err(error.into()),
        (Ok(_), Err(error)) => return Err(error.into()),
    };
    let aviso = if codigo_cierre == 0 {
        None
    } else {
        Some(format!(
            "aviso: el cierre de sesión devolvió código {codigo_cierre}; se continúa igual"
        ))
    };
    Ok(aviso)
}

/// Fase «descartar sqlstore y rearrancar» (pasos 6-7 de D5): detiene el sidecar sin plazo,
/// ejecuta un contenedor hermano con el volumen montado que borra `sqlstore.db`, rearranca el
/// sidecar y reenvía la pausa de envío reintentando mientras la respuesta sea `sin_conexion`.
pub fn descartar_sqlstore_y_rearrancar(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeCelulaParaRebind,
    imagen: &str,
    plazos: &PlazosDeReemparejamiento,
    limite_http: u64,
) -> Result<(), ErrorDeCicloDeVida> {
    // Paso 6a: detener el sidecar sin plazo.
    cliente.detener_contenedor_sin_plazo(&nombres.sidecar)?;

    // Paso 6b: contenedor hermano con el volumen montado que borra sólo sqlstore.db.
    let cmd_rm = vec![
        "rm".to_string(),
        "-f".to_string(),
        format!("{}/sqlstore.db", RUTA_DE_DATOS_DE_CELULA),
        format!("{}/sqlstore.db-wal", RUTA_DE_DATOS_DE_CELULA),
        format!("{}/sqlstore.db-shm", RUTA_DE_DATOS_DE_CELULA),
    ];
    let opciones_rm = OpcionesDeContenedor {
        red: "none".to_string(),
        cmd: cmd_rm,
    };
    let sonda_rm = match cliente.crear_e_iniciar_contenedor_con_volumen(
        imagen,
        opciones_rm,
        &datos.volumen,
        RUTA_DE_DATOS_DE_CELULA,
    ) {
        Ok(ResultadoDeArranque::Iniciado { id_contenedor })
        | Ok(ResultadoDeArranque::YaEnEjecucion { id_contenedor }) => id_contenedor,
        Err(ErrorDeClienteDocker::NoEncontrado) => {
            return Err(ErrorDeCicloDeVida::ImagenDeSondaNoEncontrada {
                imagen: imagen.to_string(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let espera_rm = cliente.esperar_contenedor(&sonda_rm);
    let limpieza_rm = cliente.eliminar_contenedor(&sonda_rm);
    match (espera_rm, limpieza_rm) {
        (Ok(0), Ok(())) => {}
        (Ok(codigo), Ok(())) => {
            return Err(ErrorDeCicloDeVida::DescarteDeSqlstoreFallido {
                motivo: format!("el rm devolvió código {codigo}"),
            });
        }
        (Err(error), _) => return Err(error.into()),
        (Ok(_), Err(error)) => return Err(error.into()),
    }

    // Paso 7a: rearrancar el sidecar.
    cliente.iniciar_contenedor(&nombres.sidecar)?;

    // Paso 7b: reenviar POST /admin/envio/pausa pausar reintentando mientras sin_conexion.
    let url_pausa = format!(
        "http://{}:{}/admin/envio/pausa",
        nombres.nucleo, datos.puerto_admin
    );
    let cuerpo_pausa = serde_json::json!({ "accion": "pausar" }).to_string();
    let mut ultimo_resultado = None;
    for intento in 0..plazos.intentos_de_pausa {
        let respuesta = consultar_por_hermano(
            cliente,
            nombres,
            imagen,
            guion_de_peticion_http(&url_pausa, Some(&cuerpo_pausa), limite_http),
            &datos.red,
        )?;
        match desenlace_de_pausa(&respuesta)? {
            desenlace @ (DesenlaceDePausa::Aplicado { .. } | DesenlaceDePausa::CanalSinSesion) => {
                ultimo_resultado = Some(desenlace);
                break;
            }
            DesenlaceDePausa::Fallido { motivo } if motivo == "sin_conexion" => {
                if intento + 1 < plazos.intentos_de_pausa {
                    std::thread::sleep(std::time::Duration::from_millis(plazos.cadencia_ms));
                }
            }
            DesenlaceDePausa::Fallido { motivo } => {
                return Err(ErrorDeCicloDeVida::PausaDeEnvioFallida { motivo });
            }
        }
    }
    match ultimo_resultado {
        Some(_) => Ok(()),
        None => Err(ErrorDeCicloDeVida::PausaDeEnvioFallida {
            motivo: "sin_conexion: se agotaron los reintentos de la pausa de envío".to_string(),
        }),
    }
}

/// Desenlace de [`solicitar_emparejamiento`] que SÍ puede llegar al llamador.
///
/// A diferencia de [`DesenlaceDeEmparejamiento`] (el desenlace crudo de la respuesta HTTP), este
/// tipo no admite `Fallido`: dentro de `solicitar_emparejamiento` un `fallido` con motivo
/// `sin_conexion` reintenta hasta agotar el presupuesto y cualquier otro motivo aborta de
/// inmediato con `Err`, así que `Fallido` nunca sobrevive hasta el `Ok` de la función. Angostar el
/// tipo de retorno evita que el llamador tenga que escribir una rama de `match` que el compilador
/// no puede demostrar viva, y evita el par `unreachable!()`/rama muerta que eso producía.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DesenlaceDeSolicitudDeEmparejamiento {
    /// `{"resultado":"codigo","valor":"...","expira_en_ms":N}`. El `metodo` NO viaja aquí: el
    /// llamador ya sabe cuál pidió y debe reportar ESE, no el que el núcleo decida ecoar (D5.8).
    Codigo { valor: String, expira_en_ms: i64 },
    /// `{"resultado":"canal_sin_sesion"}`.
    CanalSinSesion,
}

/// Fase «solicitar emparejamiento» (paso 8 de D5): envía `POST /admin/sesion/emparejamiento` con el
/// método elegido, reintentando mientras la respuesta sea `sin_conexion`.
///
/// * `codigo` → devuelve [`DesenlaceDeSolicitudDeEmparejamiento::Codigo`] con el valor y la
///   expiración.
/// * `canal_sin_sesion` → devuelve [`DesenlaceDeSolicitudDeEmparejamiento::CanalSinSesion`] (el
///   llamador omite el paso 9).
/// * `fallido` con motivo `sin_conexion` → reintenta hasta agotar `intentos_de_emparejamiento`.
/// * cualquier otro `fallido` → [`ErrorDeCicloDeVida::EmparejamientoFallido`] de inmediato, SIN
///   reintentar.
pub fn solicitar_emparejamiento(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeCelulaParaRebind,
    metodo: &str,
    imagen: &str,
    plazos: &PlazosDeReemparejamiento,
    limite_http: u64,
) -> Result<DesenlaceDeSolicitudDeEmparejamiento, ErrorDeCicloDeVida> {
    let url = format!(
        "http://{}:{}/admin/sesion/emparejamiento",
        nombres.nucleo, datos.puerto_admin
    );
    let cuerpo = serde_json::json!({ "metodo": metodo }).to_string();
    let mut ultimo_fallido = None;
    for intento in 0..plazos.intentos_de_emparejamiento {
        let respuesta = consultar_por_hermano(
            cliente,
            nombres,
            imagen,
            guion_de_peticion_http(&url, Some(&cuerpo), limite_http),
            &datos.red,
        )?;
        match desenlace_de_emparejamiento(&respuesta)? {
            DesenlaceDeEmparejamiento::Codigo {
                valor,
                expira_en_ms,
                ..
            } => {
                return Ok(DesenlaceDeSolicitudDeEmparejamiento::Codigo {
                    valor,
                    expira_en_ms,
                });
            }
            DesenlaceDeEmparejamiento::CanalSinSesion => {
                return Ok(DesenlaceDeSolicitudDeEmparejamiento::CanalSinSesion);
            }
            DesenlaceDeEmparejamiento::Fallido { motivo } if motivo == "sin_conexion" => {
                ultimo_fallido = Some(motivo);
                if intento + 1 < plazos.intentos_de_emparejamiento {
                    std::thread::sleep(std::time::Duration::from_millis(plazos.cadencia_ms));
                }
            }
            DesenlaceDeEmparejamiento::Fallido { motivo } => {
                return Err(ErrorDeCicloDeVida::EmparejamientoFallido { motivo });
            }
        }
    }
    Err(ErrorDeCicloDeVida::EmparejamientoFallido {
        motivo: ultimo_fallido.unwrap_or_else(|| {
            "sin_conexion: se agotaron los reintentos de emparejamiento".to_string()
        }),
    })
}

/// Fase «esperar confirmación» (paso 9 de D5): consulta `GET /admin/sesion` hasta que el estado sea
/// `activa`, con un presupuesto de `min(expira_en_ms - ahora_ms, tope)`, usando el tope cuando
/// `expira_en_ms` es 0 (instante absoluto desconocido).
///
/// Devuelve [`ErrorDeCicloDeVida::CodigoExpirado`] si se agota el presupuesto sin llegar a `activa`.
#[allow(clippy::too_many_arguments)]
pub fn esperar_confirmacion(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeCelulaParaRebind,
    expira_en_ms: i64,
    ahora_ms: i64,
    imagen: &str,
    plazos: &PlazosDeReemparejamiento,
    limite_http: u64,
) -> Result<(), ErrorDeCicloDeVida> {
    let presupuesto_ms = if expira_en_ms > 0 {
        let resto = expira_en_ms.saturating_sub(ahora_ms);
        let tope_ms = plazos.tope_de_confirmacion_s.saturating_mul(1000);
        resto.min(tope_ms as i64)
    } else {
        plazos.tope_de_confirmacion_s.saturating_mul(1000) as i64
    };
    let intentos = (presupuesto_ms.max(0) as u64)
        .div_euclid(plazos.cadencia_ms.max(1))
        .max(1);
    let url = format!(
        "http://{}:{}/admin/sesion",
        nombres.nucleo, datos.puerto_admin
    );
    for _ in 0..intentos {
        let respuesta = consultar_por_hermano(
            cliente,
            nombres,
            imagen,
            guion_de_peticion_http(&url, None, limite_http),
            &datos.red,
        )?;
        if estado_de_sesion(&respuesta)? == EstadoDeSesion::Activa {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(plazos.cadencia_ms));
    }
    Err(ErrorDeCicloDeVida::CodigoExpirado)
}

/// Fase «reanudar envío» (parte Docker del paso 10 de D5): envía `POST /admin/envio/pausa
/// reanudar`. `canal_sin_sesion` se trata como éxito; cualquier otro `fallido` es error.
pub fn reanudar_envio(
    cliente: &ClienteDocker,
    nombres: &NombresDeCelula,
    datos: &DatosDeCelulaParaRebind,
    imagen: &str,
    limite_http: u64,
) -> Result<(), ErrorDeCicloDeVida> {
    let url = format!(
        "http://{}:{}/admin/envio/pausa",
        nombres.nucleo, datos.puerto_admin
    );
    let cuerpo = serde_json::json!({ "accion": "reanudar" }).to_string();
    let respuesta = consultar_por_hermano(
        cliente,
        nombres,
        imagen,
        guion_de_peticion_http(&url, Some(&cuerpo), limite_http),
        &datos.red,
    )?;
    match desenlace_de_pausa(&respuesta)? {
        DesenlaceDePausa::Aplicado { .. } | DesenlaceDePausa::CanalSinSesion => Ok(()),
        DesenlaceDePausa::Fallido { motivo } => {
            Err(ErrorDeCicloDeVida::PausaDeEnvioFallida { motivo })
        }
    }
}
