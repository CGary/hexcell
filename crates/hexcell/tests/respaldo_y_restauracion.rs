//! Test de ciclo completo de respaldo y restauración (AC-1, AC-2, AC-5, AC-6, AC-7).
//!
//! Todo ocurre en proceso, sobre tres directorios temporales distintos: el de datos original, el
//! destino del respaldo (simulando "fuera del disco del servidor" con otro directorio local,
//! `docs/STATUS.md`) y el directorio limpio donde se restaura. Ninguno se reutiliza entre pasos.
//!
//! `verificar_celula_restaurada` factoriza el criterio de éxito de AC-5/AC-6 en un único punto:
//! mirar el último envío capturado por el adaptador de la célula restaurada. El test positivo deja
//! que el motor procese antes de llamarla; el negativo, deliberadamente, no. Ambos casos comparten
//! la misma función, para que el criterio de éxito no pueda tener dos varas de medir.
//!
//! El identificador que se compara en el caso positivo es el de la **segunda** conversación
//! registrada durante la ejecución original, no la primera: con el almacén persistente, el
//! identificador de un contacto nuevo se acuña a partir del conteo de contactos ya registrados, así
//! que un almacén vacío asignaría el mismo primer identificador que uno restaurado, pero no el
//! segundo. Comparar sobre el segundo contacto es lo que hace que la aserción no sea vacía
//! (`docs/bitacora-de-descartes.md` no recoge esto porque no es un descarte, es la razón de ser de
//! esta prueba).

mod comun;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use comun::{DirectorioTemporal, abrir_persistencia_con_identidad};
use hexcell::apagado::SenalDeApagado;
use hexcell::motor::Motor;
use hexcell::procesador::ProcesadorDeEco;
use hexcell::respaldo::respaldar_celula;
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, RelojDePrueba};
use hexcell_core::canal::{ChannelAdapter, EstadoVentanaServicio, MensajeSaliente, ResultadoEnvio};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion};
use hexcell_storage::{
    NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO, NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
    NOMBRE_DE_ARCHIVO_DE_SESIONES,
};

/// Envoltorio de test: delega en un `Arc<AdaptadorSimulado>` compartido con quien inyecta y quien,
/// luego, inspecciona `envios_capturados()`. Igual que el de `tests/continuidad_de_hilo.rs`, pero
/// declarado aquí de nuevo porque los binarios de test no comparten código entre sí salvo por
/// `mod comun`.
struct AdaptadorQueDelegaEnArc(Arc<AdaptadorSimulado>);

impl ChannelAdapter for AdaptadorQueDelegaEnArc {
    type Error = ErrorDelAdaptadorSimulado;

    async fn send(
        &self,
        conversacion: &IdConversacion,
        mensaje: MensajeSaliente,
    ) -> Result<ResultadoEnvio, Self::Error> {
        self.0.send(conversacion, mensaje).await
    }

    async fn estado_ventana(
        &self,
        conversacion: &IdConversacion,
    ) -> Result<EstadoVentanaServicio, Self::Error> {
        self.0.estado_ventana(conversacion).await
    }
}

/// Motivo por el que [`verificar_celula_restaurada`] no encontró una respuesta.
#[derive(Debug, PartialEq, Eq)]
enum MotivoDeRestauracionFallida {
    /// Ningún envío quedó capturado: la célula no respondió por el puerto.
    SinRespuesta,
}

/// Único criterio de éxito de una restauración: que la célula haya respondido por el puerto, y
/// con qué identificador. Se usa igual para el caso positivo (tras dejar procesar al motor) y para
/// el negativo (sin dejarlo procesar), precisamente para que la sola presencia de archivos nunca
/// pueda hacerla pasar por sí sola.
fn verificar_celula_restaurada(
    adaptador: &AdaptadorSimulado,
) -> Result<IdConversacion, MotivoDeRestauracionFallida> {
    adaptador
        .envios_capturados()
        .last()
        .map(|(conversacion, _, _)| conversacion.clone())
        .ok_or(MotivoDeRestauracionFallida::SinRespuesta)
}

/// Deja avanzar `motor.ejecutar(..)` un tiempo corto sin moverlo de sitio, para poder seguir
/// inspeccionándolo después (mismo patrón que `tests/continuidad_de_hilo.rs`).
async fn dejar_procesar<A, P>(motor: &mut Motor<A, P>)
where
    A: ChannelAdapter,
    P: hexcell::procesador::ProcesadorDeMensajes,
{
    tokio::select! {
        () = motor.ejecutar(SenalDeApagado::nunca()) => {}
        () = tokio::time::sleep(Duration::from_millis(50)) => {}
    }
}

