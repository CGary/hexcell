-- Esquema inicial de adapter_identity.db (versión 1 de PRAGMA user_version).
--
-- Almacén de identidad del adaptador (adr-0010, puntos 5 y 6): el mapa entre el contacto que
-- conoce el adaptador y el identificador interno de conversación que le asignó. Vive separado del
-- sqlstore del sidecar -la otra base que completa las cuatro del respaldo- para sobrevivir a un
-- re-emparejamiento tras una desvinculación con dispositivo retirado, que obliga a descartar el
-- sqlstore pero nunca debería destruir a qué hilo pertenece cada contacto.
--
-- Las dos columnas son texto opaco a propósito: esta capa no construye, no interpreta y no
-- invierte el identificador interno que guarda, solo lo persiste tal y como el adaptador ya lo
-- decidió (mismo criterio que sessions.db aplica a sus propias claves opacas).
--
-- STRICT por la misma razón que el resto de esquemas de este crate: sin ella, un error de
-- escritura se descubre por su tipo semanas después en vez de al ejecutar la sentencia.
CREATE TABLE identidades_de_contacto (
    contacto              TEXT NOT NULL PRIMARY KEY,
    identificador_interno TEXT NOT NULL
) STRICT;
