//! Gestión contable de saldo, reservas y movimientos en `sessions.db`.
//!
//! Implementa las operaciones del esquema financiero en dos fases de FR-10:
//! reserva previa de presupuesto antes de llamar al proveedor de inferencia, consulta de saldo
//! y aportes iniciales.

use std::time::SystemTime;

use hexcell_core::identidad::IdConversacion;
use hexcell_core::presupuesto::UnidadesDePresupuesto;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::error::ErrorDeAlmacen;
use crate::sesiones::RepositorioDeSesiones;
use crate::tiempo::a_milisegundos;

/// Estado actual del saldo de la célula.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saldo {
    /// Unidades disponibles para gasto inmediato.
    pub disponible: i64,
    /// Unidades retenidas en reservas activas pendientes de conciliación.
    pub reservado: i64,
}

/// Veredicto del intento de reserva de presupuesto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VeredictoDeReserva {
    /// La reserva fue concedida y registrada exitosamente.
    Concedida {
        /// Identificador de la fila de reserva en la tabla `reservas`.
        id_reserva: i64,
        /// Cantidad de unidades retenidas.
        monto_reservado: i64,
    },
    /// La reserva fue rechazada por falta de saldo disponible suficiente.
    Rechazada {
        /// Saldo disponible en el momento del rechazo.
        disponible: i64,
        /// Unidades requeridas para la reserva.
        requerido: i64,
    },
}

/// Resultado de la resolución (conciliación o liberación) de una reserva de presupuesto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultadoDeResolucion {
    /// La reserva fue resuelta (conciliada o liberada) exitosamente.
    Resuelta {
        /// Ajuste neto aplicado al saldo disponible.
        ajuste_aplicado: i64,
        /// Parte del déficit por sobreconsumo que no pudo ser cargada al saldo disponible por falta de fondos.
        deficit_no_cubierto: i64,
    },
    /// La reserva no existe o ya no se encuentra en estado `'activa'`.
    ReservaNoActiva,
}

