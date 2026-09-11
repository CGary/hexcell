//! Cliente del demonio de Docker: las cinco operaciones de esta tarea.
//!
//! [`ClienteDocker`] traduce cada operación a una o dos llamadas HTTP/1.1 contra el socket Unix,
//! usando [`super::transporte::ConexionDocker`], y despacha el código de estado de forma explícita:
//! 200/201/204/304 tienen forma de éxito, 404 es [`ErrorDeClienteDocker::NoEncontrado`], 409 es
//! [`ErrorDeClienteDocker::Conflicto`] y cualquier otro código (5xx incluido) cae en
//! [`ErrorDeClienteDocker::ErrorDelDaemon`] en vez de ignorarse.

use std::path::PathBuf;
use std::time::Duration;

use super::error::ErrorDeClienteDocker;
use super::transporte::{ConexionDocker, RespuestaHttp};

/// Segundos de gracia que se piden al demonio antes de que pueda escalar a `SIGKILL`.
///
/// Es el contrato de apagado del PRD («SIGTERM Docker Container, 30-second grace»): la parada usa
/// el mecanismo nativo de la API del motor (el parámetro `t`), **nunca** un bucle de
/// `std::thread::sleep` seguido de una llamada a matar en el cliente.
const SEGUNDOS_DE_GRACIA: u32 = 30;

/// Resultado de `crear_e_iniciar_contenedor`.
///
/// Dos variantes y ambas llevan el identificador del contenedor: el arranque normal y el caso en
/// que el contenedor ya estaba en ejecución (el demonio responde 304 al arranque). La variante es
/// lo que distingue un desenlace del otro; el identificador viene siempre del cuerpo de la
/// respuesta 201 de `/containers/create` (el motor siempre devuelve `Id` ahí).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultadoDeArranque {
    /// El contenedor se creó y arrancó ahora.
    Iniciado {
        /// Identificador del contenedor, leído del cuerpo 201 de creación.
        id_contenedor: String,
    },
    /// El contenedor ya estaba en ejecución (el demonio respondió 304 al arranque).
    YaEnEjecucion {
        /// Identificador del contenedor, leído del cuerpo 201 de creación.
        id_contenedor: String,
    },
}

/// Cliente del demonio de Docker sobre su socket Unix.
pub struct ClienteDocker {
    ruta_socket: PathBuf,
    tiempo_limite: Duration,
}

impl ClienteDocker {
    /// Construye un cliente para el socket Unix en `ruta_socket`, con un tiempo límite por omisión.
    pub fn nuevo(ruta_socket: PathBuf) -> Self {
        Self {
            ruta_socket,
            tiempo_limite: Duration::from_secs(30),
        }
    }

    /// Construye un cliente con un tiempo límite explícito, para que los tests puedan acortarlo.
    pub fn con_tiempo_limite(ruta_socket: PathBuf, tiempo_limite: Duration) -> Self {
        Self {
            ruta_socket,
            tiempo_limite,
        }
    }

    /// Crea un contenedor con la imagen dada y lo arranca, devolviendo el identificador.
    ///
    /// Son dos llamadas: `POST /containers/create` (cuerpo 201 con `Id`) y
    /// `POST /containers/{id}/start`. Un 304 en el arranque significa que ya estaba en ejecución y
    /// se devuelve [`ResultadoDeArranque::YaEnEjecucion`] con el mismo identificador.
    pub fn crear_e_iniciar_contenedor(
        &self,
        imagen: &str,
    ) -> Result<ResultadoDeArranque, ErrorDeClienteDocker> {
        let cuerpo = serde_json::json!({ "Image": imagen }).to_string();

        let respuesta_de_creacion = {
            let mut conexion = self.conectar()?;
            conexion.enviar("POST", "/containers/create", Some(&cuerpo))?
        };
        let id_contenedor = extraer_id_de_creacion(&respuesta_de_creacion)?;

        let ruta_de_arranque = format!("/containers/{id_contenedor}/start");
        let respuesta_de_arranque = {
            let mut conexion = self.conectar()?;
            conexion.enviar("POST", &ruta_de_arranque, None)?
        };

        match respuesta_de_arranque.estado {
            204 => Ok(ResultadoDeArranque::Iniciado { id_contenedor }),
            304 => Ok(ResultadoDeArranque::YaEnEjecucion { id_contenedor }),
            _ => Err(clasificar_estado(&respuesta_de_arranque)),
        }
    }

    /// Detiene un contenedor pidiendo al demonio un margen de gracia de 30 segundos (`t=30`).
    pub fn detener_contenedor(&self, id: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/stop?t={SEGUNDOS_DE_GRACIA}");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("POST", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    /// Inspecciona un contenedor y devuelve el cuerpo JSON interpretado.
    pub fn inspeccionar_contenedor(
        &self,
        id: &str,
    ) -> Result<serde_json::Value, ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/json");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("GET", &ruta, None)?;
        comprobar_exito(&respuesta)?;
        serde_json::from_slice(&respuesta.cuerpo).map_err(|_| {
            ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "el cuerpo de la inspección no es JSON válido".to_string(),
            }
        })
    }

    /// Elimina un contenedor.
    pub fn eliminar_contenedor(&self, id: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("DELETE", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    /// Elimina un volumen por su nombre.
    pub fn eliminar_volumen(&self, nombre: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/volumes/{nombre}");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("DELETE", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    fn conectar(&self) -> Result<ConexionDocker, ErrorDeClienteDocker> {
        ConexionDocker::conectar_con_tiempo_limite(&self.ruta_socket, self.tiempo_limite)
    }
}

/// Da por buenos los códigos con forma de éxito (200/201/204/304) y clasifica el resto.
fn comprobar_exito(respuesta: &RespuestaHttp) -> Result<(), ErrorDeClienteDocker> {
    match respuesta.estado {
        200 | 201 | 204 | 304 => Ok(()),
        _ => Err(clasificar_estado(respuesta)),
    }
}

/// Despacho explícito del código de estado: 404 y 409 tienen variante propia; todo lo demás
/// (5xx incluido, o un código inesperado) se clasifica como error del demonio con su código.
fn clasificar_estado(respuesta: &RespuestaHttp) -> ErrorDeClienteDocker {
    match respuesta.estado {
        404 => ErrorDeClienteDocker::NoEncontrado,
        409 => ErrorDeClienteDocker::Conflicto,
        estado => ErrorDeClienteDocker::ErrorDelDaemon {
            estado,
            cuerpo: String::from_utf8_lossy(&respuesta.cuerpo).into_owned(),
        },
    }
}

/// Lee el campo `Id` del cuerpo 201 de `/containers/create`; cualquier otro estado se clasifica.
fn extraer_id_de_creacion(respuesta: &RespuestaHttp) -> Result<String, ErrorDeClienteDocker> {
    if respuesta.estado != 201 {
        return Err(clasificar_estado(respuesta));
    }
    let valor: serde_json::Value = serde_json::from_slice(&respuesta.cuerpo).map_err(|_| {
        ErrorDeClienteDocker::RespuestaMalformada {
            motivo: "el cuerpo de creación no es JSON válido".to_string(),
        }
    })?;
    valor
        .get("Id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ErrorDeClienteDocker::RespuestaMalformada {
            motivo: "el cuerpo de creación no lleva el campo Id".to_string(),
        })
}
