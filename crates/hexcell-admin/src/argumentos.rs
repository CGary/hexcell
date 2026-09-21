//! Dominio de análisis de argumentos de la CLI `hexcell-admin`.
//!
//! Tercera de tres hijas de la tarea 10 de la etapa A-6. Fija la gramática cerrada de la
//! línea de comandos: el grupo `cell` con sus seis subcomandos (`pause`, `unpause`,
//! `terminate`, `rebind`, `list`, `status`), las opciones admitidas por cada uno y el modo
//! de simulación (`--simular`). Las hermanas HEX-074-a y HEX-074-b entregaron el agregado
//! de estado de célula, los códigos de salida y los sumideros tipados; esta tarea los
//! consume sin modificarlos. HEX-081 añade el segundo grupo `config render`, aditivo y
//! sin gramática compartida con `cell`.
//!
//! El analizador es una función pura sobre una porción de argumentos: nunca lee `std::env`
//! por sí misma, de modo que el crate de pruebas externo puede ejercitarlo con un
//! `Vec<String>` propio. Sólo `src/main.rs` recoge los argumentos del proceso.
//!
//! Ningún camino de este módulo entra en pánico: todo fallo de análisis es un valor de
//! [`ErrorDeArgumentos`] que [`crate::comandos::ejecutar`] convierte en un
//! [`crate::codigo_de_salida::CodigoDeSalida`].

use std::fmt;

/// Los seis subcomandos declarados bajo el grupo `cell`.
///
/// Enumerado cerrado a propósito (sin `#[non_exhaustive]`), siguiendo el precedente de
/// `EstadoDeCelula` y `CodigoDeSalida`: las pruebas externas lo emparejan sin brazo por
/// defecto, de modo que añadir o quitar una variante rompe esa compilación.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subcomando {
    /// `cell pause --id <cell_id>`.
    Pausar,
    /// `cell unpause --id <cell_id>`.
    Reanudar,
    /// `cell terminate --id <cell_id> --confirmar`.
    Retirar,
    /// `cell rebind --id <cell_id> --motivo "<texto>" --confirmar`.
    Reemparejar,
    /// `cell list`.
    Listar,
    /// `cell status --id <cell_id>`.
    Estado,
}

impl Subcomando {
    /// Nombre del subcomando tal y como se escribe en la línea de comandos.
    pub fn nombre_en_cli(self) -> &'static str {
        match self {
            Subcomando::Pausar => "pause",
            Subcomando::Reanudar => "unpause",
            Subcomando::Retirar => "terminate",
            Subcomando::Reemparejar => "rebind",
            Subcomando::Listar => "list",
            Subcomando::Estado => "status",
        }
    }

    fn requiere_id(self) -> bool {
        !matches!(self, Subcomando::Listar)
    }
    fn admite_id(self) -> bool {
        !matches!(self, Subcomando::Listar)
    }
    fn admite_motivo(self) -> bool {
        self == Subcomando::Reemparejar
    }
    fn requiere_motivo(self) -> bool {
        self == Subcomando::Reemparejar
    }
    fn requiere_confirmar(self) -> bool {
        matches!(self, Subcomando::Retirar | Subcomando::Reemparejar)
    }
    fn admite_confirmar(self) -> bool {
        self.requiere_confirmar()
    }
}

/// Resultado válido del análisis de argumentos: un subcomando y sus opciones ya validadas.
///
/// Los campos son privados y no existe ningún constructor público fuera de este módulo: la
/// única forma de obtener una `Invocacion` es a través de [`analizar`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Invocacion {
    subcomando: Subcomando,
    id: Option<String>,
    motivo: Option<String>,
    simular: bool,
    confirmar: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Comando {
    Cell(Invocacion),
    ConfigRender(InvocacionRenderizado),
}

