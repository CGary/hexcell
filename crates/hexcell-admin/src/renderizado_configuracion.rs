//! Parser y combinador puro de archivos KEY=VALUE.

use crate::esquema_configuracion::validar_clave;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub enum ErrorDeRenderizado {
    LineaInvalida { linea: usize },
    ClaveInvalida { clave: String, razon: String },
}

impl std::fmt::Display for ErrorDeRenderizado {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineaInvalida { linea } => write!(f, "línea inválida en configuración: {linea}"),
            Self::ClaveInvalida { clave, razon } => {
                write!(f, "configuración inválida para «{clave}»: {razon}")
            }
        }
    }
}
impl std::error::Error for ErrorDeRenderizado {}

pub fn analizar_env(texto: &str) -> Result<BTreeMap<String, String>, ErrorDeRenderizado> {
    let mut resultado = BTreeMap::new();
    for (indice, cruda) in texto.lines().enumerate() {
        let linea = cruda.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        let (clave, valor) = linea
            .split_once('=')
            .ok_or(ErrorDeRenderizado::LineaInvalida { linea: indice + 1 })?;
        if clave.is_empty() || valor.contains('\n') {
            return Err(ErrorDeRenderizado::LineaInvalida { linea: indice + 1 });
        }
        validar_clave(clave, valor).map_err(|razon| ErrorDeRenderizado::ClaveInvalida {
            clave: clave.to_string(),
            razon,
        })?;
        resultado.insert(clave.to_string(), valor.to_string());
    }
    Ok(resultado)
}

pub fn combinar(
    mut defecto: BTreeMap<String, String>,
    superposicion: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, ErrorDeRenderizado> {
    for (clave, valor) in &defecto {
        validar_clave(clave, valor).map_err(|razon| ErrorDeRenderizado::ClaveInvalida {
            clave: clave.clone(),
            razon,
        })?;
    }
    for (clave, valor) in &superposicion {
        validar_clave(clave, valor).map_err(|razon| ErrorDeRenderizado::ClaveInvalida {
            clave: clave.clone(),
            razon,
        })?;
    }
    defecto.extend(superposicion);
    Ok(defecto)
}

pub fn serializar(valores: &BTreeMap<String, String>) -> String {
    valores.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}
