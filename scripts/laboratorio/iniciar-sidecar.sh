#!/usr/bin/env sh
# Inicia el sidecar Go de whatsmeow como proceso directo para el laboratorio (tarea 15 de A-3).
# AVISO: Este es el arnés de laboratorio; el empaquetado en Docker pertenece a la etapa A-6.

set -e

DIR_SCRIPT="$(cd "$(dirname "$0")" && pwd)"
RAIZ_REPOSITORIO="$(cd "$DIR_SCRIPT/../.." && pwd)"

# Cargar entorno compartido
. "$DIR_SCRIPT/entorno.ejemplo.sh"

# Asegurar directorios requeridos
mkdir -p "$(dirname "$HEXCELL_SOCKET_IPC")"
mkdir -p "$HEXCELL_RUTA_DATOS"

echo "hexcell-lab: iniciando sidecar Go (célula: $HEXCELL_ID_CELULA)..."
echo "hexcell-lab: socket IPC en $HEXCELL_SOCKET_IPC"
echo "hexcell-lab: sqlstore en $HEXCELL_RUTA_SQLSTORE"
echo "hexcell-lab: identidad en $HEXCELL_RUTA_IDENTIDAD"

cd "$RAIZ_REPOSITORIO/sidecar"
exec go run .
