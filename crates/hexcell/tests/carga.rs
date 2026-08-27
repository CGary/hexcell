//! Arnés de carga del canal: ráfaga de 100 eventos concurrentes contra el `Motor` real.
//!
//! Cierra la tarea 12 de la etapa A-4 (`docs/plan/fase-a-4-admision-presupuesto.md`) y su
//! criterio de aceptación vinculado al PRD QA «Prueba de Carga del Canal».
//!
//! No es un test de regresión: depende del anfitrión (Linux, `/proc/self/status`) y de la
//! carga del sistema en el instante de ejecución. Por eso queda `#[ignore]`; se invoca solo,
//! sin filtro más amplio que `carga_del_canal`, para que ningún otro test contamine la
//! lectura de `VmRSS`:
//!
//! ```text
//! cargo test --workspace -- --ignored carga_del_canal --nocapture
//! ```
//!
//! `ConfiguracionGcra::nueva(0.5, 9)`: tasa de 0,5 eventos/s, tolerancia de ráfaga 9. GCRA
//! admite `tolerancia_rafaga + 1 = 10` eventos consecutivos sin avance de reloj; con un
//! drenaje de milisegundos el reloj real no emite ranuras adicionales, así que la banda
//! determinista de admitidos es `10..=15`.
//!
//! La lectura de `/proc/self/status` incluye el proceso del runner de `cargo test`, así que
//! el umbral del 15 % de crecimiento de RSS es conservador frente a los 6 MB en reposo del
//! proceso desnudo (`docs/STATUS.md`); las cifras absolutas en kB se imprimen para que el
//! operador las juzgue directamente.

mod comun;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use comun::{DirectorioTemporal, repositorio_temporal};
use hexcell::apagado::SenalDeApagado;
use hexcell::concurrencia::LimitadorDeConcurrencia;
use hexcell::metricas::{RegistroDeMetricas, tomar_instantanea};
use hexcell::motor::Motor;
use hexcell::procesador::{ProcesadorDeEco, ProcesadorDeMensajes};
use hexcell_canal_simulado::{AdaptadorSimulado, ErrorDelAdaptadorSimulado, RelojDePrueba};
use hexcell_core::admision::ConfiguracionGcra;
use hexcell_core::canal::{
    ChannelAdapter, EstadoVentanaServicio, EventoEntrante, MensajeSaliente, ResultadoEnvio,
};
use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
use tokio::sync::Mutex as TokioMutex;

// Envoltura del adaptador: delega en un Arc para conservar el manejador
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

// Procesador que envuelve ProcesadorDeEco y registra el instante de finalización
struct ProcesadorQueMideLatencia {
    eco: ProcesadorDeEco,
    completados: Arc<TokioMutex<HashMap<String, Instant>>>,
}

impl ProcesadorDeMensajes for ProcesadorQueMideLatencia {
    async fn procesar(&self, evento: &EventoEntrante) -> Option<MensajeSaliente> {
        let resultado = self.eco.procesar(evento).await;
        let clave = evento.deduplicacion.como_str().to_string();
        self.completados.lock().await.insert(clave, Instant::now());
        resultado
    }
}

// Lectura de VmRSS desde /proc/self/status
fn leer_vm_rss_kb() -> u64 {
    let contenido = std::fs::read_to_string("/proc/self/status")
        .unwrap_or_else(|e| panic!("no se pudo leer /proc/self/status: {e}"));
    let linea = contenido
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .unwrap_or_else(|| panic!("no se encontró VmRSS en /proc/self/status"));
    linea
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no se pudo interpretar VmRSS: {linea}"))
}

// Cálculo de percentil sobre una lista ordenada
fn percentil(valores: &[u128], p: usize) -> u128 {
    if valores.is_empty() {
        return 0;
    }
    let indice = (p * (valores.len() - 1)) / 100;
    valores[indice]
}

