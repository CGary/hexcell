#!/usr/bin/env bash
# ============================================================================
# Verificación MANUAL y en VIVO del apagado ordenado de una célula
# (HEX-075, tarea 7 A-6)
# ============================================================================
# Prueba, con contenedores reales, que `docker stop` con margen de 30 s
# produce el apagado ordenado de la célula: código de salida 0 en ambos
# contenedores, sin recurrir a `SIGKILL`, y con el checkpoint del WAL del
# núcleo completado.
#
# ESTE SCRIPT NO ES UN GUARDIA MECÁNICO: levanta contenedores reales, tarda
# minutos y depende de un daemon Docker vivo. Por eso NUNCA se invoca desde
# .github/workflows/ci.yml ni desde verify.commands — deploy/verificar_senales.sh
# es el único guardia mecánico de esta tarea, y este script tampoco implementa
# el modo de autoprueba de mutación, exclusivo de ese guardia.
#
# QUÉ HACE
#   1. Crea una red y un volumen nombrados, VACÍOS y de un solo uso (sufijo
#      per-corrida), nunca reutilizados: un arranque en frío desde un volumen
#      que ya tiene datos no prueba nada.
#   2. Construye y levanta la célula desde la plantilla de composición con el
#      sidecar arrancando en frío, SIN emparejar: nunca se emite
#      `orden_emparejar` ni se pega una sesión real de WhatsApp.
#   3. Espera una señal de vida de cada contenedor —una línea de arranque
#      conocida en su stdout, NO un `GET /health/ready` 200—: una célula
#      deliberadamente sin emparejar nunca alcanza el estado "sesión de canal
#      activa" que exige la disposición completa, así que esperar un 200
#      colgaría el script o lo haría fallar por una razón ajena a esta tarea.
#   4. Emite `docker stop -t 30` sobre ambos contenedores.
#   5. Lee el código de salida y la bandera OOMKilled de ambos con
#      `docker inspect` y falla si alguno no salió 0, si alguno fue forzado
#      con SIGKILL (código 137) o si fue matado por falta de memoria.
#   6. Confirma el punto de control del WAL del núcleo: busca la línea
#      estructurada `punto_de_control_wal` en su stdout capturado y, como
#      confirmación adicional, inspecciona el tamaño de `sessions.db-wal` en
#      el volumen con un contenedor `alpine:3` efímero de solo lectura —la
#      misma imagen base que ya usan ambos Dockerfiles finales, no una
#      herramienta nueva— una vez que ningún contenedor de la célula sigue
#      vivo.
#   7. Borra los contenedores, la red y el volumen SIEMPRE (trampa en EXIT),
#      pase lo que pase.
#
# USO
#
#   deploy/verificar_apagado_ordenado.sh [ruta-plantilla]
#       Por omisión usa deploy/cell.compose.yml. Sale 0 si las tres
#       aserciones (AC-1, AC-2, AC-3 del 00-spec.yaml) pasan, distinto de 0
#       si alguna falla, con una línea `FALLA: ...` por cada motivo.
#
# DEPENDENCIAS
#
#   - bash, sed, mktemp, date                      (POSIX/Util-linux estándar)
#   - docker + docker compose                       (CLI v5.x verificado)
#   - deploy/celula.env.ejemplo como referente de variables
#
# Un entorno sin docker/compose NO se declara verificado: el script falla con
# un mensaje explícito en vez de omitir la comprobación.
# ============================================================================

set -u

PLANTILLA="${1:-deploy/cell.compose.yml}"

if [ ! -f "$PLANTILLA" ]; then
    echo "FALLA: la plantilla [$PLANTILLA] no existe" >&2
    exit 1
fi

if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
    echo "FALLA: docker compose no está disponible en este entorno; el ciclo vivo de docker stop no se puede verificar" >&2
    exit 1
fi

if [ ! -f deploy/celula.env.ejemplo ]; then
    echo "FALLA: deploy/celula.env.ejemplo no existe; no hay referente de variables para levantar la célula" >&2
    exit 1
fi

# --- Nombres de un solo uso --------------------------------------------------

SUFIJO="$(date +%s)-$$"
ID_CELULA="hex075verif${SUFIJO}"
RED="hex075-verif-red-${SUFIJO}"
VOLUMEN="hex075-verif-vol-${SUFIJO}"
PROYECTO="hex075verif${SUFIJO}"
CONTENEDOR_NUCLEO="${ID_CELULA}-nucleo"
CONTENEDOR_SIDECAR="${ID_CELULA}-sidecar"
ENV_TEMP="$(mktemp -t hex075-env.XXXXXX)"

limpiar() {
    echo ""
    echo "Limpiando: contenedores, red y volumen de un solo uso..."
    docker compose -p "$PROYECTO" --env-file "$ENV_TEMP" -f "$PLANTILLA" down --volumes --remove-orphans >/dev/null 2>&1 || true
    docker volume rm "$VOLUMEN" >/dev/null 2>&1 || true
    docker network rm "$RED" >/dev/null 2>&1 || true
    rm -f "$ENV_TEMP"
}
trap limpiar EXIT

# Copia deploy/celula.env.ejemplo y sustituye solo el identificador de
# célula, la red y el volumen por nombres únicos de esta corrida: la célula
# arranca en frío desde un volumen VACÍO cada vez, jamás reutilizado.
sed -E \
    -e "s/^HEXCELL_ID_CELULA=.*/HEXCELL_ID_CELULA=${ID_CELULA}/" \
    -e "s/^HEXCELL_RED_CELULA=.*/HEXCELL_RED_CELULA=${RED}/" \
    -e "s/^HEXCELL_VOLUMEN_CELULA=.*/HEXCELL_VOLUMEN_CELULA=${VOLUMEN}/" \
    deploy/celula.env.ejemplo >"$ENV_TEMP"

