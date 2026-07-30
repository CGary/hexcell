//! Test de proceso real del registro estructurado (AC-8, AC-9): campos exigidos presentes, y el
//! contenido de un mensaje reconocible ausente de toda la salida capturada.

mod comun;

use std::time::Duration;

use comun::{DirectorioTemporal, lanzar_binario_con_variables};

/// Marcador distintivo e improbable: si apareciera en la salida por cualquier vía distinta a
/// este test, sería una señal inequívoca de fuga, no un falso positivo.
const MARCADOR_DE_CONTENIDO: &str = "marcador-de-contenido-jamas-debe-aparecer-en-los-logs-93f7";

#[test]
fn los_logs_llevan_los_campos_exigidos_y_nunca_el_contenido_del_mensaje() {
    let directorio = DirectorioTemporal::nuevo("registro-estructurado");
    let binario = lanzar_binario_con_variables(
        directorio.ruta(),
        &[("HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE", MARCADOR_DE_CONTENIDO)],
    );

    let linea_evento_recibido = binario
        .esperar_linea("evento_recibido", Duration::from_secs(5))
        .expect("debe aparecer una línea evento_recibido para el evento de arranque");

    assert!(
        linea_evento_recibido.contains("\"id_celula\""),
        "la línea debe incluir id_celula: {linea_evento_recibido}"
    );
    assert!(
        linea_evento_recibido.contains("\"id_evento\""),
        "la línea debe incluir id_evento: {linea_evento_recibido}"
    );
    assert!(
        linea_evento_recibido.contains("\"latencia_ms\""),
        "la línea debe incluir latencia_ms: {linea_evento_recibido}"
    );

    // Da tiempo a que el resto del procesamiento de ese evento (inferencia, envío) también
    // termine y quede reflejado en la salida capturada antes de inspeccionarla entera.
    binario
        .esperar_linea("envio_aceptado", Duration::from_secs(5))
        .expect("el evento de arranque debe terminar de procesarse");

    let salida_completa = binario.salida_capturada();
    assert!(
        !salida_completa.contains(MARCADOR_DE_CONTENIDO),
        "el contenido del mensaje no debe aparecer en ningún lugar de la salida capturada: \
         {salida_completa}"
    );
}
