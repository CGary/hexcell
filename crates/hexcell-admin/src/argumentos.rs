//! Dominio de análisis de argumentos de la CLI `hexcell-admin`.
//!
//! Tercera de tres hijas de la tarea 10 de la etapa A-6. Fija la gramática cerrada de la
//! línea de comandos: el grupo `cell` con sus seis subcomandos (`pause`, `unpause`,
//! `terminate`, `rebind`, `list`, `status`), las opciones admitidas por cada uno y el modo
//! de simulación (`--simular`). Las hermanas HEX-074-a y HEX-074-b entregaron el agregado
//! de estado de célula, los códigos de salida y los sumideros tipados; esta tarea los
//! consume sin modificarlos. HEX-081 añade el segundo grupo `config render`, aditivo y
//! sin gramática compartida con `cell`. HEX-084 añade el tercer grupo `reporte tokens`
//! (tarea 23 de la etapa A-6), también aditivo y sin gramática compartida: su analizador
//! valida las fechas `AAAA-MM-DD` en UTC a mano —el workspace no arrastra ningún crate de
//! fechas, por el mismo criterio de D-53— y rechaza por nombre toda copia que se llame
//! `sessions.db` o termine en `-wal`/`-shm`, antes de que ningún archivo se abra.
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
    fn admite_metodo(self) -> bool {
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
    metodo: Option<MetodoDeEmparejamiento>,
    simular: bool,
    confirmar: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Comando {
    Cell(Invocacion),
    ConfigRender(InvocacionRenderizado),
    ReporteTokens(InvocacionReporte),
}

impl Comando {
    /// El subcomando de `cell`, o `None` para `config render` y `reporte tokens`: ninguno de
    /// los dos grupos usa la gramática de `Subcomando`, así que devolver una variante inventada
    /// (como `Listar`) sería mentirle a cualquier llamante que lea este accesor.
    pub fn subcomando(&self) -> Option<Subcomando> {
        match self {
            Self::Cell(i) => Some(i.subcomando),
            Self::ConfigRender(_) => None,
            Self::ReporteTokens(_) => None,
        }
    }
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Cell(i) => i.id(),
            Self::ConfigRender(_) => None,
            Self::ReporteTokens(_) => None,
        }
    }
    pub fn motivo(&self) -> Option<&str> {
        match self {
            Self::Cell(i) => i.motivo(),
            Self::ConfigRender(_) => None,
            Self::ReporteTokens(_) => None,
        }
    }
    pub fn metodo(&self) -> Option<MetodoDeEmparejamiento> {
        match self {
            Self::Cell(i) => i.metodo(),
            Self::ConfigRender(_) => None,
            Self::ReporteTokens(_) => None,
        }
    }
    pub fn simular(&self) -> bool {
        match self {
            Self::Cell(i) => i.simular(),
            Self::ConfigRender(i) => i.simular(),
            Self::ReporteTokens(i) => i.simular(),
        }
    }
    pub fn confirmar(&self) -> bool {
        match self {
            Self::Cell(i) => i.confirmar(),
            Self::ConfigRender(_) => false,
            Self::ReporteTokens(_) => false,
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

/// Resultado válido del análisis del grupo `reporte tokens`: las opciones ya validadas y
/// las fechas ya convertidas a milisegundos desde la época.
///
/// Los campos son privados y no existe ningún constructor público fuera de este módulo: la
/// única forma de obtener una `InvocacionReporte` es a través de [`analizar`]. Las fechas
/// originales se conservan tal cual las escribió el operador —solo para imprimirlas en la
/// línea `TOTAL` sin volver a formatearlas— junto con su valor en milisegundos UTC, que es
/// lo que la consulta necesita para la ventana por `resuelta_ms`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InvocacionReporte {
    celula: String,
    copia: String,
    desde: Option<String>,
    hasta: Option<String>,
    desde_ms: Option<i64>,
    hasta_ms: Option<i64>,
    simular: bool,
}

impl InvocacionReporte {
    /// El valor de `--celula`.
    pub fn celula(&self) -> &str {
        &self.celula
    }
    /// El valor de `--copia`: la ruta de la copia `VACUUM INTO` que el comando leerá.
    pub fn copia(&self) -> &str {
        &self.copia
    }
    /// El texto original de `--desde`, si fue aportado.
    pub fn desde(&self) -> Option<&str> {
        self.desde.as_deref()
    }
    /// El texto original de `--hasta`, si fue aportado.
    pub fn hasta(&self) -> Option<&str> {
        self.hasta.as_deref()
    }
    /// `--desde` como milisegundos desde la época Unix (UTC, inicio de día), si fue aportado.
    pub fn desde_ms(&self) -> Option<i64> {
        self.desde_ms
    }
    /// `--hasta` como milisegundos desde la época Unix (UTC, inicio de día), si fue aportado.
    pub fn hasta_ms(&self) -> Option<i64> {
        self.hasta_ms
    }
    /// Si el operador pidió el modo de simulación (`--simular`).
    pub fn simular(&self) -> bool {
        self.simular
    }
}

/// Método de emparejamiento admitido por `cell rebind` (tarea 13 de A-6, HEX-085-b).
///
/// Tipo LOCAL de la CLI: `hexcell-admin` NO depende del crate `hexcell-canal-whatsmeow`, así que este
/// enumerado reparte sólo la gramática del flag `--metodo`. La traducción al nombre de cable
/// (`qr`, `codigo_de_vinculacion`) vive en la capa de ciclo de vida.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MetodoDeEmparejamiento {
    /// Emparejamiento mediante código QR.
    Qr,
    /// Emparejamiento mediante código de vinculación textual.
    CodigoDeVinculacion,
}