impl RepositorioDeSesiones {
    /// Intenta reservar de forma atómica una cantidad de unidades de presupuesto.
    ///
    /// Todo ocurre dentro de **una** única transacción SQLite sobre `sessions.db`:
    /// 1. Verificación de saldo disponible.
    /// 2. Inserción de la reserva con estado `'activa'` en la tabla `reservas`.
    /// 3. Actualización de la tabla `saldo` (decremento de disponible, incremento de reservado).
    /// 4. Registro del movimiento en la tabla `movimientos` con clase `'reserva'` y monto negativo.
    pub fn reservar_presupuesto(
        &self,
        id_conversacion: &IdConversacion,
        unidades: UnidadesDePresupuesto,
        marca_temporal: SystemTime,
    ) -> Result<VeredictoDeReserva, ErrorDeAlmacen> {
        let marca_ms = a_milisegundos(marca_temporal);
        let unidades_i64 = i64::try_from(unidades).unwrap_or(i64::MAX);

        self.pools.sesiones().con_escritura(|conexion| {
            let transaccion = conexion
                .unchecked_transaction()
                .map_err(ErrorDeAlmacen::en("abrir la transacción de reserva de presupuesto"))?;

            let disponible: i64 = transaccion
                .query_row(
                    "SELECT disponible FROM saldo WHERE id = 1",
                    [],
                    |fila| fila.get(0),
                )
                .map_err(ErrorDeAlmacen::en("consultar el saldo disponible"))?;

            if disponible < unidades_i64 {
                return Ok(VeredictoDeReserva::Rechazada {
                    disponible,
                    requerido: unidades_i64,
                });
            }

            transaccion
                .execute(
                    "INSERT INTO reservas (id_conversacion, monto_reservado, estado, creada_ms, resuelta_ms) \
                     VALUES (?1, ?2, 'activa', ?3, NULL)",
                    params![id_conversacion.como_str(), unidades_i64, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("insertar la reserva de presupuesto"))?;

            let id_reserva = transaccion.last_insert_rowid();

            transaccion
                .execute(
                    "UPDATE saldo SET disponible = disponible - ?1, reservado = reservado + ?1, actualizado_ms = ?2 \
                     WHERE id = 1",
                    params![unidades_i64, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("actualizar el saldo tras reserva"))?;

            let saldo_resultante = disponible - unidades_i64;

            transaccion
                .execute(
                    "INSERT INTO movimientos (id_reserva, id_conversacion, clase, monto, saldo_resultante, registrado_ms) \
                     VALUES (?1, ?2, 'reserva', ?3, ?4, ?5)",
                    params![
                        id_reserva,
                        id_conversacion.como_str(),
                        -unidades_i64,
                        saldo_resultante,
                        marca_ms
                    ],
                )
                .map_err(ErrorDeAlmacen::en("registrar el movimiento de reserva"))?;

            transaccion
                .commit()
                .map_err(ErrorDeAlmacen::en("confirmar la reserva de presupuesto"))?;

            Ok(VeredictoDeReserva::Concedida {
                id_reserva,
                monto_reservado: unidades_i64,
            })
        })
    }

    /// Aporta unidades de presupuesto al saldo disponible.
    ///
    /// Se ejecuta en una única transacción SQLite: actualiza el saldo disponible y añade un
    /// registro a la tabla `movimientos` con clase `'aporte'`.
    pub fn aportar_presupuesto(
        &self,
        unidades: UnidadesDePresupuesto,
        marca_temporal: SystemTime,
    ) -> Result<(), ErrorDeAlmacen> {
        if unidades == 0 {
            return Ok(());
        }

        let marca_ms = a_milisegundos(marca_temporal);
        let unidades_i64 = i64::try_from(unidades).unwrap_or(i64::MAX);

        self.pools.sesiones().con_escritura(|conexion| {
            let transaccion = conexion
                .unchecked_transaction()
                .map_err(ErrorDeAlmacen::en("abrir la transacción de aporte de presupuesto"))?;

            transaccion
                .execute(
                    "UPDATE saldo SET disponible = disponible + ?1, actualizado_ms = ?2 WHERE id = 1",
                    params![unidades_i64, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("incrementar el saldo disponible"))?;

            let saldo_resultante: i64 = transaccion
                .query_row(
                    "SELECT disponible FROM saldo WHERE id = 1",
                    [],
                    |fila| fila.get(0),
                )
                .map_err(ErrorDeAlmacen::en("consultar el saldo resultante tras aporte"))?;

            transaccion
                .execute(
                    "INSERT INTO movimientos (id_reserva, id_conversacion, clase, monto, saldo_resultante, registrado_ms) \
                     VALUES (NULL, NULL, 'aporte', ?1, ?2, ?3)",
                    params![unidades_i64, saldo_resultante, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("registrar el movimiento de aporte"))?;

            transaccion
                .commit()
                .map_err(ErrorDeAlmacen::en("confirmar el aporte de presupuesto"))?;

            Ok(())
        })
    }

    /// Consulta la instantánea actual del saldo disponible y reservado.
    pub fn saldo(&self) -> Result<Saldo, ErrorDeAlmacen> {
        self.pools.sesiones().con_lectura(|conexion| {
            conexion
                .query_row(
                    "SELECT disponible, reservado FROM saldo WHERE id = 1",
                    [],
                    |fila| {
                        Ok(Saldo {
                            disponible: fila.get(0)?,
                            reservado: fila.get(1)?,
                        })
                    },
                )
                .map_err(ErrorDeAlmacen::en("consultar el saldo"))
        })
    }

    /// Indica si el libro de movimientos de presupuesto no tiene ningún registro.
    ///
    /// Devuelve `true` si no se ha realizado ningún movimiento (aporte ni reserva), lo cual permite
    /// inicializar la semilla de presupuesto una sola vez en el arranque.
    pub fn presupuesto_sin_iniciar(&self) -> Result<bool, ErrorDeAlmacen> {
        self.pools.sesiones().con_lectura(|conexion| {
            let cantidad: i64 = conexion
                .query_row("SELECT COUNT(*) FROM movimientos", [], |fila| fila.get(0))
                .map_err(ErrorDeAlmacen::en(
                    "consultar cantidad de movimientos de presupuesto",
                ))?;
            Ok(cantidad == 0)
        })
    }

    /// Concilia una reserva activa de presupuesto tras la ejecución exitosa de una inferencia.
    ///
    /// Transición de estado a `'conciliada'` dentro de **una** única transacción SQLite sobre `sessions.db`:
    /// - Si la cantidad consumida `M` es menor que la reservada `N`, el excedente `(N - M)` se devuelve a disponible.
    /// - Si la cantidad consumida `M` excede la reservada `N`, el déficit `(M - N)` se carga a disponible acotado por el saldo disponible existente (sin violar `disponible >= 0`). La fracción del déficit no cubierta se devuelve en `ResultadoDeResolucion::Resuelta.deficit_no_cubierto` y deliberadamente **no** se registra en `movimientos` (la migración 0002 solo admite `'aporte'`, `'reserva'`, `'conciliacion'` y `'liberacion'`).
    /// - Si la variación neta sobre disponible es cero (`M == N` o déficit sin saldo disponible), se actualiza la reserva y el saldo sin insertar fila en `movimientos`, respetando la restricción `CHECK (monto <> 0)`.
    /// - Si la reserva no existe o no está en estado `'activa'`, devuelve [`ResultadoDeResolucion::ReservaNoActiva`].
    pub fn conciliar_presupuesto(
        &self,
        id_reserva: i64,
        unidades_consumidas: UnidadesDePresupuesto,
        marca_temporal: SystemTime,
    ) -> Result<ResultadoDeResolucion, ErrorDeAlmacen> {
        let marca_ms = a_milisegundos(marca_temporal);
        let consumidas_i64 = i64::try_from(unidades_consumidas).unwrap_or(i64::MAX);

        self.pools.sesiones().con_escritura(|conexion| {
            let transaccion = conexion
                .unchecked_transaction()
                .map_err(ErrorDeAlmacen::en("abrir la transacción de conciliación de presupuesto"))?;

            let fila_reserva: Option<(String, i64)> = transaccion
                .query_row(
                    "SELECT id_conversacion, monto_reservado FROM reservas WHERE id = ?1 AND estado = 'activa'",
                    params![id_reserva],
                    |fila| Ok((fila.get(0)?, fila.get(1)?)),
                )
                .optional()
                .map_err(ErrorDeAlmacen::en("consultar la reserva activa para conciliación"))?;

            let Some((id_conversacion, monto_reservado)) = fila_reserva else {
                return Ok(ResultadoDeResolucion::ReservaNoActiva);
            };

            let disponible_actual: i64 = transaccion
                .query_row(
                    "SELECT disponible FROM saldo WHERE id = 1",
                    [],
                    |fila| fila.get(0),
                )
                .map_err(ErrorDeAlmacen::en("consultar el saldo disponible para conciliación"))?;

            let (ajuste_aplicado, deficit_no_cubierto) = if consumidas_i64 <= monto_reservado {
                let excedente = monto_reservado - consumidas_i64;
                (excedente, 0)
            } else {
                let deficit_total = consumidas_i64 - monto_reservado;
                if disponible_actual >= deficit_total {
                    (-deficit_total, 0)
                } else {
                    let cargo_posible = disponible_actual;
                    let no_cubierto = deficit_total - cargo_posible;
                    (-cargo_posible, no_cubierto)
                }
            };

            transaccion
                .execute(
                    "UPDATE reservas SET estado = 'conciliada', resuelta_ms = ?2 WHERE id = ?1",
                    params![id_reserva, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("actualizar el estado de la reserva a conciliada"))?;

            transaccion
                .execute(
                    "UPDATE saldo SET disponible = disponible + ?1, reservado = reservado - ?2, actualizado_ms = ?3 WHERE id = 1",
                    params![ajuste_aplicado, monto_reservado, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("actualizar el saldo tras conciliación"))?;

            if ajuste_aplicado != 0 {
                let saldo_resultante: i64 = transaccion
                    .query_row(
                        "SELECT disponible FROM saldo WHERE id = 1",
                        [],
                        |fila| fila.get(0),
                    )
                    .map_err(ErrorDeAlmacen::en("consultar el saldo resultante tras conciliación"))?;

                transaccion
                    .execute(
                        "INSERT INTO movimientos (id_reserva, id_conversacion, clase, monto, saldo_resultante, registrado_ms) \
                         VALUES (?1, ?2, 'conciliacion', ?3, ?4, ?5)",
                        params![
                            id_reserva,
                            id_conversacion,
                            ajuste_aplicado,
                            saldo_resultante,
                            marca_ms
                        ],
                    )
                    .map_err(ErrorDeAlmacen::en("registrar el movimiento de conciliación"))?;
            }

            transaccion
                .commit()
                .map_err(ErrorDeAlmacen::en("confirmar la conciliación de presupuesto"))?;

            Ok(ResultadoDeResolucion::Resuelta {
                ajuste_aplicado,
                deficit_no_cubierto,
            })
        })
    }

    /// Libera una reserva activa de presupuesto tras un fallo o cancelación del proveedor de inferencia.
    ///
    /// Transición de estado a `'liberada'` dentro de **una** única transacción SQLite sobre `sessions.db`:
    /// - Se devuelve el monto total reservado a `saldo.disponible` y se reduce `saldo.reservado`.
    /// - Se inserta un movimiento con clase `'liberacion'` y monto positivo igual al monto reservado.
    /// - Si la reserva no existe o no está en estado `'activa'`, devuelve [`ResultadoDeResolucion::ReservaNoActiva`].
    pub fn liberar_presupuesto(
        &self,
        id_reserva: i64,
        marca_temporal: SystemTime,
    ) -> Result<ResultadoDeResolucion, ErrorDeAlmacen> {
        let marca_ms = a_milisegundos(marca_temporal);

        self.pools.sesiones().con_escritura(|conexion| {
            let transaccion = conexion
                .unchecked_transaction()
                .map_err(ErrorDeAlmacen::en("abrir la transacción de liberación de presupuesto"))?;

            let fila_reserva: Option<(String, i64)> = transaccion
                .query_row(
                    "SELECT id_conversacion, monto_reservado FROM reservas WHERE id = ?1 AND estado = 'activa'",
                    params![id_reserva],
                    |fila| Ok((fila.get(0)?, fila.get(1)?)),
                )
                .optional()
                .map_err(ErrorDeAlmacen::en("consultar la reserva activa para liberación"))?;

            let Some((id_conversacion, monto_reservado)) = fila_reserva else {
                return Ok(ResultadoDeResolucion::ReservaNoActiva);
            };

            transaccion
                .execute(
                    "UPDATE reservas SET estado = 'liberada', resuelta_ms = ?2 WHERE id = ?1",
                    params![id_reserva, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("actualizar el estado de la reserva a liberada"))?;

            transaccion
                .execute(
                    "UPDATE saldo SET disponible = disponible + ?1, reservado = reservado - ?1, actualizado_ms = ?2 WHERE id = 1",
                    params![monto_reservado, marca_ms],
                )
                .map_err(ErrorDeAlmacen::en("actualizar el saldo tras liberación"))?;

            let saldo_resultante: i64 = transaccion
                .query_row(
                    "SELECT disponible FROM saldo WHERE id = 1",
                    [],
                    |fila| fila.get(0),
                )
                .map_err(ErrorDeAlmacen::en("consultar el saldo resultante tras liberación"))?;

            transaccion
                .execute(
                    "INSERT INTO movimientos (id_reserva, id_conversacion, clase, monto, saldo_resultante, registrado_ms) \
                     VALUES (?1, ?2, 'liberacion', ?3, ?4, ?5)",
                    params![
                        id_reserva,
                        id_conversacion,
                        monto_reservado,
                        saldo_resultante,
                        marca_ms
                    ],
                )
                .map_err(ErrorDeAlmacen::en("registrar el movimiento de liberación"))?;

            transaccion
                .commit()
                .map_err(ErrorDeAlmacen::en("confirmar la liberación de presupuesto"))?;

            Ok(ResultadoDeResolucion::Resuelta {
                ajuste_aplicado: monto_reservado,
                deficit_no_cubierto: 0,
            })
        })
    }
}
