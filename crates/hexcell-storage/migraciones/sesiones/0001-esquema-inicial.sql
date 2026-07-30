-- Esquema inicial de sessions.db (versión 1 de PRAGMA user_version).
--
-- sessions.db es la base de lectura y escritura caliente de la persistencia dual de FR-05: aquí
-- viven el registro de deduplicación y el historial de conversación, que hasta HEX-005 estaban
-- solo en memoria. No hay aquí ni una sola columna con un identificador de transporte crudo: la
-- única clave de conversación es el IdConversacion interno y la única clave de contacto es el
-- IdRemitente interno, ambos recibidos ya traducidos por el adaptador de canal (adr-0010).
--
-- Todas las tablas se declaran STRICT. Sin STRICT, SQLite acepta cualquier valor en cualquier
-- columna por su afinidad de tipos, y un error de escritura se descubre semanas después leyendo
-- un entero donde debía haber texto. STRICT convierte ese error en un fallo inmediato de la
-- sentencia que lo cometió.
--
-- Los instantes se guardan como enteros de milisegundos desde el epoch Unix, no como texto ISO
-- ni como segundos: milisegundos porque el horizonte de deduplicación necesita ordenar dos
-- eventos llegados dentro del mismo segundo, y entero porque es el tipo que SQLite compara e
-- indexa más barato en el hardware objetivo.

-- Contactos vistos por esta célula, en identidad interna.
CREATE TABLE contactos (
    id_remitente          TEXT    PRIMARY KEY,
    primera_actividad_ms  INTEGER NOT NULL,
    ultima_actividad_ms   INTEGER NOT NULL
) STRICT;

-- Hilos de conversación. Una conversación puede tener varios contactos (un grupo), y por eso su
-- identidad se declara aparte de la del contacto y no como una columna suya.
CREATE TABLE conversaciones (
    id_conversacion      TEXT    PRIMARY KEY,
    creada_ms            INTEGER NOT NULL,
    ultima_actividad_ms  INTEGER NOT NULL
) STRICT;

-- Historial de la conversación: lo que entró y lo que salió, en el orden en que el motor lo
-- procesó. El orden lo da la clave primaria entera autoincremental y no la marca temporal,
-- porque dos eventos pueden compartir marca y el historial debe reproducirse siempre igual.
--
-- `id_remitente` admite NULL a propósito: un mensaje saliente lo produce la célula, no un
-- contacto, y rellenarlo con un valor centinela sería inventar un remitente que no existe.
CREATE TABLE mensajes (
    id                 INTEGER PRIMARY KEY,
    id_conversacion    TEXT    NOT NULL REFERENCES conversaciones(id_conversacion),
    id_remitente       TEXT    REFERENCES contactos(id_remitente),
    direccion          TEXT    NOT NULL CHECK (direccion IN ('entrante', 'saliente')),
    clase              TEXT    NOT NULL CHECK (clase IN ('texto', 'plantilla')),
    contenido          TEXT    NOT NULL,
    marca_temporal_ms  INTEGER NOT NULL
) STRICT;

-- Parámetros posicionales de un mensaje saliente de clase 'plantilla'. Se guardan en una tabla
-- hija en vez de serializados en una sola columna porque la lista es ordenada y de longitud
-- variable, y una columna de texto con separadores rompería en cuanto un parámetro contuviera
-- el separador. `posicion` forma parte de la clave: el orden es dato, no presentación.
CREATE TABLE parametros_de_plantilla (
    id_mensaje  INTEGER NOT NULL REFERENCES mensajes(id),
    posicion    INTEGER NOT NULL,
    valor       TEXT    NOT NULL,
    PRIMARY KEY (id_mensaje, posicion)
) STRICT;

-- Identificadores de deduplicación ya procesados dentro de la ventana de retención vigente.
CREATE TABLE deduplicacion (
    id_deduplicacion   TEXT    PRIMARY KEY,
    marca_temporal_ms  INTEGER NOT NULL
) STRICT;

-- Estado escalar del motor que debe sobrevivir a un reinicio. Hoy guarda una sola clave, el
-- horizonte monótono de deduplicación: el máximo instante recibido por el canal, contra el que
-- se mide la poda. Es una tabla clave-valor y no una tabla de una fila con columnas fijas
-- porque cada valor nuevo debe poder añadirse sin migrar el esquema de los anteriores.
CREATE TABLE estado_del_motor (
    clave  TEXT    PRIMARY KEY,
    valor  INTEGER NOT NULL
) STRICT;

-- Recuperar el historial de una conversación en orden es la lectura más frecuente de la célula;
-- sin este índice cada lectura recorrería la tabla entera de mensajes.
CREATE INDEX idx_mensajes_conversacion ON mensajes (id_conversacion, id);

-- La poda de deduplicación borra por rango de marca temporal en cada evento entrante; sin este
-- índice, el borrado sería un recorrido completo de la tabla en el camino caliente.
CREATE INDEX idx_deduplicacion_marca ON deduplicacion (marca_temporal_ms);
