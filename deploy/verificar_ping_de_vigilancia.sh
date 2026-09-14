#!/usr/bin/env bash
# ============================================================================
# Guardia mecánico del ping de vigilancia externa (HEX-077-d)
# ============================================================================
# Ancla, de forma mecánica, las propiedades que el emisor
# deploy/ping_de_vigilancia_externa.sh debe cumplir, y las comprueba contra un
# sumidero HTTP local en 127.0.0.1 (sin DNS, sin salir del loopback, sin cuenta
# de healthchecks.io real). Cuatro casos:
#
#   - caso_un_solo_ping        : exactamente UN GET y código 0 con la URL en el
#                                entorno apuntando al sumidero local.
#   - caso_falla_cerrada_sin_url: sin la variable, código distinto de 0 y CERO
#                                peticiones (fail-closed, sin URL de respaldo).
#   - caso_no_enmascara_fallo  : contra un puerto cerrado, código distinto de 0
#                                (un ping fallido no se reporta como éxito).
#   - caso_higiene_de_secreto  : ni celula.env.ejemplo ni cell.compose.yml ganan
#                                la variable, y el emisor no lleva URL embebida.
#
# USO
#
#   deploy/verificar_ping_de_vigilancia.sh
#       Corre los cuatro casos sobre el emisor real. Sale 0 si pasan todos.
#
#   deploy/verificar_ping_de_vigilancia.sh --autoprueba
#       Copia el emisor a un directorio temporal y ROMPE UNA propiedad por vez
#       (doble ping, URL de respaldo embebida, "|| true" que traga el estado de
#       curl). Verifica que el guardia FALLA sobre cada copia mutada: si alguna
#       mutación pasara, el guardia no es todavía un guardia. Prueba de mutación
#       exigida por AC-1/AC-2 del 00-spec.yaml.
#
# CADA CASO imprime su propia línea PASA/FALLA nombrando el caso; un recuento
# agregado desnudo no es evidencia aceptable.
#
# DEPENDENCIAS
#
#   - bash, sed, grep, mktemp, rm                  (POSIX/Util-linux estándar)
#   - curl                                         (el propio emisor lo exige)
#   - python3                                       (solo el sumidero de prueba)
#
# Un entorno sin curl o sin python3 NO se declara verificado: el guardia falla
# con mensaje explícito, porque "omitido" sería indistinguible de "pasa".
# ============================================================================

set -u

EMISOR_REAL="deploy/ping_de_vigilancia_externa.sh"

# --- Prerrequisitos ---------------------------------------------------------

if [ ! -f "$EMISOR_REAL" ]; then
    echo "FALLA: el emisor [$EMISOR_REAL] no existe" >&2
    exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
    echo "FALLA: curl no está disponible; el guardia no puede ejecutar el emisor" >&2
    exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "FALLA: python3 no está disponible; el guardia no puede levantar el sumidero local" >&2
    exit 1
fi

# Directorio temporal de toda la ejecución del guardia; se limpia al salir.
GUARD_TMP="$(mktemp -d -t hex077-guard.XXXXXX)"
# shellcheck disable=SC2064  # expandir $GUARD_TMP ahora, no en la trampa
trap "rm -rf '$GUARD_TMP'" EXIT

# --- Sumidero HTTP local de un solo proceso ---------------------------------

