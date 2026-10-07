//! Servicio de `contacto restablecer` mediante la sonda HTTP hermana.

use std::io::Write;

use crate::argumentos::InvocacionContacto;
use crate::ciclo_de_vida::{self, DatosDeSondeo, ErrorDeCicloDeVida, NombresDeCelula};
use crate::codigo_de_salida::CodigoDeSalida;
use crate::docker::{ClienteDocker, ErrorDeClienteDocker};
use crate::salida::Salida;

pub fn validar_id_de_contacto(contacto: &str) -> bool {
    let bytes = contacto.as_bytes();
    bytes.len() == 35
        && bytes.starts_with(b"ct-")
        && bytes[3..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

#[derive(Debug, PartialEq, Eq)]
pub enum DesenlaceDeRestablecimientoDeContacto {
    Aplicado {
        existe: bool,
        cortacircuitos: i64,
        presentacion_de_conversacion: i64,
        baja_de_contacto: i64,
    },
    ContactoDesconocido,
    CanalSinSesion,
    Fallido(String),
}

pub fn desenlace_de_restablecimiento(
    cuerpo: &[u8],
) -> Result<DesenlaceDeRestablecimientoDeContacto, ErrorDeCicloDeVida> {
    let valor: serde_json::Value =
        serde_json::from_slice(cuerpo).map_err(|_| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de contacto no es JSON válido".into(),
        })?;
    let resultado = valor
        .get("resultado")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: "la respuesta de contacto no lleva resultado".into(),
        })?;
    match resultado {
        "aplicado" => {
            let existe = valor
                .get("existe")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
                    motivo: "la respuesta de contacto no lleva existe booleano".into(),
                })?;
            if !existe {
                return Ok(DesenlaceDeRestablecimientoDeContacto::ContactoDesconocido);
            }
            let numero = |nombre: &str| {
                valor
                    .get(nombre)
                    .and_then(|v| v.as_i64())
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
                        motivo: format!("la respuesta de contacto no lleva {nombre} válido"),
                    })
            };
            Ok(DesenlaceDeRestablecimientoDeContacto::Aplicado {
                existe,
                cortacircuitos: numero("cortacircuitos")?,
                presentacion_de_conversacion: numero("presentacion_de_conversacion")?,
                baja_de_contacto: numero("baja_de_contacto")?,
            })
        }
        "contacto_desconocido" => Ok(DesenlaceDeRestablecimientoDeContacto::ContactoDesconocido),
        "canal_sin_sesion" => Ok(DesenlaceDeRestablecimientoDeContacto::CanalSinSesion),
        "fallido" => Ok(DesenlaceDeRestablecimientoDeContacto::Fallido(
            valor
                .get("motivo")
                .and_then(|v| v.as_str())
                .unwrap_or("fallido")
                .to_string(),
        )),
        otro => Err(ErrorDeCicloDeVida::CuerpoDeSondaIlegible {
            motivo: format!("resultado de contacto inesperado: {otro}"),
        }),
    }
}

pub fn linea_de_simulacion_de_contacto(invocacion: &InvocacionContacto) -> String {
    let baja = if invocacion.incluir_baja() {
        "incluida"
    } else {
        "no tocada"
    };
    format!(
        "simulación: contacto restablecer --id {} --contacto {} -> cortacircuitos, presentacion_de_conversacion; baja_de_contacto: {baja}",
        invocacion.id(),
        invocacion.contacto()
    )
}

pub fn ejecutar_contacto<S: Write, D: Write>(
    invocacion: InvocacionContacto,
    salida: &mut Salida<S, D>,
    cliente: &ClienteDocker,
    datos: &DatosDeSondeo,
) -> CodigoDeSalida {
    let nombres = NombresDeCelula::nueva(invocacion.id());
    let inspeccion = match cliente.inspeccionar_contenedor(&nombres.nucleo) {
        Ok(v) => v,
        Err(ErrorDeClienteDocker::NoEncontrado) => return fallo(salida, "célula no encontrada"),
        Err(e) => return fallo(salida, &e.to_string()),
    };
    let estado = inspeccion.pointer("/State/Status").and_then(|v| v.as_str());
    if estado != Some("running") {
        return fallo(salida, "el núcleo de la célula no está en ejecución");
    }
    let datos_celula = match ciclo_de_vida::resolver_datos_de_celula_para_rebind(cliente, &nombres)
    {
        Ok(v) => v,
        Err(e) => return fallo(salida, &e.to_string()),
    };
    if invocacion.incluir_baja()
        && salida
            .diagnostico("ADVERTENCIA: se revivirá la baja de contacto (--confirmar)")
            .is_err()
    {
        return CodigoDeSalida::Fallo;
    }
    let cuerpo = serde_json::json!({ "contacto": invocacion.contacto(), "incluir_baja": invocacion.incluir_baja() }).to_string();
    let url = format!(
        "http://{}:{}/admin/contacto/restablecer",
        nombres.nucleo, datos_celula.puerto_admin
    );
    let respuesta = match ciclo_de_vida::consultar_por_hermano(
        cliente,
        &nombres,
        &datos.imagen,
        ciclo_de_vida::guion_de_peticion_http(&url, Some(&cuerpo), datos.limite_segundos),
        &datos_celula.red,
    ) {
        Ok(v) => v,
        Err(e) => return fallo(salida, &e.to_string()),
    };
    match desenlace_de_restablecimiento(&respuesta) {
        Ok(DesenlaceDeRestablecimientoDeContacto::Aplicado {
            existe: true,
            cortacircuitos,
            presentacion_de_conversacion,
            baja_de_contacto,
        }) => {
            if salida
                .linea(&format!("cortacircuitos: {cortacircuitos}"))
                .is_err()
                || salida
                    .linea(&format!(
                        "presentacion_de_conversacion: {presentacion_de_conversacion}"
                    ))
                    .is_err()
            {
                return CodigoDeSalida::Fallo;
            }
            if invocacion.incluir_baja() {
                if salida
                    .linea(&format!("baja_de_contacto: {baja_de_contacto}"))
                    .is_err()
                {
                    return CodigoDeSalida::Fallo;
                }
            } else if salida.linea("baja_de_contacto: no tocada").is_err() {
                return CodigoDeSalida::Fallo;
            }
            if cortacircuitos == 0
                && presentacion_de_conversacion == 0
                && (!invocacion.incluir_baja() || baja_de_contacto == 0)
            {
                let _ = salida.linea("sin cambios");
            }
            CodigoDeSalida::Exito
        }
        Ok(DesenlaceDeRestablecimientoDeContacto::ContactoDesconocido) => {
            fallo(salida, "contacto_desconocido")
        }
        Ok(DesenlaceDeRestablecimientoDeContacto::CanalSinSesion) => {
            fallo(salida, "canal_sin_sesion")
        }
        Ok(DesenlaceDeRestablecimientoDeContacto::Fallido(motivo)) => fallo(salida, &motivo),
        Ok(DesenlaceDeRestablecimientoDeContacto::Aplicado { existe: false, .. }) => {
            fallo(salida, "contacto_desconocido")
        }
        Err(e) => fallo(salida, &e.to_string()),
    }
}

fn fallo<S: Write, D: Write>(salida: &mut Salida<S, D>, mensaje: &str) -> CodigoDeSalida {
    let _ = salida.diagnostico(mensaje);
    CodigoDeSalida::Fallo
}
