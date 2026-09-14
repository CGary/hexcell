#!/usr/bin/env bash
# ============================================================================
# Emisor del ping de vigilancia externa (dead-man's switch) — HEX-077-d
# ============================================================================
# Este script es el ÚNICO actor de la tarea HEX-077-d. Emite, una vez por
# invocación, un único GET saliente a la URL que lee de la variable de entorno
# HEXCELL_URL_PING_VIGILANCIA. Nada más: no notifica, no consulta ninguna base,
# no abre ningún puerto de entrada.
#
# POR QUÉ EXISTE COMO SCRIPT DE deploy/ Y NO COMO SUBCOMANDO DE hexcell-admin NI
# COMO NUEVO BINARIO/CRATE: lo decide el razonamiento de "compartir el destino"
# (fate-sharing) del plan de la etapa A-6. El vigilante externo prueba que el
# SERVIDOR está vivo; el scheduler que debe disparar ese ping es el cron del
# propio sistema operativo, sobre el anfitrión que se atestigua a sí mismo. Si el
# anfitrión pierde alimentación, el kernel muere o la red se cae, cron se detiene
# y el ping se detiene —y ESA AUSENCIA es precisamente la alarma. Un demonio de
# larga duración o un temporizador dentro de un contenedor introducirían una
# forma de seguir haciendo ping mientras la carga real del anfitrión ya no está,
# o de morir independientemente de ella. Un subcomando de hexcell-admin, además,
# forzaría una pila TLS (rustls + hyper + tokio) en el único crate cuyo punto de
# diseño es casi-cero dependencias, para hablar https; este script añade CERO
# dependencias de terceros. La notificación la dispara el servicio externo
# compatible con healthchecks.io al dejar de recibir el ping, nunca este script.
#
# POR QUÉ FALLA CERRADO (fail-closed): un servidor muerto no puede reportar que
# murió. Si la URL no está en el entorno, el script termina distinto de 0 SIN
# emitir ninguna petición; no hay URL por omisión ni respaldo. Un éxito silencioso
# aquí es el peor fallo posible de un vigilante: dejaría la alarma muda. Lo mismo
# si curl falta: se informa en español y se sale distinto de 0, nunca con 0.
#
# POR QUÉ SIN --retry NI BACKOFF LOCAL: la tolerancia a un traspié transitorio es
# del servicio externo (su periodo de gracia), y está fuera del alcance de esta
# tarea. Un reintento local permitiría que un anfitrión degradado siguiera
# pareciendo sano al cruzar el límite del intervalo de 5 minutos, que es
# exactamente la ventana que la alarma debe cubrir.
#
# CONTRATO DE SALIDA: el estado de curl se propaga al estado del script. Un ping
# fallido (host caído, puerto cerrado, HTTP 4xx/5xx con --fail) sale distinto de
# 0, porque el silencio hacia afuera es la alarma buscada.
#
# DEPENDENCIAS EN TIEMPO DE EJECUCIÓN: {bash, curl}. Nada de python3: el
# anfitrión de producción no necesita python3 para que el vigilante funcione.
# ============================================================================

set -u

# --- Lectura de la URL (falla cerrado si no está) ---------------------------

# exigir_url_de_vigilancia
#   Lee HEXCELL_URL_PING_VIGILANCIA exclusivamente del entorno del proceso.
#   Devuelve la URL por stdout y código 0 si está definida y no vacía; si no,
#   imprime el motivo en español a stderr y devuelve 1 (cero peticiones emitidas).
exigir_url_de_vigilancia() {
    local url="${HEXCELL_URL_PING_VIGILANCIA:-}"
    if [ -z "$url" ]; then
        echo "FALLA: HEXCELL_URL_PING_VIGILANCIA no está definida o está vacía;" \
             "el vigilante externo no puede emitir el ping (fail-closed)" >&2
        return 1
    fi
    printf '%s' "$url"
}

# --- Emisión del ping -------------------------------------------------------

# emitir_ping <url>
#   Envía exactamente un GET saliente con curl. --max-time 60s queda holgado por
#   debajo del periodo de 300s del cron para que dos corridas nunca se solapen.
#   --fail hace que curl salga distinto de 0 ante HTTP 4xx/5xx, así un ping
#   entregado pero rechazado por el servicio no se reporta como éxito. El estado
#   de curl se propaga al llamador; no se enmascara.
emitir_ping() {
    local url="$1"

    if ! command -v curl >/dev/null 2>&1; then
        echo "FALLA: curl no está disponible en este sistema; el vigilante" \
             "externo no puede emitir el ping" >&2
        return 127
    fi

    curl --silent --show-error --fail --max-time 60 "$url" >/dev/null
}

# --- Punto de entrada -------------------------------------------------------

main() {
    local url
    url="$(exigir_url_de_vigilancia)" || exit 1
    emitir_ping "$url"
    exit $?
}

main "$@"
