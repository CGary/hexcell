//! Procesador de mensajes: punto de extensión del motor, sin ninguna regla de producto.
//!
//! El motor de mensajería (`crate::motor`) despacha cada evento entrante a una implementación de
//! [`ProcesadorDeMensajes`] y envía lo que esta devuelva. Esta tarea añade
//! [`ProcesadorDeInferencia`], que consulta un [`ProveedorDeInferencia`] para decidir la
//! respuesta, y conserva [`ProcesadorDeEco`] tal cual: cinco archivos de test existentes lo usan
//! para ejercitar deduplicación, historial, reinicio y la política ante `FueraDeVentana`, y no
//! deben convertirse en tests del proveedor de inferencia.
//!
//! # Por qué el método se declara `-> impl Future` y no `async fn`
//!
//! La misma razón que `hexcell_core::inferencia::ProveedorDeInferencia`: sobre rustc 1.92.0, `async
//! fn` en un trait dispara `async_fn_in_trait`, que `cargo clippy --workspace -- -D warnings`
//! convierte en error. Las implementaciones sí pueden — y deben — escribirse como `async fn`
//! corriente: el aviso solo se dispara en la declaración del trait, no en sus implementaciones.
//!
//! # Por qué `ProcesadorDeInferencia<I>` exige `I: ProveedorDeInferencia + Sync`
//!
//! `&self` cruza un punto de espera dentro de `procesar`, y el futuro resultante debe seguir
//! siendo `Send` para que el motor pueda lanzarlo en su tarea asíncrona. Sin la cota `Sync` sobre
//! `I`, la compilación falla con un error que señala un punto muy alejado de esta causa; queda
//! escrito aquí para que nadie tenga que redescubrirlo.
//!
//! # Qué hace este procesador ante un fallo del proveedor o rechazo de presupuesto
//!
//! Ante una avería del proveedor, no se genera respuesta (`None`). Sin embargo, ante un
//! rechazo de presupuesto por falta de saldo, el procesador activa el modo degradado:
//! emite un registro estructurado y genera una respuesta local provisional basada en
//! reglas fijas (`Some(MensajeSaliente)`), sin consumir saldo ni invocar al proveedor.

use std::sync::Arc;

use hexcell_core::canal::{EventoEntrante, MensajeSaliente, TestigoDeEntrante};
use hexcell_core::inferencia::{PeticionDeInferencia, ProveedorDeInferencia};
use hexcell_core::presupuesto::estimar_coste;
use hexcell_storage::{RepositorioDeSesiones, ResultadoDeResolucion, VeredictoDeReserva};

use crate::registro::{EntradaDeRegistro, NivelDeRegistro, emitir};

/// Puerto del procesador de mensajes, local a este binario.
///
/// No es un trait del dominio (`hexcell-core`), porque cómo se decide una respuesta es una
/// política de la célula, no un tipo canónico de FR-12.
pub trait ProcesadorDeMensajes {
    /// Decide qué responder, si algo, ante un evento entrante ya normalizado por el adaptador.
    ///
    /// Devolver `None` significa que este evento no genera respuesta; el motor simplemente no
    /// llama a `send` en ese caso.
    fn procesar(
        &self,
        evento: &EventoEntrante,
    ) -> impl Future<Output = Option<MensajeSaliente>> + Send;
}

/// Procesador mínimo de eco: repite el contenido del evento entrante como respuesta libre.
///
/// No decide nada sobre el negocio: ni interpreta el contenido, ni consulta ningún catálogo, ni
/// invoca ningún proveedor externo. Sirve para que los tests que preceden a esta tarea sigan
/// teniendo algo determinista que despachar, sin volverse tests del proveedor de inferencia.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcesadorDeEco;

impl ProcesadorDeMensajes for ProcesadorDeEco {
    async fn procesar(&self, evento: &EventoEntrante) -> Option<MensajeSaliente> {
        let testigo = TestigoDeEntrante::observar(evento);
        Some(
            MensajeSaliente::respuesta_libre(
                &testigo,
                &evento.conversacion,
                evento.contenido.clone(),
            )
            .expect("la conversación coincide siempre"),
        )
    }
}

