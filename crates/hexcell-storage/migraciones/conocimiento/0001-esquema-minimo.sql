-- Esquema mínimo de knowledge_live.db (versión 1 de PRAGMA user_version).
--
-- Deliberadamente mínimo: **el esquema real de la base de conocimiento se diseña en la etapa
-- A-5**, la que introduce la Shadow DB y la conmutación atómica por épocas (FR-07, adr-0006).
-- Adelantar aquí ese diseño sería fijar sin contraste la parte del producto que esa etapa existe
-- para decidir.
--
-- Esta única tabla existe por una razón operativa concreta y no como maqueta de nada: el pool de
-- conocimiento abre el archivo en modo SQLITE_OPEN_READ_ONLY, y abrir en solo lectura un archivo
-- que no existe falla. Así que la célula crea la base una vez en lectura y escritura, aplica
-- esta migración, la cierra y solo entonces abre el pool de solo lectura. La sonda de vitalidad
-- necesita además alguna tabla real contra la que lanzar su consulta barata.
CREATE TABLE metadatos_de_conocimiento (
    clave  TEXT PRIMARY KEY,
    valor  TEXT NOT NULL
) STRICT;
