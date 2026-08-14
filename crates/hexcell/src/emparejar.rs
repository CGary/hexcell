//! Servicio de aplicación para el modo de emparejamiento del operador.
//!
//! Conecta al socket IPC del sidecar, orquesta la secuencia de mensajes de emparejamiento
//! (`orden_emparejar` -> flujo de `codigo_emparejamiento` -> `acuse_emparejamiento`),
//! y traduce el resultado para el binario y para el operador.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use hexcell_canal_whatsmeow::adaptador::AdaptadorWhatsmeow;
use hexcell_canal_whatsmeow::error::ErrorCanalWhatsmeow;
use hexcell_canal_whatsmeow::mensajes::CodigoEmparejamiento;
use hexcell_canal_whatsmeow::reconexion::Retroceso;
use hexcell_core::canal::EstadoSesion;

/// Nombre de la variable de entorno para la ruta del socket IPC.
pub const HEXCELL_SOCKET_IPC: &str = "HEXCELL_SOCKET_IPC";
/// Ruta por omisión del socket IPC documentada en el protocolo.
pub const RUTA_SOCKET_IPC_POR_DEFECTO: &str = "/var/lib/hexcell/ipc/sidecar.sock";
/// Plazo por omisión en segundos para el modo de emparejamiento.
pub const PLAZO_EMPAREJAR_POR_DEFECTO_SEGUNDOS: u64 = 120;
/// Variable opcional para sobreescribir el plazo en segundos.
pub const HEXCELL_EMPAREJAR_PLAZO_SEGUNDOS: &str = "HEXCELL_EMPAREJAR_PLAZO_SEGUNDOS";

/// Resultado del proceso de emparejamiento con el canal whatsmeow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResultadoEmparejamiento {
    /// El emparejamiento concluyó exitosamente con una sesión activa.
    Completado,
    /// El emparejamiento expiró antes de completarse la vinculación.
    Expirado,
    /// El emparejamiento falló con un motivo descriptivo.
    Fallido {
        /// Descripción del motivo del fallo (sin credenciales).
        motivo: String,
    },
}

/// Errores durante la ejecución del modo de emparejamiento.
#[derive(Debug)]
pub enum ErrorModoEmparejar {
    /// El método de emparejamiento especificado no es válido.
    MetodoInvalido(String),
    /// No se pudo establecer conexión activa con el sidecar dentro del plazo.
    ConexionNoEstablecida,
    /// Error proveniente de la capa de transporte o canal whatsmeow.
    Canal(ErrorCanalWhatsmeow),
}

impl fmt::Display for ErrorModoEmparejar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MetodoInvalido(m) => write!(
                f,
                "método de emparejamiento inválido: «{m}» (debe ser 'codigo_de_vinculacion' o 'qr')"
            ),
            Self::ConexionNoEstablecida => write!(
                f,
                "no se pudo establecer conexión activa con el sidecar IPC dentro del plazo"
            ),
            Self::Canal(e) => write!(f, "error en canal whatsmeow: {e}"),
        }
    }
}