/// Procesador que delega la decisión de respuesta en un [`ProveedorDeInferencia`] inyectado,
/// previa verificación y reserva atómica de presupuesto en [`RepositorioDeSesiones`].
///
/// Genérico sobre el trait, nunca sobre el tipo concreto del proveedor simulado: el motor que
/// construye este procesador no nombra `ProveedorSimulado` en ningún punto de su firma pública.
pub struct ProcesadorDeInferencia<I>
where
    I: ProveedorDeInferencia,
{
    proveedor: I,
    repositorio: Arc<RepositorioDeSesiones>,
}

impl<I> ProcesadorDeInferencia<I>
where
    I: ProveedorDeInferencia,
{
    /// Construye el procesador sobre el proveedor de inferencia y el repositorio de sesiones.
    pub fn nuevo(proveedor: I, repositorio: Arc<RepositorioDeSesiones>) -> Self {
        Self {
            proveedor,
            repositorio,
        }
    }
}

impl<I> ProcesadorDeMensajes for ProcesadorDeInferencia<I>
where
    I: ProveedorDeInferencia + Sync,
{
    async fn procesar(&self, evento: &EventoEntrante) -> Option<MensajeSaliente> {
        let estimacion = estimar_coste(&evento.contenido);

        let id_reserva = match self.repositorio.reservar_presupuesto(
            &evento.conversacion,
            estimacion,
            evento.marca_temporal,
        ) {
            Ok(VeredictoDeReserva::Concedida { id_reserva, .. }) => id_reserva,
            Ok(VeredictoDeReserva::Rechazada {
                disponible,
                requerido,
            }) => {
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "presupuesto_rechazado")
                        .con_id_conversacion(evento.conversacion.como_str())
                        .con_detalle(format!("requerido: {requerido}, disponible: {disponible}")),
                );
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Aviso, "modo_degradado")
                        .con_id_conversacion(evento.conversacion.como_str()),
                );
                let respuesta_local = crate::reglas_locales::responder_localmente();
                let testigo = TestigoDeEntrante::observar(evento);
                return Some(
                    MensajeSaliente::respuesta_libre(
                        &testigo,
                        &evento.conversacion,
                        respuesta_local.contenido,
                    )
                    .expect("la conversación coincide siempre"),
                );
            }
            Err(error) => {
                // Política fail-closed: a diferencia de la deduplicación que es fail-open (duplicar
                // un mensaje es barato, gastar saldo no contabilizado no lo es), ante un error de
                // almacenamiento al consultar o reservar presupuesto no se realiza la llamada al
                // proveedor de inferencia para evitar consumo sin registro contable.
                emitir(
                    EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                        .con_id_conversacion(evento.conversacion.como_str())
                        .con_detalle(format!(
                            "fallo al reservar presupuesto de inferencia: {error}"
                        )),
                );
                return None;
            }
        };

        let peticion = PeticionDeInferencia {
            conversacion: evento.conversacion.clone(),
            contenido: evento.contenido.clone(),
        };

        match self.proveedor.generar(peticion).await {
            Ok(respuesta) => {
                match self.repositorio.conciliar_presupuesto(
                    id_reserva,
                    respuesta.unidades_consumidas,
                    evento.marca_temporal,
                ) {
                    Ok(ResultadoDeResolucion::Resuelta {
                        deficit_no_cubierto,
                        ..
                    }) => {
                        if deficit_no_cubierto > 0 {
                            emitir(
                                EntradaDeRegistro::nueva(
                                    NivelDeRegistro::Aviso,
                                    "presupuesto_deficit_no_cubierto",
                                )
                                .con_id_conversacion(evento.conversacion.como_str())
                                .con_detalle(format!("déficit no cubierto: {deficit_no_cubierto}")),
                            );
                        }
                    }
                    Ok(ResultadoDeResolucion::ReservaNoActiva) => {
                        // Inalcanzable en la ruta normal del procesador porque la reserva se
                        // acaba de crear en esta misma llamada; la variante se cubre en tests.
                    }
                    Err(error) => {
                        emitir(
                            EntradaDeRegistro::nueva(
                                NivelDeRegistro::Error,
                                "fallo_de_persistencia",
                            )
                            .con_id_conversacion(evento.conversacion.como_str())
                            .con_detalle(format!(
                                "fallo al conciliar presupuesto de inferencia: {error}"
                            )),
                        );
                    }
                }

                let testigo = TestigoDeEntrante::observar(evento);
                Some(
                    MensajeSaliente::respuesta_libre(
                        &testigo,
                        &evento.conversacion,
                        respuesta.contenido,
                    )
                    .expect("la conversación coincide siempre"),
                )
            }
            Err(_averia) => {
                if let Err(error) = self
                    .repositorio
                    .liberar_presupuesto(id_reserva, evento.marca_temporal)
                {
                    emitir(
                        EntradaDeRegistro::nueva(NivelDeRegistro::Error, "fallo_de_persistencia")
                            .con_id_conversacion(evento.conversacion.como_str())
                            .con_detalle(format!(
                                "fallo al liberar presupuesto de inferencia: {error}"
                            )),
                    );
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inferencia::ErrorDeInferenciaSimulada;
    use crate::registro;
    use hexcell_core::identidad::{IdConversacion, IdDeduplicacion, IdRemitente};
    use hexcell_core::inferencia::RespuestaDeInferencia;
    use hexcell_storage::GestorDePools;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    /// Proveedor mínimo de prueba: si llegara a invocarse con saldo insuficiente el test de más
    /// abajo fallaría por otra vía (un envío inesperado), así que basta con que cumpla el trait.
    #[derive(Clone, Copy, Default)]
    struct ProveedorDePrueba;

    impl ProveedorDeInferencia for ProveedorDePrueba {
        type Error = ErrorDeInferenciaSimulada;

        async fn generar(
            &self,
            peticion: PeticionDeInferencia,
        ) -> Result<RespuestaDeInferencia, Self::Error> {
            Ok(RespuestaDeInferencia {
                contenido: peticion.contenido,
                unidades_consumidas: 0,
            })
        }
    }

    /// Proveedor de prueba con consumo personalizable para forzar déficit.
    #[derive(Clone, Copy)]
    struct ProveedorDeExceso {
        unidades: u64,
    }

    impl ProveedorDeInferencia for ProveedorDeExceso {
        type Error = ErrorDeInferenciaSimulada;

        async fn generar(
            &self,
            peticion: PeticionDeInferencia,
        ) -> Result<RespuestaDeInferencia, Self::Error> {
            Ok(RespuestaDeInferencia {
                contenido: peticion.contenido,
                unidades_consumidas: self.unidades,
            })
        }
    }

    fn evento_de_prueba(conversacion: &IdConversacion) -> EventoEntrante {
        EventoEntrante {
            remitente: IdRemitente::nuevo("remitente-de-prueba"),
            conversacion: conversacion.clone(),
            contenido: "contenido de prueba".to_string(),
            marca_temporal: SystemTime::UNIX_EPOCH,
            deduplicacion: IdDeduplicacion::nuevo("dedup-presupuesto-rechazado"),
        }
    }

    /// Contador de directorios temporales de este proceso. Sustituye a la lectura de nanosegundos
    /// del reloj: su granularidad no garantiza unicidad, así que dos ayudantes construidos a la vez
    /// en hilos distintos podían leer el mismo instante y compartir directorio. Un contador atómico
    /// los distingue **por construcción**, sin depender del reloj del sistema.
    static SECUENCIA_DE_DIRECTORIOS: AtomicU64 = AtomicU64::new(0);

    /// Mitad de AC-2 que el test de integración `crates/hexcell/tests/inferencia.rs` no puede
    /// cubrir: `registro::pruebas` es `pub(crate)`, así que solo un test dentro de este crate
    /// puede comprobar que el rechazo de presupuesto deja la entrada `presupuesto_rechazado`,
    /// igual que `motor.rs` comprueba `admision_descartada` y `concurrencia_descartada`.
    #[tokio::test]
    async fn saldo_insuficiente_deja_registro_presupuesto_rechazado() {
        let id_unico = SECUENCIA_DE_DIRECTORIOS.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("hx-proc-{}-{}", std::process::id(), id_unico));
        // Antes este error se descartaba con `let _ =` y el fallo aparecía después, disfrazado de
        // fallo al abrir el pool. Nombrar la ruta y el error de origen es lo único que hace
        // diagnosticable un fallo intermitente a partir de la salida del test.
        if let Err(error) = std::fs::create_dir_all(&dir) {
            panic!(
                "no se pudo crear el directorio temporal del test «{}»: {error}",
                dir.display()
            );
        }
        let pools = match GestorDePools::abrir(&dir) {
            Ok(p) => p,
            Err(error) => panic!(
                "no se pudo abrir el gestor de pools sobre «{}»: {error:?}",
                dir.display()
            ),
        };
        let repositorio = Arc::new(RepositorioDeSesiones::nuevo(Arc::new(pools)));
        // El saldo inicial es 0 por defecto: cualquier estimación de coste mayor lo rechaza.

        let procesador = ProcesadorDeInferencia::nuevo(ProveedorDePrueba, repositorio);
        let conversacion = IdConversacion::nuevo("conversacion-sin-saldo");

        registro::pruebas::instalar();
        let resultado = procesador.procesar(&evento_de_prueba(&conversacion)).await;
        let registros = registro::pruebas::tomar();

        assert!(
            resultado.is_some(),
            "con saldo insuficiente el procesador debe generar respuesta en modo degradado"
        );
        if let Some(MensajeSaliente::RespuestaLibre { texto, .. }) = resultado {
            assert_eq!(
                texto,
                crate::reglas_locales::TEXTO_DE_RESPUESTA_DEGRADADA,
                "la respuesta debe ser el texto degradado"
            );
        } else {
            panic!("se esperaba una respuesta libre con el texto degradado");
        }

        let rechazo = registros
            .iter()
            .find(|entrada| entrada.evento == "presupuesto_rechazado");
        assert!(
            rechazo.is_some(),
            "debe existir una entrada de registro para presupuesto_rechazado"
        );
        assert_eq!(
            rechazo.unwrap().id_conversacion.as_deref(),
            Some("conversacion-sin-saldo")
        );

        let degradado = registros
            .iter()
            .find(|entrada| entrada.evento == "modo_degradado");
        assert!(
            degradado.is_some(),
            "debe existir una entrada de registro para modo_degradado"
        );
        assert_eq!(
            degradado.unwrap().id_conversacion.as_deref(),
            Some("conversacion-sin-saldo")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn deficit_no_cubierto_deja_registro_presupuesto_deficit_no_cubierto() {
        let id_unico = SECUENCIA_DE_DIRECTORIOS.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("hx-proc-def-{}-{}", std::process::id(), id_unico));
        // Antes este error se descartaba con `let _ =` y el fallo aparecía después, disfrazado de
        // fallo al abrir el pool. Nombrar la ruta y el error de origen es lo único que hace
        // diagnosticable un fallo intermitente a partir de la salida del test.
        if let Err(error) = std::fs::create_dir_all(&dir) {
            panic!(
                "no se pudo crear el directorio temporal del test «{}»: {error}",
                dir.display()
            );
        }
        let pools = match GestorDePools::abrir(&dir) {
            Ok(p) => p,
            Err(error) => panic!(
                "no se pudo abrir el gestor de pools sobre «{}»: {error:?}",
                dir.display()
            ),
        };
        let repositorio = Arc::new(RepositorioDeSesiones::nuevo(Arc::new(pools)));
        let conversacion = IdConversacion::nuevo("conversacion-deficit");

        repositorio
            .anotar_entrante(
                &conversacion,
                &IdRemitente::nuevo("remitente-deficit"),
                "mensaje inicial",
                SystemTime::UNIX_EPOCH,
            )
            .expect("anotar mensaje entrante");

        repositorio
            .aportar_presupuesto(5, SystemTime::UNIX_EPOCH)
            .expect("aportar saldo");

        let procesador =
            ProcesadorDeInferencia::nuevo(ProveedorDeExceso { unidades: 100 }, repositorio);

        registro::pruebas::instalar();
        let resultado = procesador.procesar(&evento_de_prueba(&conversacion)).await;
        let registros = registro::pruebas::tomar();

        assert!(resultado.is_some());
        let deficit = registros
            .iter()
            .find(|entrada| entrada.evento == "presupuesto_deficit_no_cubierto");
        assert!(
            deficit.is_some(),
            "debe existir una entrada de registro para presupuesto_deficit_no_cubierto"
        );
        assert_eq!(
            deficit.unwrap().id_conversacion.as_deref(),
            Some("conversacion-deficit")
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
