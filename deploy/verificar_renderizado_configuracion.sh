#!/usr/bin/env bash
# Guardia de configuración por célula (HEX-081). Comprueba éxito y mutaciones de rechazo.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${HEXCELL_ADMIN_BIN:-$RAIZ/target/debug/hexcell-admin}"
DEF="$RAIZ/deploy/celula.defecto.env.ejemplo"
SUP="$RAIZ/deploy/celula.superposicion.env.ejemplo"
if [ ! -x "$BIN" ]; then echo "FALLA: no existe el binario $BIN" >&2; exit 1; fi
if [ ! -f "$DEF" ] || [ ! -f "$SUP" ]; then echo "FALLA: faltan archivos de ejemplo" >&2; exit 1; fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

verificar_normal() {
    local salida="$tmp/render.env"
    if ! "$BIN" config render --defecto "$DEF" --superposicion "$SUP" --salida "$salida"; then
        echo "FALLA: el renderizado de los ejemplos falló"; return 1
    fi
    [ -s "$salida" ] || { echo "FALLA: el renderizado no produjo salida"; return 1; }
    grep -q '^HEXCELL_ID_CELULA=celula-ejemplo-01$' "$salida" || { echo "FALLA: no prevaleció la superposición"; return 1; }
    echo "PASA: renderizado normal"; return 0
}

verificar_mutacion() {
    local nombre="$1" mutada="$2" salida="$tmp/$1.env"
    if "$BIN" config render --defecto "$DEF" --superposicion "$mutada" --salida "$salida" >/dev/null 2>&1; then
        echo "FALLA: la mutación $nombre fue aceptada"; return 1
    fi
    if [ -e "$salida" ]; then echo "FALLA: la mutación $nombre dejó salida"; return 1; fi
    echo "PASA: mutación $nombre rechazada"; return 0
}

if [ "${1:-}" = "--autoprueba" ]; then
    verificar_normal || exit 1
    cp "$SUP" "$tmp/desconocida.env"; printf '\nHEXCELL_CLAVE_INVENTADA=valor\n' >> "$tmp/desconocida.env"
    verificar_mutacion "clave-desconocida" "$tmp/desconocida.env" || exit 1
    cp "$SUP" "$tmp/invalida.env"; sed -i 's/^HEXCELL_TELEFONO_CELULA=.*/HEXCELL_TELEFONO_CELULA=/' "$tmp/invalida.env"
    verificar_mutacion "valor-invalido" "$tmp/invalida.env" || exit 1
else
    verificar_normal || exit 1
fi
