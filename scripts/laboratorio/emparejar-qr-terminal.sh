#!/usr/bin/env sh
# Emparejamiento QR en terminal para el laboratorio: dibuja cada código QR
# con qrencode en cuanto el núcleo lo emite (rotan cada ~20 s, plazo 120 s).
# Requiere: sidecar ya corriendo y qrencode instalado.

set -e

DIR_SCRIPT="$(cd "$(dirname "$0")" && pwd)"
RAIZ_REPOSITORIO="$(cd "$DIR_SCRIPT/../.." && pwd)"

. "$DIR_SCRIPT/entorno.ejemplo.sh"

cd "$RAIZ_REPOSITORIO"
cargo run -p hexcell -- emparejar --metodo qr 2>&1 | while IFS= read -r linea; do
    case "$linea" in
        "Código QR recibido (cadena cruda): "*)
            cadena="${linea#Código QR recibido (cadena cruda): }"
            clear
            echo "Escanea AHORA (WhatsApp → Dispositivos vinculados). Se renueva solo:"
            qrencode -t ANSIUTF8 "$cadena"
            ;;
        *) echo "$linea" ;;
    esac
done
