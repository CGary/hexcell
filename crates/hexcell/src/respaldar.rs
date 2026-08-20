//! Servicio de aplicación para el modo de respaldo de la célula por el operador.
//!
//! Orquesta el respaldo de las cinco bases de una célula (`sessions.db`, `knowledge_live.db`,
//! `adapter_identity.db`, más `sqlstore.db` e `identidad.db` del sidecar sobre IPC) tras verificar
//! que los cinco destinos en el directorio especificado están libres y accesibles.
//!
//! # Disciplina operacional: pausa previa del núcleo
//!
//! La superficie se ejecuta bajo la disciplina operacional de **núcleo detenido y sidecar en
//! ejecución**. Tres razones justifican este diseño:
//!
//! 1. **Socket IPC de conexión única**: El sidecar aplica relevo de conexión única donde la más
//!    reciente gana (`servidor/manejo.go`, `protocolo-ipc-nucleo-sidecar.md`). Si un proceso de
//!    respaldo se conectara con el núcleo en ejecución, desplazaría al núcleo; el núcleo se
//!    reconectaría a los ~500 ms y desplazaría a su vez la conexión del respaldo, cerrando esa
//!    conexión y provocando que el `acuse_respaldo_sqlstore` se pierda y la operación falle.
//! 2. **Riesgo de aperturas rw en migración**: `GestorDePools::abrir` aplica migraciones si el
//!    esquema lo requiere. Ejecutar migraciones desde un segundo proceso sobre bases SQLite vivas
//!    introduce riesgos de concurrencia no cubiertos por las garantías de `VACUUM INTO` de `adr-0020`.
//! 3. **Semántica de salida para el operador**: Permite entregar un código de salida (`ExitCode`)
//!    y un mensaje claro en `stderr` identificando la base que falló, lo que un disparador por
//!    señales en segundo plano no podría entregar directamente al operador.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_storage::{
    AlmacenDeIdentidad, CopiaVerificada, ErrorDeAlmacen, GestorDePools,
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
    NOMBRE_DE_ARCHIVO_DE_SESIONES, verificar_destino_disponible,
};

use crate::configuracion::{
    HEXCELL_ID_CELULA, HEXCELL_RUTA_DATOS, HEXCELL_SOCKET_IPC, RUTA_SOCKET_IPC_POR_DEFECTO,
};
use crate::emparejar::esperar_conexion_activa;
use crate::respaldo::{
    ResultadoRespaldoIdentidad, ResultadoRespaldoSqlstore, ordenar_respaldo_identidad,
    ordenar_respaldo_sqlstore, respaldar_celula_con_ronda,
};

/// Plazo por omisión en segundos para el modo de respaldo.
pub const PLAZO_RESPALDAR_POR_DEFECTO_SEGUNDOS: u64 = 60;
/// Variable de entorno opcional para ajustar el plazo en segundos.
pub const HEXCELL_RESPALDAR_PLAZO_SEGUNDOS: &str = "HEXCELL_RESPALDAR_PLAZO_SEGUNDOS";

const NOMBRE_CANONICO_SQLSTORE: &str = "sqlstore.db";
const NOMBRE_CANONICO_IDENTIDAD: &str = "identidad.db";

/// Resumen agregado de las cinco bases respaldadas.
#[derive(Debug)]
pub struct ResumenDeRespaldoCompleto {
    /// Copias verificadas de las cinco bases (`sqlstore.db`, `identidad.db`, `sessions.db`, `knowledge_live.db`, `adapter_identity.db`).
    pub copias: Vec<CopiaVerificada>,
    /// Identificador de ronda compartido por las cinco copias, correlacionable con los acuses del sidecar.
    pub identificador_de_ronda: String,
}