#[tokio::test]
async fn una_celula_restaurada_procesa_y_responde_con_el_identificador_original() {
    let original = DirectorioTemporal::nuevo("respaldo-original");
    let destino_de_respaldo = DirectorioTemporal::nuevo("respaldo-destino");
    let restaurado = DirectorioTemporal::nuevo("respaldo-restaurado");

    // Paso 1: una célula procesa dos contactos distintos, en un orden conocido.
    let (pools, repositorio, almacen) = abrir_persistencia_con_identidad(original.ruta());
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) =
        AdaptadorSimulado::nuevo_con_almacen(Arc::new(reloj), 8, Arc::clone(&almacen));
    let adaptador = Arc::new(adaptador);

    let conversacion_uno = adaptador
        .inyectar_desde_contacto(
            "contacto-ciclo-uno",
            "hola",
            IdDeduplicacion::nuevo("dedup-ciclo-uno"),
        )
        .await
        .expect("el primer contacto debe aceptarse");
    let conversacion_dos = adaptador
        .inyectar_desde_contacto(
            "contacto-ciclo-dos",
            "hola también",
            IdDeduplicacion::nuevo("dedup-ciclo-dos"),
        )
        .await
        .expect("el segundo contacto debe aceptarse");
    assert_ne!(
        conversacion_uno, conversacion_dos,
        "dos contactos distintos deben resolver a conversaciones distintas"
    );

    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        Arc::clone(&repositorio),
    );
    dejar_procesar(&mut motor).await;

    let historial_original_de_dos = motor
        .historial(&conversacion_dos)
        .expect("leer el historial original del segundo contacto");
    assert!(
        !historial_original_de_dos.is_empty(),
        "el segundo contacto debe haber dejado historial antes del respaldo"
    );

    // Paso 2: respaldo en caliente de las tres bases alcanzables desde esta etapa.
    let resumen = respaldar_celula(&pools, &almacen, destino_de_respaldo.ruta())
        .expect("el respaldo con la célula operando no debe fallar");
    assert_eq!(resumen.copias.len(), 3, "deben salir las tres copias");
    for nombre_esperado in [
        NOMBRE_DE_ARCHIVO_DE_SESIONES,
        NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO,
        NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
    ] {
        assert!(
            resumen
                .copias
                .iter()
                .any(|copia| copia.nombre_logico == nombre_esperado),
            "falta la copia de {nombre_esperado}"
        );
        assert!(
            destino_de_respaldo.ruta().join(nombre_esperado).is_file(),
            "el archivo de respaldo de {nombre_esperado} debe existir en disco"
        );
    }

    // Paso 3: restauración, copiando los tres archivos a un directorio limpio distinto del
    // original, bajo sus nombres canónicos.
    for nombre in [
        NOMBRE_DE_ARCHIVO_DE_SESIONES,
        NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO,
        NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
    ] {
        std::fs::copy(
            destino_de_respaldo.ruta().join(nombre),
            restaurado.ruta().join(nombre),
        )
        .unwrap_or_else(|error| panic!("copiar {nombre} al directorio restaurado: {error}"));
    }

    // Paso 4: una SEGUNDA célula arranca contra el directorio restaurado.
    let (_pools_restaurados, repositorio_restaurado, almacen_restaurado) =
        abrir_persistencia_con_identidad(restaurado.ruta());
    let reloj_restaurado = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador_restaurado, receptor_restaurado) = AdaptadorSimulado::nuevo_con_almacen(
        Arc::new(reloj_restaurado),
        8,
        Arc::clone(&almacen_restaurado),
    );
    let adaptador_restaurado = Arc::new(adaptador_restaurado);
    let mut motor_restaurado = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador_restaurado)),
        ProcesadorDeEco,
        receptor_restaurado,
        Duration::from_secs(3600),
        Arc::clone(&repositorio_restaurado),
    );

    // El historial restaurado debe seguir ahí antes de inyectar nada nuevo.
    let historial_restaurado_de_dos = motor_restaurado
        .historial(&conversacion_dos)
        .expect("leer el historial restaurado del segundo contacto");
    assert_eq!(
        historial_restaurado_de_dos, historial_original_de_dos,
        "el historial restaurado debe coincidir con el que había antes del respaldo"
    );

    // Paso 5: se inyecta un evento nuevo desde el SEGUNDO contacto, ya conocido.
    let conversacion_reinyectada = adaptador_restaurado
        .inyectar_desde_contacto(
            "contacto-ciclo-dos",
            "hola de vuelta",
            IdDeduplicacion::nuevo("dedup-ciclo-dos-tras-restaurar"),
        )
        .await
        .expect("el contacto restaurado debe seguir aceptando eventos");
    assert_eq!(
        conversacion_reinyectada, conversacion_dos,
        "el almacén restaurado debe resolver al mismo identificador que la célula original"
    );

    dejar_procesar(&mut motor_restaurado).await;

    // Paso 6: la célula restaurada respondió con el identificador original, no con uno que un
    // almacén vacío también habría podido producir.
    let identificador_de_respuesta = verificar_celula_restaurada(&adaptador_restaurado)
        .expect("la célula restaurada debe haber respondido por el puerto");
    assert_eq!(
        identificador_de_respuesta, conversacion_dos,
        "la respuesta debe llevar el identificador interno que la célula original asignó"
    );

    let historial_final_de_dos = motor_restaurado
        .historial(&conversacion_dos)
        .expect("leer el historial final del segundo contacto");
    assert!(
        historial_final_de_dos.len() > historial_restaurado_de_dos.len(),
        "el historial debe continuar, no reiniciarse, tras la restauración"
    );
    assert_eq!(
        &historial_final_de_dos[..historial_restaurado_de_dos.len()],
        historial_restaurado_de_dos.as_slice(),
        "el historial restaurado debe seguir intacto y en el mismo orden"
    );
}

