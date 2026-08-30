-- Tercera migración de knowledge_staging.db / knowledge_epoch_N.db / knowledge_live.db
-- (versión 3 de PRAGMA user_version).
--
-- Esta migración introduce la tabla singleton `sonda_semantica` para persistir la sonda
-- semántica de validación (vector y su correspondiente umbral de aceptación) requerida para
-- la compuerta de integridad de la época.
--
-- ─── POR QUÉ UNA TABLA SEPARADA Y NO COLUMNAS EN METADATOS_DE_EPOCA ─────────────────────────────
--
-- SQLite no permite añadir restricciones CHECK a nivel de tabla mediante ALTER TABLE. Acoplar
-- la sonda y su umbral (ambos presentes o ninguno) dentro de `metadatos_de_epoca` forzaría una
-- reconstrucción destructiva de la tabla dentro del corredor de migraciones (`unchecked_transaction`),
-- donde `PRAGMA foreign_keys` permanece inerte y la integridad referencial se perdería en silencio.
-- Dos columnas NOT NULL dentro de una única fila opcional en una tabla independiente codifican
-- exactamente ese acoplamiento de todo o nada sin requerir ninguna reconstrucción.
--
-- ─── CONTRATO DEL VECTOR DE LA SONDA ─────────────────────────────────────────────────────────────
--
-- La columna `vector` hereda el mismo contrato binario fijado en la migración 0002: una secuencia
-- de números IEEE-754 binary32 en orden little-endian, sin cabecera ni relleno, cuya longitud
-- en bytes debe ser un múltiplo positivo de 4. No se introduce ninguna fila semilla: una base
-- recién migrada mantiene esta tabla vacía hasta que una ingesta real compute y guarde la sonda.
--
-- Diseñado el 30 de agosto de 2026 para la persistencia de la compuerta de integridad de la época.

CREATE TABLE sonda_semantica (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),
    texto_de_la_sonda     TEXT NOT NULL,
    vector                BLOB NOT NULL CHECK (length(vector) > 0 AND length(vector) % 4 = 0),
    umbral_de_aceptacion  REAL NOT NULL,
    registrada_ms         INTEGER NOT NULL
) STRICT;
