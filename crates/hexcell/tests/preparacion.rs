//! Tests del combinador de preparación (AC-10): las ocho combinaciones de los tres términos y la
//! degradación observable de `GET /health/ready` cuando una base desaparece del disco.
//!
//! Las ocho combinaciones se recorren contra la función pura, que no toca red ni disco: es la
//! única manera de ejercitar la sesión caída, porque ningún adaptador de esta etapa la produce.
//! Un término que ningún test puede tumbar es decoración, no una comprobación.

mod comun;

use comun::{DirectorioTemporal, lanzar_binario_con_ruta_de_datos, peticion_http_cruda};
use hexcell::preparacion::{
    COMPONENTE_SESION_DEL_CANAL, Preparacion, SesionDelCanal, evaluar_preparacion,
};
use hexcell_storage::{
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_SESIONES, Vitalidad,
};

/// Vitalidad caída con un motivo cualquiera, para las combinaciones del combinador puro.
fn caida(componente: &'static str) -> Vitalidad {
    Vitalidad::Caida {
        componente,
        motivo: "caída forzada por el test".to_string(),
    }
}

#[test]
fn solo_los_tres_terminos_sanos_producen_lista() {
    let combinaciones = [
        (true, true, true),
        (true, true, false),
        (true, false, true),
        (true, false, false),
        (false, true, true),
        (false, true, false),
        (false, false, true),
        (false, false, false),
    ];

    for (sesiones_sana, conocimiento_sano, sesion_activa) in combinaciones {
        let vitalidad_de_sesiones = if sesiones_sana {
            Vitalidad::Sana
        } else {
            caida(NOMBRE_DE_ARCHIVO_DE_SESIONES)
        };
        let vitalidad_de_conocimiento = if conocimiento_sano {
            Vitalidad::Sana
        } else {
            caida(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO)
        };
        let sesion = if sesion_activa {
            SesionDelCanal::siempre_activa()
        } else {
            SesionDelCanal::caida()
        };

        let preparacion =
            evaluar_preparacion(vitalidad_de_sesiones, vitalidad_de_conocimiento, &sesion);
        let todos_sanos = sesiones_sana && conocimiento_sano && sesion_activa;

        match (todos_sanos, preparacion) {
            (true, Preparacion::Lista) => {}
            (false, Preparacion::NoLista { .. }) => {}
            (esperado_listo, obtenido) => panic!(
                "combinación ({sesiones_sana}, {conocimiento_sano}, {sesion_activa}): \
                 se esperaba listo={esperado_listo} y llegó {obtenido:?}"
            ),
        }
    }
}

#[test]
fn cada_termino_caido_se_nombra_en_la_respuesta() {
    match evaluar_preparacion(
        caida(NOMBRE_DE_ARCHIVO_DE_SESIONES),
        Vitalidad::Sana,
        &SesionDelCanal::siempre_activa(),
    ) {
        Preparacion::NoLista { componente, .. } => {
            assert_eq!(componente, NOMBRE_DE_ARCHIVO_DE_SESIONES)
        }
        Preparacion::Lista => panic!("con sessions.db caída no puede estar lista"),
    }

    match evaluar_preparacion(
        Vitalidad::Sana,
        caida(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO),
        &SesionDelCanal::siempre_activa(),
    ) {
        Preparacion::NoLista { componente, .. } => {
            assert_eq!(componente, NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO)
        }
        Preparacion::Lista => panic!("con knowledge_live.db caída no puede estar lista"),
    }

    // La sesión caída no la produce ningún adaptador de esta etapa; se ejercita con el tipo
    // directamente, que es exactamente el motivo por el que el constructor existe.
    match evaluar_preparacion(Vitalidad::Sana, Vitalidad::Sana, &SesionDelCanal::caida()) {
        Preparacion::NoLista { componente, .. } => {
            assert_eq!(componente, COMPONENTE_SESION_DEL_CANAL)
        }
        Preparacion::Lista => panic!("con la sesión del canal caída no puede estar lista"),
    }
}

#[test]
fn health_ready_degrada_a_503_cuando_desaparece_la_base_de_conocimiento() {
    let directorio = DirectorioTemporal::nuevo("preparacion-degradacion");
    let binario = lanzar_binario_con_ruta_de_datos(directorio.ruta());

    let inicial = peticion_http_cruda(&binario.direccion, "/health/ready");
    assert!(
        inicial.starts_with("HTTP/1.1 200"),
        "la célula recién arrancada debe estar lista: {inicial}"
    );

    std::fs::remove_file(directorio.ruta().join(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO))
        .expect("borrar knowledge_live.db del disco");

    let degradada = peticion_http_cruda(&binario.direccion, "/health/ready");
    assert!(
        degradada.starts_with("HTTP/1.1 503"),
        "sin knowledge_live.db la célula no está lista: {degradada}"
    );
    assert!(
        degradada.contains(NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO),
        "la respuesta debe nombrar el componente que falló: {degradada}"
    );
}

#[test]
fn reconectando_produce_no_lista_con_componente_sesion() {
    match evaluar_preparacion(
        Vitalidad::Sana,
        Vitalidad::Sana,
        &SesionDelCanal::reconectando(),
    ) {
        Preparacion::NoLista { componente, motivo } => {
            assert_eq!(componente, COMPONENTE_SESION_DEL_CANAL);
            assert!(
                motivo.contains("reconectando"),
                "el motivo debe mencionar reconectando: {motivo}"
            );
        }
        Preparacion::Lista => panic!("con la sesión reconectando no puede estar lista"),
    }
}

#[test]
fn desvinculada_produce_no_lista_con_componente_sesion() {
    match evaluar_preparacion(
        Vitalidad::Sana,
        Vitalidad::Sana,
        &SesionDelCanal::desvinculada(),
    ) {
        Preparacion::NoLista { componente, motivo } => {
            assert_eq!(componente, COMPONENTE_SESION_DEL_CANAL);
            assert!(
                motivo.contains("desvinculada"),
                "el motivo debe mencionar desvinculada: {motivo}"
            );
        }
        Preparacion::Lista => panic!("con la sesión desvinculada no puede estar lista"),
    }
}

#[test]
fn pausada_produce_no_lista_con_componente_sesion() {
    match evaluar_preparacion(Vitalidad::Sana, Vitalidad::Sana, &SesionDelCanal::pausada()) {
        Preparacion::NoLista { componente, motivo } => {
            assert_eq!(componente, COMPONENTE_SESION_DEL_CANAL);
            assert!(
                motivo.contains("pausada"),
                "el motivo debe mencionar pausada: {motivo}"
            );
        }
        Preparacion::Lista => panic!("con la sesión pausada no puede estar lista"),
    }
}

#[test]
fn activa_produce_lista_con_vitalidades_sanas() {
    match evaluar_preparacion(
        Vitalidad::Sana,
        Vitalidad::Sana,
        &SesionDelCanal::siempre_activa(),
    ) {
        Preparacion::Lista => {}
        Preparacion::NoLista { componente, motivo } => {
            panic!(
                "con la sesión activa y vitalidades sanas debe estar lista, pero: {componente}: {motivo}"
            )
        }
    }
}