# levantar_sumidero_local
#   Arranca un servidor HTTP efímero en 127.0.0.1 (puerto efímero) que cuenta los
#   GET recibidos y responde 200. Deja el puerto en $SUMIDERO_URL y el contador
#   en el archivo $SUMIDERO_CONTADOR; guarda el PID en $SUMIDERO_PID.
levantar_sumidero_local() {
    local dir_tmp
    dir_tmp="$GUARD_TMP/sumidero"
    mkdir -p "$dir_tmp"

    SUMIDERO_CONTADOR="$dir_tmp/contador"
    local puerto_archivo="$dir_tmp/puerto"
    : > "$SUMIDERO_CONTADOR"
    export HEX_SUMIDERO_CONTADOR="$SUMIDERO_CONTADOR"

    python3 - "$puerto_archivo" <<'PY' &
import os, sys, http.server, socketserver

puerto_archivo = sys.argv[1]
contador = os.environ["HEX_SUMIDERO_CONTADOR"]

class Manejador(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        try:
            with open(contador) as f:
                n = int(f.read() or "0")
        except FileNotFoundError:
            n = 0
        n += 1
        with open(contador, "w") as f:
            f.write(str(n))
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(b"ok")
    def log_message(self, *a):
        pass

with socketserver.TCPServer(("127.0.0.1", 0), Manejador) as httpd:
    with open(puerto_archivo, "w") as f:
        f.write(str(httpd.server_address[1]))
    httpd.serve_forever()
PY
    SUMIDERO_PID=$!

    # Esperar a que el servidor escriba su puerto (sin salir del loopback).
    local intentos=0
    while [ ! -s "$puerto_archivo" ] && [ "$intentos" -lt 50 ]; do
        sleep 0.1
        intentos=$((intentos + 1))
    done
    if [ ! -s "$puerto_archivo" ]; then
        echo "FALLA: el sumidero local no arrancó" >&2
        kill "$SUMIDERO_PID" 2>/dev/null
        return 1
    fi

    local puerto
    puerto="$(cat "$puerto_archivo")"
    SUMIDERO_URL="http://127.0.0.1:${puerto}/"
    export SUMIDERO_URL
    return 0
}

# detener_sumidero
#   Termina el servidor efímero y limpia el contador.
detener_sumidero() {
    [ -n "${SUMIDERO_PID:-}" ] && kill "$SUMIDERO_PID" 2>/dev/null
    SUMIDERO_PID=""
    SUMIDERO_URL=""
}

# leer_contador
#   Devuelve el número de GET que el sumidero ha registrado (0 si el archivo
#   está vacío o ausente).
leer_contador() {
    local n=0
    [ -f "${SUMIDERO_CONTADOR:-}" ] && n="$(cat "${SUMIDERO_CONTADOR}" 2>/dev/null)"
    echo "${n:-0}"
}

# --- Casos ------------------------------------------------------------------

# caso_un_solo_ping <ruta-emisor>
#   Con la URL en el entorno apuntando al sumidero local, el emisor debe enviar
#   EXACTAMENTE un GET y salir 0.
caso_un_solo_ping() {
    local emisor="$1"
    levantar_sumidero_local || return 1
    local ok=1

    HEXCELL_URL_PING_VIGILANCIA="$SUMIDERO_URL" bash "$emisor"
    local estado=$?

    if [ "$estado" -ne 0 ]; then
        echo "FALLA: caso_un_solo_ping -> el emisor salió con código $estado (se esperaba 0)"
        ok=0
    fi
    if [ "$(leer_contador)" -ne 1 ]; then
        echo "FALLA: caso_un_solo_ping -> el sumidero registró $(leer_contador) petición(es), se esperaba exactamente 1"
        ok=0
    fi
    detener_sumidero
    [ "$ok" -eq 1 ] && return 0 || return 1
}

# caso_falla_cerrada_sin_url <ruta-emisor>
#   Sin la variable de entorno, el emisor debe salir distinto de 0 y no emitir
#   NINGUNA petición (fail-closed, sin URL de respaldo).
caso_falla_cerrada_sin_url() {
    local emisor="$1"
    levantar_sumidero_local || return 1
    local ok=1

    env -u HEXCELL_URL_PING_VIGILANCIA bash "$emisor"
    local estado=$?

    if [ "$estado" -eq 0 ]; then
        echo "FALLA: caso_falla_cerrada_sin_url -> el emisor salió 0 sin URL (debió fallar cerrado)"
        ok=0
    fi
    if [ "$(leer_contador)" -ne 0 ]; then
        echo "FALLA: caso_falla_cerrada_sin_url -> el sumidero registró $(leer_contador) petición(es) sin URL (debió ser 0)"
        ok=0
    fi
    detener_sumidero
    [ "$ok" -eq 1 ] && return 0 || return 1
}

# caso_no_enmascara_fallo <ruta-emisor>
#   Contra un puerto cerrado, el emisor debe salir distinto de 0: un ping
#   fallido no se reporta nunca como éxito.
caso_no_enmascara_fallo() {
    local emisor="$1"
    # Puerto efímero casi seguro cerrado en loopback; no levantamos sumidero.
    local url_caida="http://127.0.0.1:1/"

    HEXCELL_URL_PING_VIGILANCIA="$url_caida" bash "$emisor"
    local estado=$?

    if [ "$estado" -eq 0 ]; then
        echo "FALLA: caso_no_enmascara_fallo -> el emisor salió 0 contra un puerto cerrado (debió fallar)"
        return 1
    fi
    echo "PASA: caso_no_enmascara_fallo -> el emisor sale distinto de 0 contra un puerto cerrado"
    return 0
}

# caso_higiene_de_secreto <ruta-emisor>
#   Las superficies per-célula no ganan la variable, y el emisor no lleva URL
#   embebida (http:// o https://). Recibe la ruta del emisor a revisar.
caso_higiene_de_secreto() {
    local emisor="$1"
    local ok=1

    if grep -qE 'HEXCELL_URL_PING_VIGILANCIA' deploy/celula.env.ejemplo; then
        echo "FALLA: caso_higiene_de_secreto -> celula.env.ejemplo ganó HEXCELL_URL_PING_VIGILANCIA"
        ok=0
    fi
    if grep -qE 'HEXCELL_URL_PING_VIGILANCIA' deploy/cell.compose.yml; then
        echo "FALLA: caso_higiene_de_secreto -> cell.compose.yml ganó HEXCELL_URL_PING_VIGILANCIA"
        ok=0
    fi
    if grep -qE 'https?://' "$emisor"; then
        echo "FALLA: caso_higiene_de_secreto -> el emisor lleva una URL embebida (http(s)://)"
        ok=0
    fi

    [ "$ok" -eq 1 ] && return 0 || return 1
}

# --- Modo normal ------------------------------------------------------------

if [ "${1:-}" != "--autoprueba" ]; then
    if [ -n "${1:-}" ]; then
        echo "Uso: $0 [--autoprueba]" >&2
        exit 2
    fi

    echo "Verificación mecánica del emisor de vigilancia externa (HEX-077-d):"
    TOTAL=0; ACIERTOS=0

    TOTAL=$((TOTAL + 1)); if caso_un_solo_ping "$EMISOR_REAL"; then
        echo "PASA: caso_un_solo_ping"; ACIERTOS=$((ACIERTOS + 1))
    else
        echo "FALLA: caso_un_solo_ping"
    fi

    TOTAL=$((TOTAL + 1)); if caso_falla_cerrada_sin_url "$EMISOR_REAL"; then
        echo "PASA: caso_falla_cerrada_sin_url"; ACIERTOS=$((ACIERTOS + 1))
    else
        echo "FALLA: caso_falla_cerrada_sin_url"
    fi

    TOTAL=$((TOTAL + 1)); if caso_no_enmascara_fallo "$EMISOR_REAL"; then
        ACIERTOS=$((ACIERTOS + 1))
    else
        echo "FALLA: caso_no_enmascara_fallo"
    fi

    TOTAL=$((TOTAL + 1)); if caso_higiene_de_secreto "$EMISOR_REAL"; then
        echo "PASA: caso_higiene_de_secreto"; ACIERTOS=$((ACIERTOS + 1))
    else
        echo "FALLA: caso_higiene_de_secreto"
    fi

    echo ""
    echo "Resumen: $ACIERTOS/$TOTAL casos pasan"
    if [ "$ACIERTOS" -eq "$TOTAL" ]; then
        echo "OK: el emisor cumple el contrato de vigilancia externa"
        exit 0
    fi
    echo "FALLA: el emisor no cumple el contrato de vigilancia externa"
    exit 1
fi

# --- Modo --autoprueba (prueba de mutación) ---------------------------------

echo "Modo --autoprueba: cada propiedad anclada, una por vez, debe ser detectada al romperse."
TOTAL=0; ACIERTOS=0

# 1) Doble ping: el emisor mutado emite dos GET; caso_un_solo_ping debe ponerse rojo.
COPIA="$GUARD_TMP/emisor.doble.sh"
cp "$EMISOR_REAL" "$COPIA"
# Duplicar la llamada a emitir_ping dentro de main.
sed -i -E 's/^(\s*)emitir_ping "\$url"$/\1emitir_ping "$url"\n\1emitir_ping "$url"/' "$COPIA"
if ! caso_un_solo_ping "$COPIA" >/dev/null 2>&1; then
    echo "PASA: doble ping -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: doble ping -> el guardia PASÓ la copia mutada (no es un guardia)"
fi
TOTAL=$((TOTAL + 1))

# 2) URL de respaldo embebida: el emisor mutado usa ${VAR:-https://...}; tanto
#    caso_falla_cerrada_sin_url como caso_higiene_de_secreto deben ponerse rojos.
COPIA="$GUARD_TMP/emisor.respaldo.sh"
cp "$EMISOR_REAL" "$COPIA"
sed -i -E 's#\$\{HEXCELL_URL_PING_VIGILANCIA:-\}#${HEXCELL_URL_PING_VIGILANCIA:-https://hc-ping.com/HEX-077-d-ficticio}#' "$COPIA"
if ! caso_falla_cerrada_sin_url "$COPIA" >/dev/null 2>&1; then
    echo "PASA: URL de respaldo embebida -> caso_falla_cerrada_sin_url falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: URL de respaldo embebida -> caso_falla_cerrada_sin_url PASÓ la copia mutada"
fi
TOTAL=$((TOTAL + 1))

if ! caso_higiene_de_secreto "$COPIA" >/dev/null 2>&1; then
    echo "PASA: URL de respaldo embebida -> caso_higiene_de_secreto falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: URL de respaldo embebida -> caso_higiene_de_secreto PASÓ la copia mutada"
fi
TOTAL=$((TOTAL + 1))

# 3) "|| true" que traga el estado de curl; caso_no_enmascara_fallo debe ponerse rojo.
COPIA="$GUARD_TMP/emisor.silencia.sh"
cp "$EMISOR_REAL" "$COPIA"
sed -i -E 's#^(\s*)curl --silent --show-error --fail --max-time 60 "\$url"$#\1curl --silent --show-error --fail --max-time 60 "$url" || true#' "$COPIA"
if ! caso_no_enmascara_fallo "$COPIA" >/dev/null 2>&1; then
    echo "PASA: '|| true' que traga curl -> caso_no_enmascara_fallo falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: '|| true' que traga curl -> caso_no_enmascara_fallo PASÓ la copia mutada"
fi
TOTAL=$((TOTAL + 1))

echo ""
echo "Resumen autoprueba: $ACIERTOS/$TOTAL mutaciones detectadas"
if [ "$ACIERTOS" -eq "$TOTAL" ]; then
    echo "OK: el guardia falla bajo cada mutación"
    exit 0
fi
echo "FALLA: el guardia no detectó todas las mutaciones"
exit 1
