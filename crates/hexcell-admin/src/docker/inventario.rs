//! Inventario de contenedores del demonio de Docker.
//!
//! Tarea 14 de la etapa A-6 (HEX-083): emite `GET /containers/json?all=true` sobre el
//! transporte público de `docker::transporte` y devuelve un resumen de cada contenedor con
//! su nombre y estado. Vive en su propio archivo porque `docker/cliente.rs` está fuera del
//! alcance de esta tarea y `ClienteDocker` no expone ningún método de listado ni ningún
//! acceso a su `ruta_socket` privada.

use std::path::PathBuf;
use std::time::Duration;

use super::error::ErrorDeClienteDocker;
use super::transporte::ConexionDocker;

/// Resumen de un contenedor leído del listado de Docker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenDeContenedor {
    /// Nombre del contenedor, sin la barra inicial que Docker añade.
    pub nombre: String,
    /// Estado del contenedor tal y como lo reporta Docker (`running`, `exited`, …).
    pub estado: String,
}

/// Cliente reducido al listado de contenedores.
///
/// Tiene sus propios campos de socket y tiempo límite porque `ClienteDocker` no los expone.
/// No duplica ningún método de `ClienteDocker`: sólo emite `GET /containers/json`.
pub struct InventarioDocker {
    ruta_socket: PathBuf,
    tiempo_limite: Duration,
}

impl InventarioDocker {
    /// Construye un inventario para el socket Unix en `ruta_socket`.
    pub fn nuevo(ruta_socket: PathBuf, tiempo_limite: Duration) -> Self {
        Self {
            ruta_socket,
            tiempo_limite,
        }
    }

    /// Lista todos los contenedores conocidos por el demonio, incluidos los detenidos.
    pub fn listar_contenedores(&self) -> Result<Vec<ResumenDeContenedor>, ErrorDeClienteDocker> {
        let mut conexion =
            ConexionDocker::conectar_con_tiempo_limite(&self.ruta_socket, self.tiempo_limite)?;
        let respuesta = conexion.enviar("GET", "/containers/json?all=true", None)?;
        match respuesta.estado {
            200 => {}
            estado => {
                return Err(ErrorDeClienteDocker::ErrorDelDaemon {
                    estado,
                    cuerpo: String::from_utf8_lossy(&respuesta.cuerpo).into_owned(),
                });
            }
        }
        let valor: serde_json::Value = serde_json::from_slice(&respuesta.cuerpo).map_err(|_| {
            ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "el cuerpo del listado no es JSON válido".to_string(),
            }
        })?;
        let arreglo =
            valor
                .as_array()
                .ok_or_else(|| ErrorDeClienteDocker::RespuestaMalformada {
                    motivo: "el cuerpo del listado no es un arreglo JSON".to_string(),
                })?;
        let mut resultado = Vec::new();
        for entrada in arreglo {
            let estado = entrada
                .get("State")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let nombre = entrada
                .get("Names")
                .and_then(|v| v.as_array())
                .and_then(|n| n.first())
                .and_then(|v| v.as_str())
                .map(|s| s.strip_prefix('/').unwrap_or(s))
                .unwrap_or("")
                .to_string();
            if !nombre.is_empty() {
                resultado.push(ResumenDeContenedor { nombre, estado });
            }
        }
        Ok(resultado)
    }
}
