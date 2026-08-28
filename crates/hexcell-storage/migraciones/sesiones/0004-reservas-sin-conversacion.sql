-- Cuarta migración de sessions.db (versión 4 de PRAGMA user_version).
--
-- Esta migración flexibiliza la definición de la tabla `reservas` para que el campo
-- `id_conversacion` sea nullable (NULL). Esto permite reservar presupuesto para la ingesta
-- de catálogos sin asociarlo a ninguna conversación y sin crear registros ficticios que
-- distorsionen las estadísticas reales.
--
-- ─── POR QUÉ SE USA DEFER_FOREIGN_KEYS Y NO FOREIGN_KEYS = OFF ───────────────────────────
--
-- El corredor de migraciones en Rust ejecuta cada paso dentro de una transacción ya abierta
-- (`unchecked_transaction`). En SQLite, `PRAGMA foreign_keys = OFF` es un no-op si se invoca
-- dentro de una transacción activa. Por tanto, desactivar claves foráneas no es una opción aquí.
-- Sin embargo, `PRAGMA defer_foreign_keys = ON` sí toma efecto dentro de una transacción, posponiendo
-- la validación de claves foráneas hasta el momento del COMMIT. Esto nos permite recrear y
-- renombrar la tabla `reservas` sin que las filas referenciadas en `movimientos` aborten la transacción
-- de forma inmediata.
--
-- ─── POR QUÉ NO SE RECREA NI MODIFICA LA TABLA DE MOVIMIENTOS ──────────────────────────────
--
-- Recrear la tabla `movimientos` implicaría transcribir manualmente su definición DDL, lo cual
-- introduce el riesgo de perder o relajar de forma silenciosa alguna de sus seis restricciones de
-- integridad. La decisión de diseño del 27 de agosto de 2026 determinó que esto no es necesario:
-- basta con alterar y renombrar únicamente `reservas`.
--
-- ─── POR QUÉ SE REQUIERE LA COMPUERTA DE INTEGRIDAD EXPLICITA (GATE) ──────────────────────
--
-- SQLite no valida las restricciones diferidas durante la ejecución de sentencias intermedias,
-- y además `PRAGMA foreign_key_check` solo devuelve filas de error en lugar de provocar un aborto.
-- Si ejecutáramos `PRAGMA defer_foreign_keys = OFF` directamente, SQLite descartaría de forma
-- silenciosa cualquier violación pendiente en lugar de verificarla, permitiendo confirmar una base
-- de datos corrupta con filas de movimientos huérfanas.
--
-- Por lo tanto, se introduce una compuerta activa previa: un UPDATE sobre la columna STRICT INTEGER
-- `saldo.disponible`. Si `pragma_foreign_key_check` detecta alguna inconsistencia, el CASE intenta
-- asignar una cadena de texto (TEXT) a esta columna entera. Al ser una tabla STRICT, SQLite aborta
-- inmediatamente la sentencia y toda la transacción se revierte de forma atómica. Si todo está limpio,
-- asigna `disponible` a sí mismo, resultando en un no-op seguro.
--

PRAGMA defer_foreign_keys = ON;

-- Eliminar la vista que depende de reservas para permitir su recreación.
DROP VIEW consumo_por_conversacion;

-- Reconstruir la tabla reservas eliminando la restricción NOT NULL de id_conversacion.
CREATE TABLE reservas_nueva (
    id              INTEGER PRIMARY KEY,
    id_conversacion TEXT    REFERENCES conversaciones(id_conversacion),
    monto_reservado INTEGER NOT NULL CHECK (monto_reservado > 0),
    estado          TEXT    NOT NULL CHECK (estado IN ('activa', 'conciliada', 'liberada')),
    creada_ms       INTEGER NOT NULL,
    resuelta_ms     INTEGER,
    CHECK ((estado = 'activa') = (resuelta_ms IS NULL))
) STRICT;

-- Copiar los datos históricos desde la tabla antigua.
INSERT INTO reservas_nueva (id, id_conversacion, monto_reservado, estado, creada_ms, resuelta_ms)
SELECT id, id_conversacion, monto_reservado, estado, creada_ms, resuelta_ms
FROM reservas;

-- Intercambiar las tablas.
DROP TABLE reservas;
ALTER TABLE reservas_nueva RENAME TO reservas;

-- Recrear el índice para barrido de reservas activas.
CREATE INDEX idx_reservas_activas ON reservas (estado, creada_ms);

-- Compuerta de integridad: fuerza el aborto del paso si existen violaciones de clave foránea.
-- Este UPDATE evalúa pragma_foreign_key_check y asigna texto a un entero estricto si hay fallos.
UPDATE saldo
SET disponible = CASE
    WHEN (SELECT count(*) FROM pragma_foreign_key_check) = 0 THEN disponible
    ELSE 'Violacion de clave foranea detectada al reconstruir la tabla reservas en la migracion 0004'
END
WHERE id = 1;

-- Desactivar el diferimiento de claves foráneas tras pasar la compuerta de integridad de forma segura.
-- Esto limpia el estado diferido para permitir el COMMIT de la transacción de la migración.
PRAGMA defer_foreign_keys = OFF;

-- Recrear la vista de consumo por conversación excluyendo las reservas de ingesta sin conversación.
CREATE VIEW consumo_por_conversacion AS
SELECT
    r.id_conversacion,
    SUM(CASE WHEN r.estado = 'conciliada' THEN r.monto_reservado - COALESCE(m.monto, 0) ELSE 0 END) AS unidades_consumidas
FROM reservas AS r
LEFT JOIN movimientos AS m ON m.id_reserva = r.id AND m.clase = 'conciliacion'
WHERE r.id_conversacion IS NOT NULL
GROUP BY r.id_conversacion;

-- Crear la vista de consumo de ingesta para agrupar únicamente las reservas sin conversación.
-- Un agregado SUM sin GROUP BY siempre devuelve exactamente una fila. Si no hay filas coincidentes,
-- el resultado de SUM es NULL. Envolvemos el resultado en COALESCE(..., 0) para asegurar que la vista
-- devuelva siempre un entero (0 si no hay consumos), evitando fallos en la lectura desde Rust.
CREATE VIEW consumo_de_ingesta AS
SELECT
    COALESCE(SUM(CASE WHEN r.estado = 'conciliada' THEN r.monto_reservado - COALESCE(m.monto, 0) ELSE 0 END), 0) AS unidades_consumidas
FROM reservas AS r
LEFT JOIN movimientos AS m ON m.id_reserva = r.id AND m.clase = 'conciliacion'
WHERE r.id_conversacion IS NULL;
