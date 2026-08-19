#!/usr/bin/env sh
# Respaldar la célula de laboratorio (HEX-029, tarea 18 de A-3).
# Disciplina operacional obligatoria:
# - El NÚCLEO de la célula debe estar DETENIDO (detenido con Ctrl-C/SIGTERM en iniciar-nucleo.sh)
# - El SIDECAR debe continuar EN EJECUCIÓN (iniciar-sidecar.sh) para responder a la orden de respaldo IPC.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RAIZ_REPOSITORIO="$(cd "$SCRIPT_DIR/../.." && pwd)"

. "$SCRIPT_DIR/entorno.ejemplo.sh"

DESTINO_BASE="${HEXCELL_RUTA_RESPALDOS:-$HEXCELL_LAB_DIR/respaldos}"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
DESTINO="$DESTINO_BASE/respaldo-$TIMESTAMP"

mkdir -p "$DESTINO"

echo "hexcell-lab: iniciando respaldo de la célula $HEXCELL_ID_CELULA..."
echo "hexcell-lab: destino del respaldo: $DESTINO"
echo "hexcell-lab: recordatorio de disciplina operacional obligatoria: núcleo detenido, sidecar en ejecución."

HEXCELL_BIN="${HEXCELL_BIN:-cargo run -p hexcell --}"

cd "$RAIZ_REPOSITORIO"
$HEXCELL_BIN respaldar --directorio "$DESTINO"