// Arnés principal
#[tokio::test]
#[ignore]
async fn carga_del_canal_100_eventos_concurrentes() {
    // --- 1. Infraestructura de persistencia y adaptador ---
    let directorio = DirectorioTemporal::nuevo("carga");
    let repositorio = repositorio_temporal(directorio.ruta());

    let reloj = RelojDePrueba::nuevo(SystemTime::UNIX_EPOCH);
    let (adaptador, receptor_eventos) = AdaptadorSimulado::nuevo(Arc::new(reloj.clone()), 128);
    let adaptador = Arc::new(adaptador);
    // --- 2. Componentes observables ---
    let registro = Arc::new(RegistroDeMetricas::nuevo());
    let limitador = LimitadorDeConcurrencia::nuevo(100);
    let completados: Arc<TokioMutex<HashMap<String, Instant>>> =
        Arc::new(TokioMutex::new(HashMap::new()));

    let procesador = ProcesadorQueMideLatencia {
        eco: ProcesadorDeEco,
        completados: Arc::clone(&completados),
    };
    // --- 3. Motor con GCRA personalizada ---
    let configuracion_gcra =
        ConfiguracionGcra::nueva(0.5, 9).expect("ConfiguracionGcra::nueva(0.5, 9) debe ser válida");

    let motor = Motor::nuevo(
        AdaptadorQueDelegaEnArc(Arc::clone(&adaptador)),
        procesador,
        receptor_eventos,
        Duration::from_secs(3600),
        Arc::clone(&repositorio),
    )
    .con_configuracion_gcra(configuracion_gcra)
    .con_limite_de_concurrencia(limitador.clone())
    .con_metricas(Arc::clone(&registro));
    // --- 4. VmRSS antes de la inyección ---
    let rss_antes_kb = leer_vm_rss_kb();
    // --- 5. Inyección concurrente de 100 eventos ---
    let conversacion = IdConversacion::nuevo("conversacion-de-carga");
    let instantes_de_inyeccion: Arc<TokioMutex<HashMap<String, Instant>>> =
        Arc::new(TokioMutex::new(HashMap::new()));

    let mut manejadores = Vec::with_capacity(100);
    for i in 0..100u32 {
        let adaptador_clon = Arc::clone(&adaptador);
        let conv = conversacion.clone();
        let instantes = Arc::clone(&instantes_de_inyeccion);
        let id_dedup = format!("dedup-carga-{i}");

        manejadores.push(tokio::spawn(async move {
            let evento = EventoEntrante {
                remitente: IdRemitente::nuevo("remitente-carga"),
                conversacion: conv,
                contenido: format!("mensaje-carga-{i}"),
                marca_temporal: SystemTime::UNIX_EPOCH,
                deduplicacion: IdDeduplicacion::nuevo(&id_dedup),
            };
            let ahora = Instant::now();
            adaptador_clon
                .inyectar(evento)
                .await
                .expect("el canal de 128 debe aceptar los 100 eventos");
            instantes.lock().await.insert(id_dedup, ahora);
        }));
    }

    for manejador in manejadores {
        manejador
            .await
            .expect("la tarea de inyección no debe fallar");
    }
    // --- 6. Drenaje determinista: sondear contadores hasta 100 o 30 s ---
    let mut motor = motor;
    let reg_clon = Arc::clone(&registro);
    let lim_clon = limitador.clone();
    let repo_clon = Arc::clone(&repositorio);
    let manejador_motor = tokio::spawn(async move {
        motor.ejecutar(SenalDeApagado::nunca()).await;
    });

    let plazo = Instant::now() + Duration::from_secs(30);
    loop {
        let instantanea = tomar_instantanea(&reg_clon, &lim_clon, &repo_clon)
            .expect("tomar_instantanea no debe fallar");
        if instantanea.admitidos + instantanea.descartados_admision >= 100 {
            break;
        }
        if Instant::now() >= plazo {
            panic!(
                "plazo de 30 s agotado: admitidos={}, descartados_admision={}",
                instantanea.admitidos, instantanea.descartados_admision
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    manejador_motor.abort();
    let _ = manejador_motor.await;
    // --- 7. VmRSS después del drenaje ---
    let rss_despues_kb = leer_vm_rss_kb();
    // --- 8. Instantánea final y aserciones ---
    let instantanea = tomar_instantanea(&registro, &limitador, &repositorio)
        .expect("tomar_instantanea no debe fallar");

    let admitidos = instantanea.admitidos;
    let descartados_admision = instantanea.descartados_admision;
    let descartados_concurrencia = instantanea.descartados_concurrencia;

    // AC-2: identidad de contadores
    assert_eq!(
        admitidos + descartados_admision,
        100,
        "admitidos + descartados_admision debe ser 100"
    );
    assert!(
        descartados_admision > 0,
        "descartados_admision debe ser mayor que cero"
    );

    // AC-2 (aislamiento): ningún descarte por concurrencia
    assert_eq!(
        descartados_concurrencia, 0,
        "descartados_concurrencia debe ser 0 (limitador de 100)"
    );

    // Banda de determinismo: admitidos en 10..=15
    assert!(
        (10..=15).contains(&admitidos),
        "admitidos ({admitidos}) debe caer en la banda 10..=15"
    );

    // AC-2 (sin efectos secundarios): los envíos capturados igualan admitidos
    let envios = adaptador.envios_capturados();
    assert_eq!(
        envios.len() as u64,
        admitidos,
        "envios_capturados ({}) debe igualar admitidos ({admitidos})",
        envios.len()
    );

    // AC-3: crecimiento de RSS <= 15 %
    let crecimiento_pct = if rss_antes_kb > 0 {
        ((rss_despues_kb.saturating_sub(rss_antes_kb)) * 100) / rss_antes_kb
    } else {
        0
    };
    assert!(
        crecimiento_pct <= 15,
        "crecimiento de VmRSS ({crecimiento_pct} %) supera el 15 %: antes={rss_antes_kb} kB, después={rss_despues_kb} kB"
    );
    // --- Latencia de eventos admitidos ---
    let inyecciones = instantes_de_inyeccion.lock().await;
    let finalizaciones = completados.lock().await;

    let mut latencias_us: Vec<u128> = Vec::new();
    for (clave, inicio) in inyecciones.iter() {
        if let Some(fin) = finalizaciones.get(clave) {
            latencias_us.push(fin.duration_since(*inicio).as_micros());
        }
    }
    latencias_us.sort();

    let (latencia_min, latencia_p50, latencia_max) = if latencias_us.is_empty() {
        (0, 0, 0)
    } else {
        (
            latencias_us[0],
            percentil(&latencias_us, 50),
            latencias_us[latencias_us.len() - 1],
        )
    };
    // --- Imprimir resultados en español ---
    println!("=== Resultados del arnés de carga ===");
    println!("eventos_inyectados=100");
    println!("admitidos={admitidos}");
    println!("descartados_admision={descartados_admision}");
    println!("descartados_concurrencia={descartados_concurrencia}");
    println!("envios_capturados={}", envios.len());
    println!("latencia_min_us={latencia_min}");
    println!("latencia_p50_us={latencia_p50}");
    println!("latencia_max_us={latencia_max}");
    println!("rss_antes_kb={rss_antes_kb}");
    println!("rss_despues_kb={rss_despues_kb}");
    println!("rss_crecimiento_pct={crecimiento_pct}");
}