impl Comando {
    /// El subcomando de `cell`, o `None` para `config render`: el grupo `config` no
    /// tiene subcomandos de `Subcomando`, así que devolver una variante inventada
    /// (como `Listar`) sería mentirle a cualquier llamante que lea este accesor.
    pub fn subcomando(&self) -> Option<Subcomando> {
        match self {
            Self::Cell(i) => Some(i.subcomando),
            Self::ConfigRender(_) => None,
        }
    }
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Cell(i) => i.id(),
            Self::ConfigRender(_) => None,
        }
    }
    pub fn motivo(&self) -> Option<&str> {
        match self {
            Self::Cell(i) => i.motivo(),
            Self::ConfigRender(_) => None,
        }
    }
    pub fn simular(&self) -> bool {
        match self {
            Self::Cell(i) => i.simular(),
            Self::ConfigRender(i) => i.simular(),
        }
    }
    pub fn confirmar(&self) -> bool {
        match self {
            Self::Cell(i) => i.confirmar(),
            Self::ConfigRender(_) => false,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InvocacionRenderizado {
    defecto: String,
    superposicion: String,
    salida: String,
    simular: bool,
}

impl InvocacionRenderizado {
    pub fn defecto(&self) -> &str {
        &self.defecto
    }
    pub fn superposicion(&self) -> &str {
        &self.superposicion
    }
    pub fn salida(&self) -> &str {
        &self.salida
    }
    pub fn simular(&self) -> bool {
        self.simular
    }
}

impl Invocacion {
    /// El subcomando reconocido.
    pub fn subcomando(&self) -> Subcomando {
        self.subcomando
    }
    /// El valor de `--id`, si fue aportado.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    /// El valor de `--motivo`, si fue aportado.
    pub fn motivo(&self) -> Option<&str> {
        self.motivo.as_deref()
    }
    /// Si el operador pidió el modo de simulación (`--simular`).
    pub fn simular(&self) -> bool {
        self.simular
    }
    /// Si el operador aportó la bandera de confirmación (`--confirmar`).
    pub fn confirmar(&self) -> bool {
        self.confirmar
    }
}

/// Rechazo tipado del análisis de argumentos.
///
/// Enumerado cerrado (sin `#[non_exhaustive]`) por el mismo motivo que `Subcomando`,
/// `EstadoDeCelula` y `CodigoDeSalida`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrorDeArgumentos {
    /// No se aportó ningún argumento tras el nombre del programa.
    SinSubcomando,
    /// El grupo de nivel superior no es `cell` ni `config`.
    GrupoDesconocido {
        grupo: String,
    },
    /// Tras `cell` no vino ningún nombre de subcomando conocido.
    SubcomandoDesconocido {
        nombre: String,
    },
    /// Apareció una opción `--...` que el subcomando no admite.
    OpcionDesconocida {
        subcomando: Subcomando,
        opcion: String,
    },
    /// La misma opción apareció dos veces en la misma invocación.
    OpcionRepetida {
        subcomando: Subcomando,
        opcion: String,
    },
    /// Una opción que exige valor (`--id`, `--motivo`) apareció sin valor detrás.
    FaltaValorDeOpcion {
        subcomando: Subcomando,
        opcion: String,
    },
    /// Una opción obligatoria para el subcomando no fue aportada.
    FaltaOpcionObligatoria {
        subcomando: Subcomando,
        opcion: String,
    },
    /// Una opción que el subcomando no admite fue aportada.
    OpcionNoAdmitida {
        subcomando: Subcomando,
        opcion: String,
    },
    /// Sobró un argumento posicional tras las opciones del subcomando.
    ArgumentoPosicionalSobrante {
        subcomando: Subcomando,
        argumento: String,
    },
    ConfiguracionInvalida {
        mensaje: String,
    },
}

impl fmt::Display for ErrorDeArgumentos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorDeArgumentos::SinSubcomando => {
                write!(f, "falta el subcomando: se esperaba «cell <subcomando>»")
            }
            ErrorDeArgumentos::GrupoDesconocido { grupo } => write!(
                f,
                "grupo desconocido: «{grupo}» (los grupos admitidos son «cell» y «config»)"
            ),
            ErrorDeArgumentos::SubcomandoDesconocido { nombre } => write!(
                f,
                "subcomando desconocido: «{nombre}» (subcomandos admitidos: pause, unpause, \
                 terminate, rebind, list, status)"
            ),
            ErrorDeArgumentos::OpcionDesconocida { subcomando, opcion } => write!(
                f,
                "opción desconocida para «{}»: «{opcion}»",
                subcomando.nombre_en_cli()
            ),
            ErrorDeArgumentos::OpcionRepetida { subcomando, opcion } => write!(
                f,
                "opción repetida para «{}»: «{opcion}»",
                subcomando.nombre_en_cli()
            ),
            ErrorDeArgumentos::FaltaValorDeOpcion { subcomando, opcion } => write!(
                f,
                "falta el valor de «{opcion}» para «{}»",
                subcomando.nombre_en_cli()
            ),
            ErrorDeArgumentos::FaltaOpcionObligatoria { subcomando, opcion } => write!(
                f,
                "falta la opción obligatoria «{opcion}» para «{}»",
                subcomando.nombre_en_cli()
            ),
            ErrorDeArgumentos::OpcionNoAdmitida { subcomando, opcion } => write!(
                f,
                "la opción «{opcion}» no se admite para «{}»",
                subcomando.nombre_en_cli()
            ),
            ErrorDeArgumentos::ArgumentoPosicionalSobrante {
                subcomando,
                argumento,
            } => write!(
                f,
                "argumento posicional sobrante para «{}»: «{argumento}»",
                subcomando.nombre_en_cli()
            ),
            ErrorDeArgumentos::ConfiguracionInvalida { mensaje } => f.write_str(mensaje),
        }
    }
}

