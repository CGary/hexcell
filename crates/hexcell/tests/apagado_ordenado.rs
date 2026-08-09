//! Tests de proceso real del apagado ordenado (AC-5, AC-6, AC-7), siguiendo el patrón de
//! `tests/salud_http.rs`: se lanza el binario compilado y se le envía la señal de verdad.

mod comun;

use std::time::Duration;

use comun::{DirectorioTemporal, abrir_persistencia, lanzar_binario_con_variables};
use hexcell_storage::{EventoDeHistorial, SalienteHistorico};

/// Contenido del evento de arranque para estos tests: no necesita ser distintivo de contenido
/// (eso lo cubre `tests/registro_estructurado.rs`), solo distinto entre los dos casos de este
/// archivo para no confundir sus historiales.
const CONTENIDO_DEL_EVENTO_DE_ARRANQUE: &str = "evento en vuelo durante el apagado";

/// `AdaptadorSimulado::inyectar_desde_contacto` deriva el `IdConversacion` del contacto como
/// `conversacion-de-{contacto}`; `main.rs` usa siempre el mismo contacto sintético de arranque.
const CONVERSACION_DEL_EVENTO_DE_ARRANQUE: &str = "conversacion-de-arranque-simulado";

#[test]
fn un_evento_en_vuelo_se_completa_antes_de_que_el_proceso_termine_por_sigterm() {
    let directorio = DirectorioTemporal::nuevo("apagado-en-vuelo");
    let mut binario = lanzar_binario_con_variables(
        directorio.ruta(),
        &[
            (
                "HEXCELL_EVENTO_SIMULADO_DE_ARRANQUE",
                CONTENIDO_DEL_EVENTO_DE_ARRANQUE,
            ),
            ("HEXCELL_LATENCIA_INFERENCIA_SIMULADA_MS", "1500"),
        ],
    );

    // Espera a que el evento esté demostrablemente en curso antes de enviar la señal: esto es lo
    // que hace que el test sea una prueba y no una carrera con un `sleep` arbitrario (AC-7).
    binario
        .esperar_linea("inferencia_iniciada", Duration::from_secs(5))
        .expect("el evento de arranque debe empezar a procesarse antes de la señal");

    binario.enviar_sigterm();

    let estado = binario
        .esperar_salida(Duration::from_secs(30))
        .expect("el proceso debe terminar en menos de 30 segundos tras SIGTERM");
    assert!(
        estado.success(),
        "el proceso debe terminar con código de salida 0, obtuvo: {estado:?}"
    );

    let salida = binario.salida_capturada();
    assert!(
        salida.contains("punto_de_control_wal"),
        "la salida debe incluir la línea del punto de control del WAL: {salida}"
    );
    assert!(
        salida.contains("\"wal_sesiones_bytes\":0") || salida.contains("wal_sesiones_bytes=0"),
        "el punto de control debe reportar el WAL de sessions.db en cero bytes: {salida}"
    );

    drop(binario);

    let (_pools, repositorio) = abrir_persistencia(directorio.ruta());
    let conversacion =
        hexcell_core::identidad::IdConversacion::nuevo(CONVERSACION_DEL_EVENTO_DE_ARRANQUE);
    let historial = repositorio
        .historial(&conversacion)
        .expect("el historial debe poder leerse tras la salida del proceso");

    assert!(
        historial.iter().any(
            |entrada| matches!(entrada, EventoDeHistorial::Entrante(contenido)
                if contenido == CONTENIDO_DEL_EVENTO_DE_ARRANQUE)
        ),
        "el evento en vuelo debe quedar registrado como entrante: {historial:?}"
    );
    assert!(
        historial.iter().any(|entrada| matches!(
            entrada,
            EventoDeHistorial::Saliente(SalienteHistorico::RespuestaLibre { .. })
        )),
        "la respuesta al evento en vuelo debe quedar registrada como saliente: {historial:?}"
    );
}

#[test]
fn sin_evento_en_vuelo_el_proceso_termina_rapido_y_con_exito_tras_sigterm() {
    let directorio = DirectorioTemporal::nuevo("apagado-rapido");
    let mut binario = lanzar_binario_con_variables(directorio.ruta(), &[]);

    binario.enviar_sigterm();

    let estado = binario
        .esperar_salida(Duration::from_secs(30))
        .expect("el proceso debe terminar en menos de 30 segundos tras SIGTERM");
    assert!(
        estado.success(),
        "el proceso debe terminar con código de salida 0, obtuvo: {estado:?}"
    );
}
