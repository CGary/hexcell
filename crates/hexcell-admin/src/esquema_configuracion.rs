//! Esquema cerrado de parámetros no secretos que puede renderizar la CLI.

pub const ESQUEMA_PERMITIDO: &[&str] = &[
    "HEXCELL_ID_CELULA",
    "HEXCELL_RED_CELULA",
    "HEXCELL_VOLUMEN_CELULA",
    "HEXCELL_IMAGEN_NUCLEO",
    "HEXCELL_IMAGEN_SIDECAR",
    "HEXCELL_VENTANA_ZONA",
    "HEXCELL_TELEFONO_CELULA",
    "HEXCELL_NUCLEO_LIMITE_MEMORIA",
    "HEXCELL_NUCLEO_LIMITE_CPUS",
    "HEXCELL_NUCLEO_LIMITE_NOFILE",
    "HEXCELL_SIDECAR_LIMITE_MEMORIA",
    "HEXCELL_SIDECAR_LIMITE_CPUS",
    "HEXCELL_SIDECAR_LIMITE_NOFILE",
    "HEXCELL_TELEGRAM_CHAT_ID",
    "HEXCELL_TELEGRAM_URL_BASE",
    "HEXCELL_TELEGRAM_TIMEOUT_MS",
    "HEXCELL_ALERTAS_VENTANA_RECONEXION_SEGUNDOS",
    "HEXCELL_ALERTAS_SUELO_BALANCE_DISPONIBLE",
    "HEXCELL_ALERTAS_LIMITE_TASA_DESCARTES",
    "HEXCELL_ALERTAS_LIMITE_CAIDA_RATIO_ACUSES",
    "HEXCELL_ALERTAS_MINIMO_ENVIOS_PARA_EVALUAR_ACUSE",
];

pub fn validar_clave(clave: &str, valor: &str) -> Result<(), String> {
    if !ESQUEMA_PERMITIDO.contains(&clave) {
        return Err("clave desconocida".to_string());
    }
    let no_vacio = || {
        if valor.is_empty() {
            Err("valor vacío".to_string())
        } else {
            Ok(())
        }
    };
    match clave {
        "HEXCELL_VENTANA_ZONA" => no_vacio().and_then(|_| {
            if valor.is_ascii() && valor.contains('/') {
                Ok(())
            } else {
                Err("zona horaria inválida".to_string())
            }
        }),
        k if k.ends_with("_LIMITE_MEMORIA") => validar_memoria(valor),
        k if k.ends_with("_LIMITE_CPUS") => decimal_positivo(valor),
        k if k.ends_with("_LIMITE_NOFILE")
            || k.ends_with("_TIMEOUT_MS")
            || k.ends_with("_SEGUNDOS")
            || k.ends_with("_ENVIOS_PARA_EVALUAR_ACUSE") =>
        {
            entero_positivo(valor)
        }
        "HEXCELL_ALERTAS_SUELO_BALANCE_DISPONIBLE" => entero_con_signo(valor),
        "HEXCELL_ALERTAS_LIMITE_TASA_DESCARTES" => decimal_no_negativo(valor),
        "HEXCELL_ALERTAS_LIMITE_CAIDA_RATIO_ACUSES" => valor
            .parse::<f64>()
            .ok()
            .filter(|v| (0.0..=1.0).contains(v))
            .map(|_| ())
            .ok_or_else(|| "debe ser un número entre 0 y 1".to_string()),
        _ => no_vacio().and_then(|_| {
            if valor.chars().any(char::is_whitespace) {
                Err("no puede contener espacios".to_string())
            } else {
                Ok(())
            }
        }),
    }
}

fn entero_positivo(v: &str) -> Result<(), String> {
    v.parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .map(|_| ())
        .ok_or_else(|| "debe ser un entero positivo".to_string())
}
fn decimal_positivo(v: &str) -> Result<(), String> {
    v.parse::<f64>()
        .ok()
        .filter(|n| *n > 0.0 && n.is_finite())
        .map(|_| ())
        .ok_or_else(|| "debe ser un decimal positivo".to_string())
}
fn decimal_no_negativo(v: &str) -> Result<(), String> {
    v.parse::<f64>()
        .ok()
        .filter(|n| *n >= 0.0 && n.is_finite())
        .map(|_| ())
        .ok_or_else(|| "debe ser un decimal no negativo".to_string())
}
fn entero_con_signo(v: &str) -> Result<(), String> {
    v.parse::<i64>()
        .map(|_| ())
        .map_err(|_| "debe ser un entero, p. ej. 0 o -100".to_string())
}
fn validar_memoria(v: &str) -> Result<(), String> {
    // El corte se hace por CARÁCTER, no por índice de byte: `v.len() - 1` cae en medio de un
    // carácter multibyte (p. ej. una tilde) y `split_at` entra en pánico. Ver D-56.
    let mut caracteres = v.chars();
    let sufijo = match caracteres.next_back() {
        Some(c) => c,
        None => return Err("debe ser una cantidad de memoria como 48m".to_string()),
    };
    let numero = caracteres.as_str();
    if numero.parse::<u64>().ok().filter(|n| *n > 0).is_some() && matches!(sufijo, 'k' | 'm' | 'g')
    {
        Ok(())
    } else {
        Err("debe ser una cantidad de memoria como 48m".to_string())
    }
}