impl std::error::Error for ErrorDeArgumentos {}

/// Texto de uso en español, escrito tal cual se envía al sumidero de diagnóstico junto al
/// mensaje de error concreto.
pub const TEXTO_DE_USO: &str = "\
Uso: hexcell-admin cell <subcomando> [opciones]

Subcomandos:
  pause       --id <cell_id>                Suspender temporalmente una célula.
  unpause     --id <cell_id>                Reactivar una célula.
  terminate   --id <cell_id> --confirmar    Eliminar definitivamente una célula.
  rebind      --id <cell_id> --motivo <texto> --confirmar
                                              Sustituir el número de una célula.
  list                                        Listar las células conocidas.
  status      --id <cell_id>                Mostrar el estado de una célula.

Opciones comunes:
  --simular                                   Reportar la acción sin ejecutarla.

Uso: hexcell-admin config render --defecto <ruta> --superposicion <ruta> --salida <ruta> [--simular]
  Renderizar la configuración de una célula fusionando un archivo de valores
  compartidos y uno de superposición contra el esquema cerrado.";

/// Analiza una porción de argumentos y produce una [`Invocacion`] validada o un
/// [`ErrorDeArgumentos`] con la forma del rechazo.
///
/// Función pura sobre la porción de argumentos que recibe: nunca lee `std::env` por sí
/// misma. El único punto del proceso que recoge los argumentos del sistema operativo es
/// `src/main.rs`. La gramática cerrada (grupo `cell` con sus seis subcomandos, reglas de
/// `--id`/`--motivo`/`--confirmar`/`--simular`, ambas ortografías `--clave valor` y
/// `--clave=valor`) vive documentada en `adr-0036`.
pub fn analizar(argumentos: &[String]) -> Result<Comando, ErrorDeArgumentos> {
    if argumentos.is_empty() {
        return Err(ErrorDeArgumentos::SinSubcomando);
    }
    let grupo = &argumentos[0];
    if grupo == "config" {
        return analizar_configuracion(&argumentos[1..]);
    }
    if grupo != "cell" {
        return Err(ErrorDeArgumentos::GrupoDesconocido {
            grupo: grupo.clone(),
        });
    }
    if argumentos.len() < 2 {
        return Err(ErrorDeArgumentos::SinSubcomando);
    }
    let subcomando = match argumentos[1].as_str() {
        "pause" => Subcomando::Pausar,
        "unpause" => Subcomando::Reanudar,
        "terminate" => Subcomando::Retirar,
        "rebind" => Subcomando::Reemparejar,
        "list" => Subcomando::Listar,
        "status" => Subcomando::Estado,
        otro => {
            return Err(ErrorDeArgumentos::SubcomandoDesconocido {
                nombre: otro.to_string(),
            });
        }
    };
    let opciones = extraer_opciones(subcomando, &argumentos[2..])?;
    validar_opciones(subcomando, &opciones).map(Comando::Cell)
}