/// Errores durante la ejecución del subcomando `respaldar`.
#[derive(Debug)]
pub enum ErrorModoRespaldar {
    /// No se proporcionó el argumento obligatorio `--directorio`.
    FaltaDirectorio,
    /// La ruta especificada en `--directorio` es relativa y se exige absoluta.
    DirectorioRelativo,
    /// Se especificó un argumento no reconocido.
    ArgumentoDesconocido(String),
    /// Falta una variable de entorno obligatoria para la configuración básica.
    FaltaVariableDeEntorno(&'static str),
    /// Error en la capa de almacenamiento local.
    Almacen(ErrorDeAlmacen),
    /// Error en la capa de transporte/canal IPC.
    Canal(ErrorCanalWhatsmeow),
    /// No se pudo establecer conexión activa con el sidecar IPC dentro del plazo.
    ConexionNoEstablecida,
    /// El sidecar rechazó u ordenó un respaldo fallido del `sqlstore`.
    SqlstoreFallido {
        /// Motivo reportado por el sidecar.
        motivo: String,
    },
    /// El sidecar rechazó u ordenó un respaldo fallido de `identidad.db`.
    IdentidadFallido {
        /// Motivo reportado por el sidecar.
        motivo: String,
    },
}

impl fmt::Display for ErrorModoRespaldar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FaltaDirectorio => {
                write!(f, "falta el argumento obligatorio --directorio <ruta>")
            }
            Self::DirectorioRelativo => {
                write!(f, "la ruta del directorio de respaldo debe ser absoluta")
            }
            Self::ArgumentoDesconocido(arg) => write!(f, "argumento desconocido: «{arg}»"),
            Self::FaltaVariableDeEntorno(var) => {
                write!(f, "falta la variable de entorno obligatoria {var}")
            }
            Self::Almacen(e) => write!(f, "error en almacenamiento: {e}"),
            Self::Canal(e) => write!(f, "error en canal whatsmeow: {e}"),
            Self::ConexionNoEstablecida => write!(
                f,
                "no se pudo establecer conexión activa con el sidecar IPC dentro del plazo"
            ),
            Self::SqlstoreFallido { motivo } => {
                write!(f, "fallo en respaldo de sqlstore.db: {motivo}")
            }
            Self::IdentidadFallido { motivo } => {
                write!(f, "fallo en respaldo de identidad.db: {motivo}")
            }
        }
    }
}

impl std::error::Error for ErrorModoRespaldar {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Almacen(e) => Some(e),
            Self::Canal(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ErrorDeAlmacen> for ErrorModoRespaldar {
    fn from(e: ErrorDeAlmacen) -> Self {
        Self::Almacen(e)
    }
}

impl From<ErrorCanalWhatsmeow> for ErrorModoRespaldar {
    fn from(e: ErrorCanalWhatsmeow) -> Self {
        Self::Canal(e)
    }
}

/// Analiza los argumentos CLI para extraer y validar la ruta absoluta de destino.
pub fn analizar_argumentos(argumentos: &[String]) -> Result<PathBuf, ErrorModoRespaldar> {
    let mut directorio = None;
    let mut i = 0;
    while i < argumentos.len() {
        match argumentos[i].as_str() {
            "--directorio" => {
                if i + 1 < argumentos.len() {
                    directorio = Some(argumentos[i + 1].clone());
                    i += 2;
                } else {
                    return Err(ErrorModoRespaldar::FaltaDirectorio);
                }
            }
            arg if arg.starts_with("--directorio=") => {
                let valor = arg.trim_start_matches("--directorio=");
                if valor.is_empty() {
                    return Err(ErrorModoRespaldar::FaltaDirectorio);
                }
                directorio = Some(valor.to_string());
                i += 1;
            }
            arg => {
                return Err(ErrorModoRespaldar::ArgumentoDesconocido(arg.to_string()));
            }
        }
    }

    let ruta_str = directorio.ok_or(ErrorModoRespaldar::FaltaDirectorio)?;
    let ruta = PathBuf::from(ruta_str);
    if !ruta.is_absolute() {
        return Err(ErrorModoRespaldar::DirectorioRelativo);
    }
    Ok(ruta)
}

fn generar_identificador_de_ronda() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("ronda-{nanos}")
}

/// Orquesta la comprobación previa de los 5 destinos y el respaldo de las 5 bases.
pub async fn ejecutar(
    ruta_socket: &Path,
    id_celula: &str,
    ruta_datos: &Path,
    directorio: &Path,
    plazo: Duration,
) -> Result<ResumenDeRespaldoCompleto, ErrorModoRespaldar> {
    verificar_destino_disponible(&directorio.join(NOMBRE_DE_ARCHIVO_DE_SESIONES))?;
    verificar_destino_disponible(&directorio.join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO))?;
    verificar_destino_disponible(&directorio.join(NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR))?;
    verificar_destino_disponible(&directorio.join(NOMBRE_CANONICO_SQLSTORE))?;
    verificar_destino_disponible(&directorio.join(NOMBRE_CANONICO_IDENTIDAD))?;

    let identificador_de_ronda = generar_identificador_de_ronda();
    let inicio = tokio::time::Instant::now();

    let (adaptador, _rx) =
        AdaptadorWhatsmeow::nuevo(ruta_socket, id_celula, 8, Retroceso::por_omision());
    adaptador.arrancar();

    esperar_conexion_activa(&adaptador, plazo)
        .await
        .map_err(|_| ErrorModoRespaldar::ConexionNoEstablecida)?;

    let transcurrido = inicio.elapsed();
    let plazo_restante = plazo
        .checked_sub(transcurrido)
        .ok_or(ErrorModoRespaldar::ConexionNoEstablecida)?;