impl std::error::Error for ErrorModoEmparejar {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Canal(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ErrorCanalWhatsmeow> for ErrorModoEmparejar {
    fn from(e: ErrorCanalWhatsmeow) -> Self {
        Self::Canal(e)
    }
}

/// Espera de manera orientada a eventos a que el adaptador establezca una conexión activa con el sidecar.
pub async fn esperar_conexion_activa(
    adaptador: &AdaptadorWhatsmeow,
    plazo: Duration,
) -> Result<(), ErrorModoEmparejar> {
    if adaptador.estado_actual() == EstadoSesion::Activa {
        return Ok(());
    }

    let mut receptor = adaptador.suscribir_estado();
    if *receptor.borrow() == EstadoSesion::Activa {
        return Ok(());
    }

    let espera = async {
        while receptor.changed().await.is_ok() {
            if *receptor.borrow() == EstadoSesion::Activa {
                return Ok(());
            }
        }
        Err(ErrorModoEmparejar::ConexionNoEstablecida)
    };

    tokio::time::timeout(plazo, espera)
        .await
        .map_err(|_| ErrorModoEmparejar::ConexionNoEstablecida)?
}

/// Ordena el emparejamiento a través del adaptador y traduce el resultado al dominio de la aplicación.
pub async fn ordenar_emparejamiento(
    adaptador: &AdaptadorWhatsmeow,
    metodo: &str,
    plazo: Duration,
    manejador: impl FnMut(&CodigoEmparejamiento) + Send,
) -> Result<ResultadoEmparejamiento, ErrorModoEmparejar> {
    if metodo != "qr" && metodo != "codigo_de_vinculacion" {
        return Err(ErrorModoEmparejar::MetodoInvalido(metodo.to_string()));
    }

    let acuse = adaptador
        .ordenar_emparejamiento(metodo, plazo, manejador)
        .await?;

    let resultado = match acuse.resultado.as_str() {
        "completado" => ResultadoEmparejamiento::Completado,
        "expirado" => ResultadoEmparejamiento::Expirado,
        "fallido" => ResultadoEmparejamiento::Fallido {
            motivo: acuse.motivo,
        },
        otro => ResultadoEmparejamiento::Fallido {
            motivo: format!("resultado desconocido en acuse: {otro}"),
        },
    };

    Ok(resultado)
}

/// Orquesta el flujo completo de emparejamiento creando un adaptador efímero sobre la ruta de socket indicada.
pub async fn ejecutar(
    ruta_socket: &Path,
    id_celula: &str,
    metodo: &str,
    plazo: Duration,
    manejador: impl FnMut(&CodigoEmparejamiento) + Send,
) -> Result<ResultadoEmparejamiento, ErrorModoEmparejar> {
    let inicio = tokio::time::Instant::now();
    let (adaptador, _rx) =
        AdaptadorWhatsmeow::nuevo(ruta_socket, id_celula, 8, Retroceso::por_omision());
    adaptador.arrancar();

    esperar_conexion_activa(&adaptador, plazo).await?;

    let tiempo_transcurrido = inicio.elapsed();
    let plazo_restante = plazo
        .checked_sub(tiempo_transcurrido)
        .ok_or(ErrorModoEmparejar::ConexionNoEstablecida)?;

    ordenar_emparejamiento(&adaptador, metodo, plazo_restante, manejador).await
}

/// Punto de entrada CLI para el subcomando `hexcell emparejar`.
pub async fn ejecutar_cli(argumentos: &[String]) -> ExitCode {
    let mut metodo = "codigo_de_vinculacion".to_string();
    let mut i = 0;
    while i < argumentos.len() {
        match argumentos[i].as_str() {
            "--metodo" => {
                if i + 1 < argumentos.len() {
                    metodo = argumentos[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("hexcell emparejar: falta el valor para --metodo");
                    return ExitCode::FAILURE;
                }
            }
            arg if arg.starts_with("--metodo=") => {
                let valor = arg.trim_start_matches("--metodo=");
                if valor.is_empty() {
                    eprintln!("hexcell emparejar: falta el valor para --metodo");
                    return ExitCode::FAILURE;
                }
                metodo = valor.to_string();
                i += 1;
            }
            arg => {
                eprintln!("hexcell emparejar: argumento desconocido: «{arg}»");
                return ExitCode::FAILURE;
            }
        }
    }

    if metodo != "codigo_de_vinculacion" && metodo != "qr" {
        eprintln!(
            "hexcell emparejar: método de emparejamiento no reconocido: «{metodo}» (debe ser 'codigo_de_vinculacion' o 'qr')"
        );
        return ExitCode::FAILURE;
    }

    let id_celula = match std::env::var(crate::configuracion::HEXCELL_ID_CELULA) {
        Ok(val) if !val.trim().is_empty() => val,
        _ => {
            eprintln!(
                "hexcell emparejar: falta la variable de entorno obligatoria {}",
                crate::configuracion::HEXCELL_ID_CELULA
            );
            return ExitCode::FAILURE;
        }
    };

    let ruta_socket_str = std::env::var(HEXCELL_SOCKET_IPC)
        .unwrap_or_else(|_| RUTA_SOCKET_IPC_POR_DEFECTO.to_string());
    let ruta_socket = PathBuf::from(ruta_socket_str);

    let plazo_segundos = match std::env::var(HEXCELL_EMPAREJAR_PLAZO_SEGUNDOS) {
        Ok(val) => match val.parse::<u64>() {
            Ok(s) if s > 0 => s,
            _ => {
                eprintln!(
                    "hexcell emparejar: {} debe ser un entero positivo de segundos",
                    HEXCELL_EMPAREJAR_PLAZO_SEGUNDOS
                );
                return ExitCode::FAILURE;
            }
        },
        Err(_) => PLAZO_EMPAREJAR_POR_DEFECTO_SEGUNDOS,
    };
    let plazo = Duration::from_secs(plazo_segundos);

    println!(
        "hexcell emparejar: iniciando emparejamiento con método «{metodo}» (célula: {id_celula}, plazo: {plazo_segundos}s)..."
    );

    let manejador = |codigo: &CodigoEmparejamiento| {
        if codigo.metodo == "qr" {
            println!("Código QR recibido (cadena cruda): {}", codigo.valor);
            println!(
                "Nota: el renderizado gráfico no está integrado; puede visualizar esta cadena con un renderizador QR externo."
            );
        } else {
            println!("Código de vinculación: {}", codigo.valor);
        }
        if codigo.expira_en_ms > 0 {
            println!(
                "Expiración declarada (milisegundos Unix): {}",
                codigo.expira_en_ms
            );
        } else {
            println!("Expiración: desconocida");
        }
    };

    match ejecutar(&ruta_socket, &id_celula, &metodo, plazo, manejador).await {
        Ok(ResultadoEmparejamiento::Completado) => {
            println!("hexcell emparejar: emparejamiento completado exitosamente.");
            ExitCode::SUCCESS
        }
        Ok(ResultadoEmparejamiento::Expirado) => {
            eprintln!("hexcell emparejar: el emparejamiento ha expirado.");
            ExitCode::FAILURE
        }
        Ok(ResultadoEmparejamiento::Fallido { motivo }) => {
            eprintln!("hexcell emparejar: emparejamiento fallido: {motivo}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("hexcell emparejar: error al ejecutar: {err}");
            ExitCode::FAILURE
        }
    }
}