fn analizar_configuracion(argumentos: &[String]) -> Result<Comando, ErrorDeArgumentos> {
    if argumentos.first().map(String::as_str) != Some("render") {
        return Err(config_error(
            "subcomando de configuración desconocido; se esperaba «render»",
        ));
    }
    let mut defecto = None;
    let mut superposicion = None;
    let mut salida = None;
    let mut simular = false;
    let mut i = 1;
    while i < argumentos.len() {
        let arg = &argumentos[i];
        if arg == "--simular" {
            if simular {
                return Err(config_error("opción repetida: --simular"));
            }
            simular = true;
            i += 1;
            continue;
        }
        let (opcion, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(a, v)| (a, Some(v)));
        let destino = match opcion {
            "--defecto" => &mut defecto,
            "--superposicion" => &mut superposicion,
            "--salida" => &mut salida,
            _ => {
                return Err(config_error(&format!(
                    "opción desconocida para «config render»: «{arg}»"
                )));
            }
        };
        if destino.is_some() {
            return Err(config_error(&format!("opción repetida: «{opcion}»")));
        }
        let valor = match inline {
            Some(v) if !v.is_empty() => v.to_string(),
            Some(_) => return Err(config_error(&format!("falta el valor de «{opcion}»"))),
            None => {
                let v = argumentos
                    .get(i + 1)
                    .ok_or_else(|| config_error(&format!("falta el valor de «{opcion}»")))?;
                if v.is_empty() {
                    return Err(config_error(&format!("falta el valor de «{opcion}»")));
                }
                i += 1;
                v.clone()
            }
        };
        *destino = Some(valor);
        i += 1;
    }
    let requerido = |v: Option<String>, n: &str| {
        v.ok_or_else(|| {
            config_error(&format!(
                "falta la opción obligatoria «{n}» para «config render»"
            ))
        })
    };
    Ok(Comando::ConfigRender(InvocacionRenderizado {
        defecto: requerido(defecto, "--defecto")?,
        superposicion: requerido(superposicion, "--superposicion")?,
        salida: requerido(salida, "--salida")?,
        simular,
    }))
}

fn config_error(mensaje: &str) -> ErrorDeArgumentos {
    ErrorDeArgumentos::ConfiguracionInvalida {
        mensaje: mensaje.to_string(),
    }
}

struct OpcionesRecogidas {
    id: Option<String>,
    motivo: Option<String>,
    simular: bool,
    confirmar: Option<bool>,
}

