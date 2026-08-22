#!/usr/bin/env sh
# Restablece el estado de pruebas de UN contacto en identidad.db del laboratorio:
# borra su fila de cortacircuitos y de presentacion_de_conversacion para poder
# volver a probar desde cero (presentación + respuestas del bot).
#
# NUNCA toca baja_de_contacto: esa tabla es la lista STOP de consentimiento
# (HEX-032); revivir una baja exige una decisión humana explícita, no un script
# de pruebas. La superficie definitiva de operador pertenece a la etapa A-6.
#
# Uso:
#   restablecer-contacto.sh              -> lista los contactos con estado
#   restablecer-contacto.sh <ct-...>     -> restablece ese contacto

set -e

DIR_SCRIPT="$(cd "$(dirname "$0")" && pwd)"
. "$DIR_SCRIPT/entorno.ejemplo.sh"

BASE="$HEXCELL_RUTA_IDENTIDAD"
if [ ! -f "$BASE" ]; then
    echo "restablecer-contacto: no existe la base de identidad en $BASE" >&2
    echo "(¿olvidó fijar HEXCELL_LAB_DIR?)" >&2
    exit 1
fi

if [ -z "$1" ]; then
    echo "Contactos con estado de cortacircuitos:"
    sqlite3 -readonly "$BASE" "SELECT '  ' || id_interno || ' (disparado: ' || COALESCE(disparado_en_ms,'no') || ')' FROM cortacircuitos LIMIT 50;"
    echo "Contactos ya presentados:"
    sqlite3 -readonly "$BASE" "SELECT '  ' || id_interno FROM presentacion_de_conversacion LIMIT 50;"
    echo "Uso: $0 <ct-...>"
    exit 0
fi

CONTACTO="$1"
sqlite3 "$BASE" "DELETE FROM cortacircuitos WHERE id_interno = '$CONTACTO'; DELETE FROM presentacion_de_conversacion WHERE id_interno = '$CONTACTO';"
echo "restablecer-contacto: contacto $CONTACTO restablecido (cortacircuitos y presentación)."
echo "AVISO: baja_de_contacto NO se toca por diseño (lista STOP, HEX-032)."