    // Las DOS bases ordenadas por IPC (sqlstore, identidad) se producen ANTES que las tres locales
    // (PAT-038 fail-empty): tras el pre-chequeo de los cinco destinos, si la disciplina de pausa se
    // violara, el destino queda VACÍO en vez de parcial-que-parece-completo.
    let copia_sqlstore = match ordenar_respaldo_sqlstore(
        &adaptador,
        directorio,
        &identificador_de_ronda,
        plazo_restante,
    )
    .await?
    {
        ResultadoRespaldoSqlstore::Completado(copia) => copia,
        ResultadoRespaldoSqlstore::Fallido { motivo } => {
            return Err(ErrorModoRespaldar::SqlstoreFallido { motivo });
        }
    };

    let transcurrido = inicio.elapsed();
    let plazo_restante = plazo
        .checked_sub(transcurrido)
        .ok_or(ErrorModoRespaldar::ConexionNoEstablecida)?;

    let copia_identidad = match ordenar_respaldo_identidad(
        &adaptador,
        directorio,
        &identificador_de_ronda,
        plazo_restante,
    )
    .await?
    {
        ResultadoRespaldoIdentidad::Completado(copia) => copia,
        ResultadoRespaldoIdentidad::Fallido { motivo } => {
            return Err(ErrorModoRespaldar::IdentidadFallido { motivo });
        }
    };

    let pools = GestorDePools::abrir(ruta_datos)?;
    let almacen = AlmacenDeIdentidad::abrir(ruta_datos)?;
    let resumen_local =
        respaldar_celula_con_ronda(&pools, &almacen, directorio, &identificador_de_ronda)?;

    let mut copias = vec![copia_sqlstore, copia_identidad];
    copias.extend(resumen_local.copias);

    Ok(ResumenDeRespaldoCompleto {
        copias,
        identificador_de_ronda,
    })
}

/// Punto de entrada CLI para el subcomando `hexcell respaldar`.
pub async fn ejecutar_cli(argumentos: &[String]) -> ExitCode {
    let directorio = match analizar_argumentos(argumentos) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("hexcell respaldar: {err}");
            return ExitCode::FAILURE;
        }
    };

    let id_celula = match std::env::var(HEXCELL_ID_CELULA) {
        Ok(val) if !val.trim().is_empty() => val,
        _ => {
            eprintln!(
                "hexcell respaldar: falta la variable de entorno obligatoria {HEXCELL_ID_CELULA}"
            );
            return ExitCode::FAILURE;
        }
    };

    let ruta_datos = match std::env::var(HEXCELL_RUTA_DATOS) {
        Ok(val) if !val.trim().is_empty() => PathBuf::from(val),
        _ => {
            eprintln!(
                "hexcell respaldar: falta la variable de entorno obligatoria {HEXCELL_RUTA_DATOS}"
            );
            return ExitCode::FAILURE;
        }
    };

    let ruta_socket_str = std::env::var(HEXCELL_SOCKET_IPC)
        .unwrap_or_else(|_| RUTA_SOCKET_IPC_POR_DEFECTO.to_string());
    let ruta_socket = PathBuf::from(ruta_socket_str);

    let plazo_segundos = match std::env::var(HEXCELL_RESPALDAR_PLAZO_SEGUNDOS) {
        Ok(val) => match val.parse::<u64>() {
            Ok(s) if s > 0 => s,
            _ => {
                eprintln!(
                    "hexcell respaldar: {HEXCELL_RESPALDAR_PLAZO_SEGUNDOS} debe ser un entero positivo de segundos"
                );
                return ExitCode::FAILURE;
            }
        },
        Err(_) => PLAZO_RESPALDAR_POR_DEFECTO_SEGUNDOS,
    };
    let plazo = Duration::from_secs(plazo_segundos);

    println!(
        "hexcell respaldar: iniciando respaldo de célula «{id_celula}» en «{}»...",
        directorio.display()
    );
    println!(
        "hexcell respaldar: disciplina operacional: el núcleo de la célula debe estar DETENIDO y el sidecar EN EJECUCIÓN."
    );

    match ejecutar(&ruta_socket, &id_celula, &ruta_datos, &directorio, plazo).await {
        Ok(resumen) => {
            for copia in &resumen.copias {
                println!(
                    "hexcell respaldar: copia ok: {} -> {} ({} bytes)",
                    copia.nombre_logico,
                    copia.ruta.display(),
                    copia.bytes
                );
            }
            let bytes_totales: u64 = resumen.copias.iter().map(|c| c.bytes).sum();
            println!(
                "hexcell respaldar: respaldo completado exitosamente ({} copias, {bytes_totales} bytes totales, ronda «{}»).",
                resumen.copias.len(),
                resumen.identificador_de_ronda
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("hexcell respaldar: error al ejecutar el respaldo: {err}");
            eprintln!(
                "hexcell respaldar: el directorio de destino NO contiene un respaldo válido de la célula."
            );
            eprintln!(
                "hexcell respaldar: para reintentar debe utilizar un directorio NUEVO y sin usar."
            );
            ExitCode::FAILURE
        }
    }
}