impl MetodoDeEmparejamiento {
    /// Nombre de cable asociado a cada método, tal y como viaja en el cuerpo JSON de
    /// `POST /admin/sesion/emparejamiento`.
    pub fn nombre_de_cable(self) -> &'static str {
        match self {
            Self::Qr => "qr",
            Self::CodigoDeVinculacion => "codigo_de_vinculacion",
        }
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
    /// El método de emparejamiento elegido, si fue aportado.
    pub fn metodo(&self) -> Option<MetodoDeEmparejamiento> {
        self.metodo
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
    /// Error de gramática o de fecha del grupo `reporte tokens`, con el mismo perfil que
    /// [`ErrorDeArgumentos::ConfiguracionInvalida`]: mensaje opaco ya formado.
    ReporteInvalido {
        mensaje: String,
    },
    /// `--copia` se llama `sessions.db` o termina en `-wal`/`-shm`: el reporte solo lee
    /// copias `VACUUM INTO`, nunca la base caliente ni sus archivos de diario.
    ///
    /// El `Display` es la cadena literal fija que AC-3 exige byte a byte, escrita a mano en
    /// el brazo del `Display` y no compuesta con `format!` en el punto de rechazo: ningún
    /// refactor de un ayudante de mensajes compartido puede desviar el texto sin que las
    /// pruebas del mensaje exacto se pongan rojas.
    CopiaEsSessionsDb,
    /// El valor aportado a `--metodo` no corresponde a ningún método de emparejamiento conocido.
    ValorDeOpcionInvalido {
        subcomando: Subcomando,
        opcion: String,
        valor: String,
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
                "grupo desconocido: «{grupo}» (los grupos admitidos son «cell», «config» y «reporte»)"
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
            ErrorDeArgumentos::ReporteInvalido { mensaje } => f.write_str(mensaje),
            // Cadena fija exigida literalmente por AC-3; ver el comentario de la variante.
            ErrorDeArgumentos::CopiaEsSessionsDb => {
                f.write_str("el reporte sólo lee copias VACUUM INTO, nunca sessions.db")
            }
            ErrorDeArgumentos::ValorDeOpcionInvalido {
                subcomando,
                opcion,
                valor,
            } => write!(
                f,
                "valor inválido para «{opcion}» en «{valor}» para «{}» (valores admitidos: qr, codigo_de_vinculacion)",
                subcomando.nombre_en_cli()
            ),
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
  rebind      --id <cell_id> --motivo <texto> --confirmar [--metodo qr|codigo_de_vinculacion]
                                              Sustituir el número de una célula.
  list                                        Listar las células conocidas.
  status      --id <cell_id>                Mostrar el estado de una célula.

Opciones comunes:
  --simular                                   Reportar la acción sin ejecutarla.

Uso: hexcell-admin config render --defecto <ruta> --superposicion <ruta> --salida <ruta> [--simular]
  Renderizar la configuración de una célula fusionando un archivo de valores
  compartidos y uno de superposición contra el esquema cerrado.

Uso: hexcell-admin reporte tokens --celula <id> --copia <ruta.db> [--desde AAAA-MM-DD] [--hasta AAAA-MM-DD] [--simular]
  Reportar el consumo de unidades de presupuesto por conversación de una célula,
  leyendo solo una copia VACUUM INTO de sessions.db, nunca la base caliente.
  El periodo es UTC: --desde inclusivo, --hasta exclusivo.";

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
    if grupo == "reporte" {
        return analizar_reporte(&argumentos[1..]);
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

/// Analiza el grupo `reporte tokens`: exige el subcomando literal `tokens`, recoge
/// `--celula`, `--copia`, `--desde`, `--hasta` y `--simular` (en ambas ortografías
/// `--clave valor` y `--clave=valor`), y valida dos cosas de forma **incondicional**,
/// antes de construir cualquier [`Comando`] y por tanto antes de que `--simular` se
/// consulte en `comandos.rs`:
///
/// * el nombre de `--copia` no puede ser `sessions.db` ni terminar en `-wal`/`-shm`
///   ([`ErrorDeArgumentos::CopiaEsSessionsDb`], AC-3);
/// * `--desde` y `--hasta` deben ser fechas `AAAA-MM-DD` en UTC reales
///   ([`ErrorDeArgumentos::ReporteInvalido`], AC-5).
///
/// Esta incondicionalidad sigue el precedente del grupo `cell`, donde `validar_opciones`
/// rechaza opciones obligatorias o malformadas independientemente de `--simular`, y es la
/// razón por la que `--simular` con una `--copia` llamada `sessions.db` sigue siendo un
/// `UsoIncorrecto`: el corte del modo de simulación (AC-4) aplica a cualquier otra ruta.
fn analizar_reporte(argumentos: &[String]) -> Result<Comando, ErrorDeArgumentos> {
    if argumentos.first().map(String::as_str) != Some("tokens") {
        return Err(reporte_error(
            "subcomando de reporte desconocido; se esperaba «tokens»",
        ));
    }
    let mut celula: Option<String> = None;
    let mut copia: Option<String> = None;
    let mut desde: Option<String> = None;
    let mut hasta: Option<String> = None;
    let mut simular = false;
    let mut i = 1;
    while i < argumentos.len() {
        let arg = &argumentos[i];
        if arg == "--simular" {
            if simular {
                return Err(reporte_error("opción repetida: --simular"));
            }
            simular = true;
            i += 1;
            continue;
        }
        let (opcion, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(a, v)| (a, Some(v)));
        let destino = match opcion {
            "--celula" => &mut celula,
            "--copia" => &mut copia,
            "--desde" => &mut desde,
            "--hasta" => &mut hasta,
            _ => {
                return Err(reporte_error(&format!(
                    "opción desconocida para «reporte tokens»: «{arg}»"
                )));
            }
        };
        if destino.is_some() {
            return Err(reporte_error(&format!("opción repetida: «{opcion}»")));
        }
        let valor = match inline {
            Some(v) if !v.is_empty() => v.to_string(),
            Some(_) => return Err(reporte_error(&format!("falta el valor de «{opcion}»"))),
            None => {
                let v = argumentos
                    .get(i + 1)
                    .ok_or_else(|| reporte_error(&format!("falta el valor de «{opcion}»")))?;
                if v.is_empty() {
                    return Err(reporte_error(&format!("falta el valor de «{opcion}»")));
                }
                i += 1;
                v.clone()
            }
        };
        *destino = Some(valor);
        i += 1;
    }
    let celula = requerido_de_reporte(celula, "--celula")?;
    let copia = requerido_de_reporte(copia, "--copia")?;
    if es_ruta_de_copia_prohibida(&copia) {
        return Err(ErrorDeArgumentos::CopiaEsSessionsDb);
    }
    let desde_ms = match desde.as_deref() {
        Some(texto) => Some(
            fecha_utc_a_ms_desde_epoca(texto)
                .map_err(|_| reporte_error(&format!("fecha inválida en --desde: «{texto}»")))?,
        ),
        None => None,
    };
    let hasta_ms = match hasta.as_deref() {
        Some(texto) => Some(
            fecha_utc_a_ms_desde_epoca(texto)
                .map_err(|_| reporte_error(&format!("fecha inválida en --hasta: «{texto}»")))?,
        ),
        None => None,
    };
    Ok(Comando::ReporteTokens(InvocacionReporte {
        celula,
        copia,
        desde,
        hasta,
        desde_ms,
        hasta_ms,
        simular,
    }))
}

fn reporte_error(mensaje: &str) -> ErrorDeArgumentos {
    ErrorDeArgumentos::ReporteInvalido {
        mensaje: mensaje.to_string(),
    }
}

fn requerido_de_reporte(valor: Option<String>, opcion: &str) -> Result<String, ErrorDeArgumentos> {
    valor.ok_or_else(|| {
        reporte_error(&format!(
            "falta la opción obligatoria «{opcion}» para «reporte tokens»"
        ))
    })
}

/// ¿Es una ruta de copia que el reporte nunca debe abrir?
///
/// Rechaza por nombre, sin tocar el sistema de archivos: el archivo se llame exactamente
/// `sessions.db` (la base caliente, cuya lectura en vivo prohíbe adr-0024) o la ruta
/// termine en `-wal`/`-shm` (los diarios de SQLite, que tampoco son una copia). La
/// comprobación es léxica a propósito: así se cumple que la copia **no se abre nunca** en
/// estos casos, ni siquiera para comprobar que existe.
fn es_ruta_de_copia_prohibida(copia: &str) -> bool {
    let ruta = std::path::Path::new(copia);
    ruta.file_name().and_then(|nombre| nombre.to_str()) == Some("sessions.db")
        || copia.ends_with("-wal")
        || copia.ends_with("-shm")
}

/// Convierte una fecha `AAAA-MM-DD` en UTC al número de milisegundos desde la época Unix,
/// o rechaza la fecha si no existe en el calendario gregoriano.
///
/// El workspace no arrastra ningún crate de fechas —el mismo criterio a mano con que D-53
/// descartó los analizadores de CLI—, así que la validación y la aritmética civil viven
/// aquí: longitud exacta y guiones en las posiciones 4 y 7, dígitos ASCII en el resto, mes
/// en `1..=12`, día en `1..=días_del_mes(año, mes)` con la regla de año bisiesto correcta
/// (divisible por 4, no por 100 salvo también por 400), y después la fórmula civil-a-días
/// de Howard Hinnant (`days_from_civil`, dominio público) multiplicada por 86 400 000.
///
/// El rechazo de una fecha inexistente como `2026-02-30` es exactamente la frontera que
/// AC-5 exige: una fecha que el calendario no tiene no puede delimitar ningún periodo.
fn fecha_utc_a_ms_desde_epoca(texto: &str) -> Result<i64, ()> {
    let bytes = texto.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, &b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return Err(());
    }
    let año = parsear_numero_de_4_digitos(&texto[0..4]).ok_or(())?;
    let mes = parsear_numero_de_2_digitos(&texto[5..7]).ok_or(())?;
    let día = parsear_numero_de_2_digitos(&texto[8..10]).ok_or(())?;
    if !(1..=12).contains(&mes) || !(1..=dias_del_mes(año, mes)).contains(&día) {
        return Err(());
    }
    Ok(dias_desde_la_epoca(año, mes, día) * MILISEGUNDOS_POR_DIA)
}

const MILISEGUNDOS_POR_DIA: i64 = 86_400_000;

fn parsear_numero_de_4_digitos(texto: &str) -> Option<i64> {
    texto.parse().ok()
}

fn parsear_numero_de_2_digitos(texto: &str) -> Option<u32> {
    texto.parse().ok()
}

/// Regla gregoriana de año bisiesto: divisible por 4, salvo los divisibles por 100 que no
/// lo sean también por 400 (1900 no lo es; 2000 sí).
fn es_bisiesto(año: i64) -> bool {
    (año % 4 == 0 && año % 100 != 0) || año % 400 == 0
}

/// Días del mes para un mes ya validado en `1..=12`; `0` es el brazo inalcanzable que
/// mantiene la función total sin entrar en pánico.
fn dias_del_mes(año: i64, mes: u32) -> u32 {
    match mes {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if es_bisiesto(año) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Días transcurridos desde la época Unix (1970-01-01) hasta la fecha civil dada, con la
/// fórmula de Howard Hinnant (`days_from_civil`), que es exacta para años en `0..=9999`
/// —el rango que admite `AAAA`— sin necesidad de corrección de redondeo: el único año
/// negativo que produce la fórmula es el 0 con enero o febrero (`año_ajustado = -1`), y
/// `(-1 - 399) / 400` divide exacto.
fn dias_desde_la_epoca(año: i64, mes: u32, día: u32) -> i64 {
    let año_ajustado = if mes <= 2 { año - 1 } else { año };
    let era = if año_ajustado >= 0 {
        año_ajustado
    } else {
        año_ajustado - 399
    } / 400;
    let año_de_la_era = año_ajustado - era * 400;
    let mes_de_la_era = if mes > 2 { mes - 3 } else { mes + 9 } as i64;
    let día_de_la_era = (153 * mes_de_la_era + 2) / 5 + día as i64 - 1;
    let día_de_la_era = día_de_la_era + 365 * año_de_la_era + año_de_la_era / 4
        - año_de_la_era / 100
        + año_de_la_era / 400;
    era * 146097 + día_de_la_era - 719468
}

struct OpcionesRecogidas {
    id: Option<String>,
    motivo: Option<String>,
    metodo: Option<String>,
    simular: bool,
    confirmar: Option<bool>,
}

fn extraer_opciones(
    subcomando: Subcomando,
    argumentos: &[String],
) -> Result<OpcionesRecogidas, ErrorDeArgumentos> {
    let mut id: Option<String> = None;
    let mut motivo: Option<String> = None;
    let mut metodo: Option<String> = None;
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
        if let Some(valor) = arg.strip_prefix("--metodo=") {
            rechazar_si_repetido(&metodo, subcomando, "--metodo")?;
            rechazar_si_vacio(valor, subcomando, "--metodo")?;
            metodo = Some(valor.to_string());
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
        if arg == "--metodo" {
            rechazar_si_repetido(&metodo, subcomando, "--metodo")?;
            let valor = tomar_valor(argumentos, i, subcomando, "--metodo")?;
            metodo = Some(valor.to_string());
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
        metodo,
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
    if opciones.metodo.is_some() && !subcomando.admite_metodo() {
        return Err(ErrorDeArgumentos::OpcionNoAdmitida {
            subcomando,
            opcion: "--metodo".to_string(),
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
    // `--metodo` es opcional: cuando `cell rebind` no lo aporta, se asume `qr`.
    let metodo = match opciones.metodo.as_deref() {
        Some(valor) => Some(parsear_metodo(valor, subcomando)?),
        None => {
            if subcomando == Subcomando::Reemparejar {
                Some(MetodoDeEmparejamiento::Qr)
            } else {
                None
            }
        }
    };
    Ok(Invocacion {
        subcomando,
        id: opciones.id.clone(),
        motivo: opciones.motivo.clone(),
        metodo,
        simular: opciones.simular,
        confirmar: opciones.confirmar.unwrap_or(false),
    })
}

/// Traduce el valor textual de `--metodo` a la variante correspondiente, o rechaza con
/// [`ErrorDeArgumentos::ValorDeOpcionInvalido`] si no es `qr` ni `codigo_de_vinculacion`.
fn parsear_metodo(
    valor: &str,
    subcomando: Subcomando,
) -> Result<MetodoDeEmparejamiento, ErrorDeArgumentos> {
    match valor {
        "qr" => Ok(MetodoDeEmparejamiento::Qr),
        "codigo_de_vinculacion" => Ok(MetodoDeEmparejamiento::CodigoDeVinculacion),
        otro => Err(ErrorDeArgumentos::ValorDeOpcionInvalido {
            subcomando,
            opcion: "--metodo".to_string(),
            valor: otro.to_string(),
        }),
    }
}