#[tokio::test]
async fn una_restauracion_con_historial_pero_sin_respuesta_no_pasa_la_verificacion() {
    // Prueba negativa de AC-6: el mismo entorno restaurado, pero el motor deliberadamente no
    // consume el puerto. La presencia de historial por sí sola no puede darse por una
    // restauración exitosa.
    let original = DirectorioTemporal::nuevo("respaldo-negativo-original");
    let destino_de_respaldo = DirectorioTemporal::nuevo("respaldo-negativo-destino");
    let restaurado = DirectorioTemporal::nuevo("respaldo-negativo-restaurado");

    let (pools, repositorio, almacen) = abrir_persistencia_con_identidad(original.ruta());
    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) =
        AdaptadorSimulado::nuevo_con_almacen(Arc::new(reloj), 8, Arc::clone(&almacen));
    let adaptador = Arc::new(adaptador);

    let conversacion = adaptador
        .inyectar_desde_contacto(
            "contacto-negativo",
            "hola",
            IdDeduplicacion::nuevo("dedup-negativo"),
        )
        .await
        .expect("el contacto debe aceptarse");

    let mut motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        ProcesadorDeEco,
        receptor_eventos,
        Duration::from_secs(3600),
        Arc::clone(&repositorio),
    );
    dejar_procesar(&mut motor).await;

    let historial_original = motor
        .historial(&conversacion)
        .expect("leer el historial original");
    assert!(!historial_original.is_empty());

    respaldar_celula(&pools, &almacen, destino_de_respaldo.ruta())
        .expect("el respaldo no debe fallar");

    for nombre in [
        NOMBRE_DE_ARCHIVO_DE_SESIONES,
        NOMBRE_DE_ARCHIVO_DE_CONOCIMIENTO,
        NOMBRE_DE_ARCHIVO_DE_IDENTIDAD_DEL_ADAPTADOR,
    ] {
        std::fs::copy(
            destino_de_respaldo.ruta().join(nombre),
            restaurado.ruta().join(nombre),
        )
        .unwrap_or_else(|error| panic!("copiar {nombre} al directorio restaurado: {error}"));
    }

    let (_pools_restaurados, repositorio_restaurado, almacen_restaurado) =
        abrir_persistencia_con_identidad(restaurado.ruta());
    let reloj_restaurado = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador_restaurado, _receptor_restaurado) = AdaptadorSimulado::nuevo_con_almacen(
        Arc::new(reloj_restaurado),
        8,
        Arc::clone(&almacen_restaurado),
    );

    // El historial restaurado está presente, leído directamente del repositorio: este test no
    // construye ningún `Motor` que llegue a correr `ejecutar`, a propósito.
    let historial_restaurado = repositorio_restaurado
        .historial(&conversacion)
        .expect("leer el historial restaurado directamente del repositorio");
    assert_eq!(
        historial_restaurado, historial_original,
        "el historial debe restaurarse íntegro aunque el motor nunca lo consuma"
    );

    // ...pero como ningún motor consume nunca el puerto, ningún evento llega a procesarse ni a
    // enviarse: la sola presencia de historial no basta.
    let veredicto = verificar_celula_restaurada(&adaptador_restaurado);
    assert_eq!(
        veredicto,
        Err(MotivoDeRestauracionFallida::SinRespuesta),
        "sin que el motor consuma el puerto, la restauración no debe darse por buena"
    );
}
