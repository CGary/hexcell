-- Segunda migración de knowledge_staging.db / knowledge_epoch_N.db / knowledge_live.db
-- (versión 2 de PRAGMA user_version).
--
-- Esta migración define el esquema real de la base de conocimiento para la etapa A-5:
-- documentos fuente, sus fragmentos de texto, los vectores de incrustación por fragmento
-- y los metadatos de la época. Las cuatro tablas comparten un único esquema con las tres
-- funciones del archivo (staging, época sellada, live de solo lectura), porque la distinción
-- entre roles la expresa el campo numero_de_epoca y no variantes del esquema.
--
-- ─── CONTRATO DE REPRESENTACIÓN DE VECTORES (sección normativa) ─────────────────────────────────
--
-- Diseño del formato de los vectores de incrustación:
-- Cada incrustación se almacena como un BLOB de valores IEEE-754 binary32 en orden little-endian,
-- sin cabecera, sin prefijo de longitud y sin relleno. El valor i-ésimo ocupa los bytes
-- 4*i .. 4*i+4 y el número de valores de punto flotante es exactamente length(vector) / 4.
-- Rust debe usar f32::to_le_bytes al serializar y f32::from_le_bytes al deserializar.
-- El orden little-endian se elige sobre el orden nativo porque los archivos de época son
-- copiados y restaurados por la ruta de respaldo de la etapa A-2, y nada dentro del archivo
-- registra la endianidad del escritor; un formato dependiente del procesador rompería la
-- portabilidad entre máquinas.
-- La búsqueda de similitud se realiza en Rust puro mediante coseno sobre todos los fragmentos
-- de la época, sin ninguna extensión de SQLite ni índice externo.
--
-- ─── CONTRATO DE IDENTIDAD INTRÍNSECA DE LA ÉPOCA ───────────────────────────────────────────────
--
-- El campo numero_de_epoca vive dentro del archivo para que una base restaurada o renombrada
-- pueda verificar su propia identidad: knowledge_epoch_N.db puede comprobarse contra el valor
-- que guarda en metadatos_de_epoca sin depender del nombre del archivo. El nombre es solo el
-- localizador; la fila es la descripción autoritativa.
-- NULL significa "en preparación, nunca promovida": así un único esquema sirve para
-- knowledge_staging.db (numero_de_epoca NULL), knowledge_epoch_N.db (numero_de_epoca = N)
-- y knowledge_live.db (enlace simbólico al época actual, solo lectura).
-- La tarea 8 (reversión a época anterior) depende de esta propiedad para verificar que el
-- archivo que está a punto de promover es realmente la época que afirma ser.
--
-- ─── LÍMITE DELIBERADO DEL CHECK DE LONGITUD ────────────────────────────────────────────────────
--
-- El CHECK de la tabla vectores_de_fragmento solo verifica que la longitud del BLOB sea
-- un múltiplo de 4, no que coincida con la dimensión registrada en metadatos_de_epoca.
-- Un CHECK no puede referenciar otra tabla, por lo que la verificación de uniformidad de
-- dimensión dentro de una época —que la tarea 5 implementará mediante la consulta
-- length(vector) <> 4 * (SELECT dimension_de_embedding FROM metadatos_de_epoca)— es un
-- defecto estructural diferido a ese validador, no un error que este esquema impida.

-- Documentos fuente. Cada fila representa un recurso externo indexado.
-- referencia_externa identifica el origen (p.ej. una URL o un identificador de fichero)
-- y debe ser único: si el mismo documento se reindexa, la tarea 4 reconstruye staging
-- desde cero y no actualiza filas existentes.
-- contenido guarda el texto fuente completo aunque los fragmentos lo repitan en trozos;
-- la tarea 5 necesita comprobar la cobertura de fragmentación contra el original, y la
-- tarea 9 puede ampliar un resultado a su documento completo.
-- actualizado_ms es el instante de última modificación del origen, en milisegundos Unix epoch.
CREATE TABLE documentos (
    id                  INTEGER PRIMARY KEY,
    referencia_externa  TEXT    NOT NULL UNIQUE,
    titulo              TEXT    NOT NULL,
    contenido           TEXT    NOT NULL,
    actualizado_ms      INTEGER NOT NULL
) STRICT;

-- Fragmentos de texto de un documento, ordenados por posición ordinal.
-- ordinal comienza en 0 y es único dentro del mismo documento, garantizado por la
-- restricción UNIQUE (id_documento, ordinal), que además genera el índice con
-- id_documento como columna más a la izquierda, el que usan las búsquedas por clave foránea.
-- La longitud mínima de texto (> 0) impide fragmentos vacíos.
-- ON DELETE CASCADE propaga el borrado del documento a sus fragmentos.
CREATE TABLE fragmentos (
    id           INTEGER PRIMARY KEY,
    id_documento INTEGER NOT NULL REFERENCES documentos(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL CHECK (ordinal >= 0),
    texto        TEXT    NOT NULL CHECK (length(texto) > 0),
    UNIQUE (id_documento, ordinal)
) STRICT;

-- Vector de incrustación de un fragmento. Relación uno a uno con fragmentos.
-- El BLOB sigue el contrato documentado arriba: f32 little-endian, longitud = 4 * dimension.
-- El CHECK verifica que el BLOB no esté vacío y que su longitud sea múltiplo de 4 (cuatro
-- bytes por valor f32), pero no puede verificar la uniformidad de dimensión entre fragmentos
-- de la misma época; esa responsabilidad pertenece al validador de la tarea 5.
-- ON DELETE CASCADE elimina el vector cuando se elimina su fragmento.
CREATE TABLE vectores_de_fragmento (
    id_fragmento  INTEGER PRIMARY KEY REFERENCES fragmentos(id) ON DELETE CASCADE,
    vector        BLOB    NOT NULL CHECK (length(vector) > 0 AND length(vector) % 4 = 0)
) STRICT;

-- Metadatos de la época. Singleton garantizado por CHECK (id = 1).
-- dimension_de_embedding registra el número de valores f32 por vector de esta época;
-- toda nueva época puede declarar una dimensión distinta, lo que permite cambiar de
-- modelo de incrustación sin alterar el esquema.
-- construida_ms es el instante de inicio de la construcción en staging.
-- sellada_ms es el instante de promoción; NULL mientras el archivo siga en staging.
-- El CHECK entre numero_de_epoca y sellada_ms garantiza que ambos campos son NULL o
-- ambos tienen valor, impidiendo épocas a medio promover.
-- La fila semilla (INSERT más abajo) establece la dimensión por defecto de 768 valores f32
-- (3 072 bytes por vector), elegida para que un catálogo de 2 000 fragmentos ocupe unos
-- 6 MB en vectores, dentro del presupuesto de 80 MB por célula en hardware objetivo.
CREATE TABLE metadatos_de_epoca (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),
    numero_de_epoca       INTEGER,
    dimension_de_embedding INTEGER NOT NULL CHECK (dimension_de_embedding > 0),
    construida_ms         INTEGER NOT NULL,
    sellada_ms            INTEGER,
    CHECK ((numero_de_epoca IS NULL) = (sellada_ms IS NULL))
) STRICT;

-- Fila semilla: staging recién creado, sin número de época, con dimensión 768.
-- Refleja el patrón de la migración 0002 de sesiones, que siembra el saldo inicial.
INSERT INTO metadatos_de_epoca (id, numero_de_epoca, dimension_de_embedding, construida_ms, sellada_ms)
VALUES (1, NULL, 768, unixepoch() * 1000, NULL);
