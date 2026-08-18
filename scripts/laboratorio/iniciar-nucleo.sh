#!/usr/bin/env sh
# Inicia el núcleo Rust de hexcell como proceso directo para el laboratorio (tarea 15 de A-3).
# AVISO: Este es el arnés de laboratorio; el empaquetado en Docker pertenece a la etapa A-6.

set -e

DIR_SCRIPT="$(cd "$(dirname "$0")" && pwd)"
RAIZ_REPOSITORIO="$(cd "$DIR_SCRIPT/../.." && pwd)"

# Cargar entorno compartido
. "$DIR_SCRIPT/entorno.ejemplo.sh"

# Asegurar que la ruta de datos existe
mkdir -p "$HEXCELL_RUTA_DATOS"
mkdir -p "$(dirname "$HEXCELL_SOCKET_IPC")"

echo "hexcell-lab: iniciando núcleo Rust (célula: $HEXCELL_ID_CELULA, canal: $HEXCELL_CANAL)..."
echo "hexcell-lab: ruta de datos en $HEXCELL_RUTA_DATOS"
echo "hexcell-lab: socket IPC en $HEXCELL_SOCKET_IPC"
echo "hexcell-lab: salud en $HEXCELL_DIRECCION_SALUD"

cd "$RAIZ_REPOSITORIO"
exec cargo run -p hexcell
