//! Tests del almacén de identidad del adaptador: persistencia a través de un reabrir, idempotencia
//! de `registrar` y el conteo que acuña el siguiente identificador (AC-5, AC-6), más su propio
//! respaldo verificado.

mod comun;

use comun::DirectorioTemporal;
use hexcell_storage::{AlmacenDeIdentidad, ErrorDeAlmacen};

#[test]
fn un_contacto_registrado_sobrevive_a_cerrar_y_reabrir_el_almacen() {
    let directorio = DirectorioTemporal::nuevo("identidad-persistencia");

    {
        let almacen = AlmacenDeIdentidad::abrir(directorio.ruta()).expect("abrir el almacén");
        almacen
            .registrar("contacto-uno", "conversacion-0")
            .expect("registrar el primer contacto");
    }

    let almacen_reabierto =
        AlmacenDeIdentidad::abrir(directorio.ruta()).expect("reabrir el almacén");
    assert_eq!(
        almacen_reabierto
            .buscar("contacto-uno")
            .expect("buscar el contacto"),
        Some("conversacion-0".to_string())
    );
    assert_eq!(
        almacen_reabierto
            .contactos_registrados()
            .expect("contar los contactos registrados"),
        1
    );
}

#[test]
fn buscar_un_contacto_desconocido_devuelve_nada_sin_fallar() {
    let directorio = DirectorioTemporal::nuevo("identidad-desconocido");
    let almacen = AlmacenDeIdentidad::abrir(directorio.ruta()).expect("abrir el almacén");

    assert_eq!(
        almacen
            .buscar("nunca-visto")
            .expect("buscar un contacto desconocido no debe fallar"),
        None
    );
}

#[test]
fn registrar_es_idempotente_y_no_pisa_un_identificador_ya_asignado() {
    let directorio = DirectorioTemporal::nuevo("identidad-idempotencia");
    let almacen = AlmacenDeIdentidad::abrir(directorio.ruta()).expect("abrir el almacén");

    almacen
        .registrar("contacto-estable", "conversacion-0")
        .expect("el primer registro no debe fallar");
    // Un segundo intento con OTRO identificador no debe sobrescribir al primero: INSERT OR
    // IGNORE deja la fila existente tal y como estaba.
    almacen
        .registrar("contacto-estable", "conversacion-9-que-no-deberia-quedar")
        .expect("un segundo registro del mismo contacto no debe fallar");

    assert_eq!(
        almacen
            .buscar("contacto-estable")
            .expect("buscar el contacto"),
        Some("conversacion-0".to_string()),
        "registrar debe ser idempotente: el segundo intento no debe pisar el primero"
    );
    assert_eq!(
        almacen
            .contactos_registrados()
            .expect("contar los contactos registrados"),
        1,
        "un registro repetido del mismo contacto no debe contar dos veces"
    );
}

#[test]
fn el_conteo_de_contactos_registrados_avanza_con_cada_contacto_nuevo_y_solo_con_uno_nuevo() {
    let directorio = DirectorioTemporal::nuevo("identidad-conteo");
    let almacen = AlmacenDeIdentidad::abrir(directorio.ruta()).expect("abrir el almacén");

    assert_eq!(
        almacen
            .contactos_registrados()
            .expect("el almacén recién abierto no tiene contactos"),
        0
    );

    almacen
        .registrar("contacto-a", "conversacion-0")
        .expect("registrar el primer contacto");
    assert_eq!(almacen.contactos_registrados().expect("contar"), 1);

    almacen
        .registrar("contacto-b", "conversacion-1")
        .expect("registrar el segundo contacto");
    assert_eq!(almacen.contactos_registrados().expect("contar"), 2);

    // Repetir el primero no avanza el conteo.
    almacen
        .registrar("contacto-a", "conversacion-0")
        .expect("repetir el registro del primer contacto");
    assert_eq!(almacen.contactos_registrados().expect("contar"), 2);
}

#[test]
fn el_respaldo_del_almacen_produce_una_copia_verificada() {
    let directorio = DirectorioTemporal::nuevo("identidad-respaldo-origen");
    let destino = DirectorioTemporal::nuevo("identidad-respaldo-destino");
    let almacen = AlmacenDeIdentidad::abrir(directorio.ruta()).expect("abrir el almacén");
    almacen
        .registrar("contacto-respaldado", "conversacion-0")
        .expect("registrar un contacto antes de respaldar");

    let copia = almacen
        .respaldar_en(destino.ruta())
        .expect("respaldar el almacén de identidad");

    assert_eq!(
        copia.nombre_logico,
        hexcell_storage::NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR
    );
    assert!(copia.ruta.is_file());
    assert!(copia.bytes > 0);

    // Un segundo respaldo hacia el mismo destino falla: VACUUM INTO no sobrescribe.
    let segundo_intento = almacen.respaldar_en(destino.ruta());
    assert!(matches!(
        segundo_intento,
        Err(ErrorDeAlmacen::DestinoDeRespaldoOcupado { .. })
    ));
}