fn extraer_opciones(
    subcomando: Subcomando,
    argumentos: &[String],
) -> Result<OpcionesRecogidas, ErrorDeArgumentos> {
    let mut id: Option<String> = None;
    let mut motivo: Option<String> = None;
    let mut simular = false;
    let mut confirmar: Option<bool> = None;
    let mut i = 0;

    while i < argumentos.len() {
        let arg = &argumentos[i];

        if let Some(valor) = arg.strip_prefix("--id=") {
            rechazar_si_repetido(&id, subcomando, "--id")?;
            rechazar_si_vacio(valor, subcomando, "--id")?;
            id = Some(valor.to_string());
            i += 1;
            continue;
        }
        if let Some(valor) = arg.strip_prefix("--motivo=") {
            rechazar_si_repetido(&motivo, subcomando, "--motivo")?;
            rechazar_si_vacio(valor, subcomando, "--motivo")?;
            motivo = Some(valor.to_string());
            i += 1;
            continue;
        }
        if arg == "--id" {
            rechazar_si_repetido(&id, subcomando, "--id")?;
            let valor = tomar_valor(argumentos, i, subcomando, "--id")?;
            id = Some(valor.to_string());
            i += 2;
            continue;
        }
        if arg == "--motivo" {
            rechazar_si_repetido(&motivo, subcomando, "--motivo")?;
            let valor = tomar_valor(argumentos, i, subcomando, "--motivo")?;
            motivo = Some(valor.to_string());
            i += 2;
            continue;
        }
        if arg == "--simular" {
            if simular {
                return Err(ErrorDeArgumentos::OpcionRepetida {
                    subcomando,
                    opcion: "--simular".to_string(),
                });
            }
            simular = true;
            i += 1;
            continue;
        }
        if arg == "--confirmar" {
            if confirmar.is_some() {
                return Err(ErrorDeArgumentos::OpcionRepetida {
                    subcomando,
                    opcion: "--confirmar".to_string(),
                });
            }
            confirmar = Some(true);
            i += 1;
            continue;
        }
        if arg.starts_with("--") {
            return Err(ErrorDeArgumentos::OpcionDesconocida {
                subcomando,
                opcion: arg.clone(),
            });
        }
        return Err(ErrorDeArgumentos::ArgumentoPosicionalSobrante {
            subcomando,
            argumento: arg.clone(),
        });
    }

    Ok(OpcionesRecogidas {
        id,
        motivo,
        simular,
        confirmar,
    })
}

fn rechazar_si_repetido(
    existente: &Option<String>,
    subcomando: Subcomando,
    opcion: &str,
) -> Result<(), ErrorDeArgumentos> {
    if existente.is_some() {
        Err(ErrorDeArgumentos::OpcionRepetida {
            subcomando,
            opcion: opcion.to_string(),
        })
    } else {
        Ok(())
    }
}

fn rechazar_si_vacio(
    valor: &str,
    subcomando: Subcomando,
    opcion: &str,
) -> Result<(), ErrorDeArgumentos> {
    if valor.is_empty() {
        Err(ErrorDeArgumentos::FaltaValorDeOpcion {
            subcomando,
            opcion: opcion.to_string(),
        })
    } else {
        Ok(())
    }
}

fn tomar_valor<'a>(
    argumentos: &'a [String],
    indice: usize,
    subcomando: Subcomando,
    opcion: &str,
) -> Result<&'a String, ErrorDeArgumentos> {
    let valor = argumentos
        .get(indice + 1)
        .ok_or(ErrorDeArgumentos::FaltaValorDeOpcion {
            subcomando,
            opcion: opcion.to_string(),
        })?;
    rechazar_si_vacio(valor, subcomando, opcion)?;
    Ok(valor)
}

fn validar_opciones(
    subcomando: Subcomando,
    opciones: &OpcionesRecogidas,
) -> Result<Invocacion, ErrorDeArgumentos> {
    if opciones.id.is_some() && !subcomando.admite_id() {
        return Err(ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando,
            opcion: "--id".to_string(),
        });
    }
    if opciones.motivo.is_some() && !subcomando.admite_motivo() {
        return Err(ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando,
            opcion: "--motivo".to_string(),
        });
    }
    if opciones.confirmar.is_some() && !subcomando.admite_confirmar() {
        return Err(ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando,
            opcion: "--confirmar".to_string(),
        });
    }
    if subcomando.requiere_id() && opciones.id.is_none() {
        return Err(ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando,
            opcion: "--id".to_string(),
        });
    }
    if subcomando.requiere_motivo() && opciones.motivo.is_none() {
        return Err(ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando,
            opcion: "--motivo".to_string(),
        });
    }
    if subcomando.requiere_confirmar() && opciones.confirmar.is_none() {
        return Err(ErrorDeArgumentos::FaltaOpcionObligatoria {
            subcomando,
            opcion: "--confirmar".to_string(),
        });
    }
    Ok(Invocacion {
        subcomando,
        id: opciones.id.clone(),
        motivo: opciones.motivo.clone(),
        simular: opciones.simular,
        confirmar: opciones.confirmar.unwrap_or(false),
    })
}
