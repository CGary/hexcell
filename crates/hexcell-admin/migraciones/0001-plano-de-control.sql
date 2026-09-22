-- Migración 0001: esquema inicial del almacén del plano de control.
--
-- Tres tablas que sostienen el estado de control de cada célula, el historial
-- de transiciones y el registro de sustituciones de número. Ninguna columna
-- guarda un identificador de transporte ni un número de teléfono: el plano de
-- control conoce la célula por su id interno y por su estado, no por el canal.
--
-- El PRAGMA user_version lo fija el corredor de migraciones en la misma
-- transacción que este guion, igual que en crates/hexcell-storage.

-- Estado actual de cada célula conocida. `motivo` documenta por qué la célula
-- llegó a ese estado (alta_implicita, sesion_cerrada, etc.); vacío por omisión.
CREATE TABLE celulas (
    id TEXT PRIMARY KEY,
    estado TEXT NOT NULL,
    motivo TEXT NOT NULL DEFAULT '',
    actualizado_ms INTEGER NOT NULL
);

-- Historial ordenado de cada transición de estado. `de` puede ser vacío en la
-- primera transición de una célula dada de alta implícita; en ese caso la
-- columna guarda la cadena vacía.
CREATE TABLE transiciones (
    id INTEGER PRIMARY KEY,
    id_celula TEXT NOT NULL,
    de TEXT NOT NULL,
    a TEXT NOT NULL,
    motivo TEXT NOT NULL,
    registrado_ms INTEGER NOT NULL
);

-- Registro auditable de sustituciones de número por célula. Esta tarea crea la
-- tabla y sólo la lee; la tarea 13 (cell rebind) es la que escribe en ella.
CREATE TABLE sustituciones (
    id INTEGER PRIMARY KEY,
    id_celula TEXT NOT NULL,
    motivo TEXT NOT NULL,
    registrado_ms INTEGER NOT NULL
);
