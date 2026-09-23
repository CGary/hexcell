//! Cliente del demonio de Docker: las cinco operaciones de HEX-074-b más las que añadió la
//! orquestación de `cell pause`/`cell unpause`.
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

/// Opciones de creación de un contenedor auxiliar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcionesDeContenedor {
    /// Red Docker a la que se conecta el contenedor.
    pub red: String,
    /// Comando y argumentos que ejecuta el contenedor.
    pub cmd: Vec<String>,
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
    ///
    /// **Reemplazada** por [`Self::detener_contenedor_sin_plazo`] desde HEX-080 (2026-09-21): tras
    /// esa tarea no le queda ningún llamador en `src/`. Se conserva intacta, junto con su prueba,
    /// porque es API que entregó HEX-074-b. Seguimiento de la tarea 15, que toca el cliente por
    /// derecho propio: fundir ambas en una sola operación con plazo opcional y mover la prueba.
    pub fn detener_contenedor(&self, id: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/stop?t={SEGUNDOS_DE_GRACIA}");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("POST", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    /// Detiene un contenedor **sin** fijar ningún plazo desde la CLI.
    ///
    /// La petición sale como `POST /containers/{id}/stop`, sin el parámetro `t`, de modo que el
    /// plazo de gracia lo decide una sola fuente: el `stop_grace_period` que la plantilla de
    /// célula declara para cada contenedor. Es la operación que usa `cell pause` para los dos
    /// contenedores, sidecar incluido: el sidecar también se detiene CON gracia, porque tiene que
    /// cerrar su websocket saliente y dejar su almacén consistente.
    ///
    /// [`Self::detener_contenedor`] se conserva intacta, con su `t=30`, porque es la operación que
    /// entregó HEX-074-b y su prueba fija la ruta exacta.
    pub fn detener_contenedor_sin_plazo(&self, id: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/stop");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("POST", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    /// Inicia un contenedor que ya existe.
    pub fn iniciar_contenedor(&self, id: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/start");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("POST", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    /// Crea e inicia un contenedor con su red y comando explícitos.
    ///
    /// Si la creación devuelve 201 pero el arranque falla, el contenedor YA existe en el demonio:
    /// antes de propagar el error del arranque se emite su `DELETE` en el mejor esfuerzo, para que
    /// ningún camino de fallo deje una sonda huérfana. El error que se devuelve sigue siendo el
    /// del arranque, nunca el de esa limpieza.
    pub fn crear_e_iniciar_contenedor_con_opciones(
        &self,
        imagen: &str,
        opciones: OpcionesDeContenedor,
    ) -> Result<ResultadoDeArranque, ErrorDeClienteDocker> {
        let cuerpo = serde_json::json!({
            "Image": imagen,
            "HostConfig": { "NetworkMode": opciones.red },
            "Cmd": opciones.cmd,
        })
        .to_string();
        let respuesta_de_creacion = {
            let mut conexion = self.conectar()?;
            conexion.enviar("POST", "/containers/create", Some(&cuerpo))?
        };
        let id_contenedor = extraer_id_de_creacion(&respuesta_de_creacion)?;
        let ruta = format!("/containers/{id_contenedor}/start");
        let respuesta = match self
            .conectar()
            .and_then(|mut conexion| conexion.enviar("POST", &ruta, None))
        {
            Ok(respuesta) => respuesta,
            Err(error) => {
                let _ = self.eliminar_contenedor(&id_contenedor);
                return Err(error);
            }
        };
        match respuesta.estado {
            204 => Ok(ResultadoDeArranque::Iniciado { id_contenedor }),
            304 => Ok(ResultadoDeArranque::YaEnEjecucion { id_contenedor }),
            _ => {
                let error = clasificar_estado(&respuesta);
                let _ = self.eliminar_contenedor(&id_contenedor);
                Err(error)
            }
        }
    }

    /// Espera a que Docker termine el contenedor y devuelve su código de salida.
    pub fn esperar_contenedor(&self, id: &str) -> Result<i64, ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/wait");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("POST", &ruta, None)?;
        comprobar_exito(&respuesta)?;
        let valor: serde_json::Value = serde_json::from_slice(&respuesta.cuerpo).map_err(|_| {
            ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "el cuerpo de espera no es JSON válido".to_string(),
            }
        })?;
        valor
            .get("StatusCode")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "el cuerpo de espera no lleva StatusCode".to_string(),
            })
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

    /// Elimina un volumen por su nombres.
    pub fn eliminar_volumen(&self, nombre: &str) -> Result<(), ErrorDeClienteDocker> {
        let ruta = format!("/volumes/{nombre}");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("DELETE", &ruta, None)?;
        comprobar_exito(&respuesta)
    }

    /// Lee la salida estándar de un contenedor y la devuelve como bytes, demultiplexando el flujo
    /// con encabezado de 8 bytes que devuelve `GET /containers/{id}/logs`.
    ///
    /// El demonio multiplexa `stdout` y `stderr` en un mismo flujo: cada trama va precedida de un
    /// encabezado de 8 bytes (byte de flujo, 3 bytes de relleno, u32 big-endian con la longitud).
    /// Esta función se queda sólo con las tramas de `stdout` (flujo = 1) y descarta las de `stderr`
    /// (flujo = 2). Si el encabezado está truncado o el cuerpo no llega completo, devuelve
    /// [`ErrorDeClienteDocker::RespuestaMalformada`] en vez de entrar en pánico.
    ///
    /// La petición consulta `stdout=1&stderr=0`, pero la demultiplexación se mantiene como defensa
    /// en profundidad: el demonio podría ignorar `stderr=0` y servir ambos flujos.
    pub fn leer_salida_estandar(&self, id: &str) -> Result<Vec<u8>, ErrorDeClienteDocker> {
        let ruta = format!("/containers/{id}/logs?stdout=1&stderr=0");
        let mut conexion = self.conectar()?;
        let respuesta = conexion.enviar("GET", &ruta, None)?;
        comprobar_exito(&respuesta)?;
        demultiplexar_salida_estandar(&respuesta.cuerpo)
    }

    /// Crea e inicia un contenedor montando un volumen de Docker en la ruta de destino indicada,
    /// con la red y el comando de [`OpcionesDeContenedor`].
    ///
    /// Comparte el contrato de creación/limpieza de
    /// [`Self::crear_e_iniciar_contenedor_con_opciones`]: si el arranque falla, el contenedor se
    /// elimina en el mejor esfuerzo. La diferencia es el montaje: `HostConfig.Mounts` lleva una
    /// entrada de tipo `volume` con el nombre del volumen y el punto de destino.
    ///
    /// El contenedor hermano que descarta `sqlstore.db` (tarea 13 de A-6, paso 6 de D5) usa esta
    /// función para montar el volumen de datos y ejecutar `rm -f` sobre los archivos del sidecar.
    pub fn crear_e_iniciar_contenedor_con_volumen(
        &self,
        imagen: &str,
        opciones: OpcionesDeContenedor,
        volumen: &str,
        destino: &str,
    ) -> Result<ResultadoDeArranque, ErrorDeClienteDocker> {
        let cuerpo = serde_json::json!({
            "Image": imagen,
            "HostConfig": {
                "NetworkMode": opciones.red,
                "Mounts": [{ "Type": "volume", "Source": volumen, "Target": destino }],
            },
            "Cmd": opciones.cmd,
        })
        .to_string();
        let respuesta_de_creacion = {
            let mut conexion = self.conectar()?;
            conexion.enviar("POST", "/containers/create", Some(&cuerpo))?
        };
        let id_contenedor = extraer_id_de_creacion(&respuesta_de_creacion)?;
        let ruta = format!("/containers/{id_contenedor}/start");
        let respuesta = match self
            .conectar()
            .and_then(|mut conexion| conexion.enviar("POST", &ruta, None))
        {
            Ok(respuesta) => respuesta,
            Err(error) => {
                let _ = self.eliminar_contenedor(&id_contenedor);
                return Err(error);
            }
        };
        match respuesta.estado {
            204 => Ok(ResultadoDeArranque::Iniciado { id_contenedor }),
            304 => Ok(ResultadoDeArranque::YaEnEjecucion { id_contenedor }),
            _ => {
                let error = clasificar_estado(&respuesta);
                let _ = self.eliminar_contenedor(&id_contenedor);
                Err(error)
            }
        }
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

/// Demultiplexa el flujo de registros de Docker (encabezado de 8 bytes por trama) y concatena
/// sólo las tramas de `stdout` (byte de flujo = 1).
///
/// Formato de cada trama: `[flujo: u8][relleno: 3 bytes][longitud: u32 big-endian][datos: longitud
/// bytes]`. Las tramas de `stderr` (flujo = 2) se descartan. Si el cuerpo termina en mitad de un
/// encabezado o de un cuerpo de trama, devuelve [`ErrorDeClienteDocker::RespuestaMalformada`] sin
/// entrar en pánico.
fn demultiplexar_salida_estandar(cuerpo: &[u8]) -> Result<Vec<u8>, ErrorDeClienteDocker> {
    let mut salida = Vec::new();
    let mut desplazamiento = 0usize;
    while desplazamiento < cuerpo.len() {
        // Encabezado de 8 bytes: necesitamos todos para saber la longitud de la trama.
        if desplazamiento + 8 > cuerpo.len() {
            return Err(ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "encabezado de registro truncado".to_string(),
            });
        }
        let flujo = cuerpo[desplazamiento];
        let longitud = u32::from_be_bytes([
            cuerpo[desplazamiento + 4],
            cuerpo[desplazamiento + 5],
            cuerpo[desplazamiento + 6],
            cuerpo[desplazamiento + 7],
        ]) as usize;
        let inicio_del_cuerpo = desplazamiento + 8;
        let fin_del_cuerpo = inicio_del_cuerpo.checked_add(longitud).ok_or_else(|| {
            ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "longitud de registro desbordada".to_string(),
            }
        })?;
        if fin_del_cuerpo > cuerpo.len() {
            return Err(ErrorDeClienteDocker::RespuestaMalformada {
                motivo: "cuerpo de registro truncado".to_string(),
            });
        }
        if flujo == 1 {
            // stdout.
            salida.extend_from_slice(&cuerpo[inicio_del_cuerpo..fin_del_cuerpo]);
        }
        desplazamiento = fin_del_cuerpo;
    }
    Ok(salida)
}