echo "Célula de verificación: ${ID_CELULA} (red ${RED}, volumen ${VOLUMEN})"
echo ""
echo "=== Arranque en frío desde volumen vacío, sidecar sin emparejar ==="

if ! docker compose -p "$PROYECTO" --env-file "$ENV_TEMP" -f "$PLANTILLA" up -d --build; then
    echo "FALLA: docker compose up no pudo levantar la célula"
    exit 1
fi

FALLAS=0

# --- Señal de vida: línea de arranque conocida, NO /health/ready -----------

# esperar_arranque <contenedor> <patrón>
esperar_arranque() {
    local contenedor="$1"
    local patron="$2"
    local intentos=30
    while [ "$intentos" -gt 0 ]; do
        if docker logs "$contenedor" 2>&1 | grep -q -- "$patron"; then
            return 0
        fi
        intentos=$((intentos - 1))
        sleep 1
    done
    return 1
}

if esperar_arranque "$CONTENEDOR_NUCLEO" 'hexcell: canal configurado: whatsmeow'; then
    echo "OK: el núcleo señaló arranque (canal whatsmeow configurado)"
else
    echo "FALLA: el núcleo no señaló arranque en 30 s"
    FALLAS=$((FALLAS + 1))
fi

if esperar_arranque "$CONTENEDOR_SIDECAR" '"evento":"sidecar.arrancado"'; then
    echo "OK: el sidecar señaló arranque (sidecar.arrancado)"
else
    echo "FALLA: el sidecar no señaló arranque en 30 s"
    FALLAS=$((FALLAS + 1))
fi

if [ "$FALLAS" -gt 0 ]; then
    echo "FALLA: la célula no llegó a un estado vivo verificable; se aborta antes de emitir docker stop"
    exit 1
fi

# --- AC-1: docker stop -t 30 sobre ambos contenedores -----------------------

echo ""
echo "=== docker stop -t 30 sobre ambos contenedores ==="

INICIO=$(date +%s)
docker stop -t 30 "$CONTENEDOR_NUCLEO" "$CONTENEDOR_SIDECAR"
FIN=$(date +%s)
echo "docker stop tardó $((FIN - INICIO)) s (margen 30 s)"

# --- AC-2: código de salida 0, sin SIGKILL/OOMKilled ------------------------

# verificar_salida_limpia <contenedor>
verificar_salida_limpia() {
    local contenedor="$1"
    local codigo oom

    codigo="$(docker inspect --format '{{.State.ExitCode}}' "$contenedor" 2>/dev/null)"
    oom="$(docker inspect --format '{{.State.OOMKilled}}' "$contenedor" 2>/dev/null)"

    if [ "$oom" = "true" ]; then
        echo "FALLA: [$contenedor] fue matado por falta de memoria (OOMKilled)"
        return 1
    fi
    if [ "$codigo" = "137" ]; then
        echo "FALLA: [$contenedor] salió con código 137 (SIGKILL forzado; no fue un apagado ordenado)"
        return 1
    fi
    if [ "$codigo" != "0" ]; then
        echo "FALLA: [$contenedor] salió con código ${codigo} (se esperaba 0)"
        return 1
    fi
    echo "OK: [$contenedor] salió con código 0, sin SIGKILL ni OOMKilled"
    return 0
}

verificar_salida_limpia "$CONTENEDOR_NUCLEO" || FALLAS=$((FALLAS + 1))
verificar_salida_limpia "$CONTENEDOR_SIDECAR" || FALLAS=$((FALLAS + 1))

# --- AC-3: punto de control del WAL del núcleo ------------------------------

echo ""
echo "=== Punto de control del WAL ==="

if docker logs "$CONTENEDOR_NUCLEO" 2>&1 | grep -q '"evento":"punto_de_control_wal"'; then
    echo "OK: el núcleo emitió punto_de_control_wal antes de salir"
else
    echo "FALLA: el núcleo no emitió la línea punto_de_control_wal; el checkpoint del WAL no se confirma"
    FALLAS=$((FALLAS + 1))
fi

# Confirmación adicional: sessions.db-wal no debe quedar con tamaño residual
# tras un checkpoint TRUNCATE. alpine:3 es la misma base que ya usan ambos
# Dockerfiles finales, montada aquí solo de lectura como utilidad de
# inspección; ningún contenedor de la célula sigue vivo en este punto.
TAMANO_WAL="$(docker run --rm -v "${VOLUMEN}:/datos:ro" alpine:3 \
    sh -c 'test -f /datos/sessions.db-wal && stat -c %s /datos/sessions.db-wal || echo 0' 2>/dev/null)"

if [ -z "$TAMANO_WAL" ]; then
    echo "FALLA: no se pudo leer el tamaño de sessions.db-wal en el volumen"
    FALLAS=$((FALLAS + 1))
elif [ "$TAMANO_WAL" -eq 0 ]; then
    echo "OK: sessions.db-wal no quedó con tamaño residual (${TAMANO_WAL} bytes)"
else
    echo "FALLA: sessions.db-wal quedó con ${TAMANO_WAL} bytes tras el apagado; el checkpoint no se completó"
    FALLAS=$((FALLAS + 1))
fi

# --- Resumen -----------------------------------------------------------------

echo ""
if [ "$FALLAS" -eq 0 ]; then
    echo "OK: apagado ordenado verificado — AC-1, AC-2 y AC-3 pasan"
    exit 0
else
    echo "FALLA: ${FALLAS} aserción(es) fallaron; ver detalle arriba"
    exit 1
fi
