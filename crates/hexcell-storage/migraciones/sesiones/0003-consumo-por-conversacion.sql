-- Tercera migración de sessions.db (versión 3 de PRAGMA user_version).
--
-- Crea la vista de consumo por conversación para exponer el acumulado
-- de consumo de tokens por cada conversación de forma estable y consultable.
--
-- Por qué se ancla en 'reservas' y no en 'movimientos':
-- conciliar_presupuesto solo inserta una fila de conciliación si el ajuste
-- neto no es cero (monto <> 0). Si el consumo real coincide exactamente con
-- lo reservado, no se crea ningún registro en movimientos. Por lo tanto,
-- una consulta basada únicamente en movimientos reportaría cero consumo.
-- Al anclarse en reservas y hacer un LEFT JOIN con el movimiento de conciliación,
-- calculamos con precisión el consumo como `monto_reservado - COALESCE(monto, 0)`.
--
-- Limitación conocida por déficit no cubierto:
-- Si el consumo real supera la reserva y el saldo disponible es insuficiente
-- para cubrir el excedente, la transacción ajusta el disponible a cero y no
-- registra el déficit restante. Por lo tanto, esta vista subestima el consumo
-- real por exactamente la cantidad del déficit no cubierto.
--
-- Esta vista se declara sin IF NOT EXISTS, siguiendo la convención de la escalera
-- de migraciones donde cada paso se ejecuta una sola vez.

CREATE VIEW consumo_por_conversacion AS
SELECT
    r.id_conversacion,
    SUM(CASE WHEN r.estado = 'conciliada' THEN r.monto_reservado - COALESCE(m.monto, 0) ELSE 0 END) AS unidades_consumidas
FROM reservas AS r
LEFT JOIN movimientos AS m ON m.id_reserva = r.id AND m.clase = 'conciliacion'
GROUP BY r.id_conversacion;
