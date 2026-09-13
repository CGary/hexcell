#!/usr/bin/env bash
# ============================================================================
# Guardia de propagación de señales de apagado (HEX-075, tarea 7 A-6)
# ============================================================================
# Ancla, de forma mecánica, las tres condiciones de las que depende un
# `docker stop` ordenado sobre la célula:
#
#   1. ENTRYPOINT en forma exec (arreglo JSON, sin shell) en Dockerfile y en
#      sidecar/Dockerfile. Una forma shell (`ENTRYPOINT /ruta/al/binario`,
#      sin corchetes) envuelve el proceso en `/bin/sh -c` y SIGTERM deja de
#      llegar al PID 1 real.
#   2. Línea literal `STOPSIGNAL SIGTERM` en ambos Dockerfiles.
#   3. `stop_grace_period: "30s"` en los servicios `nucleo` y `sidecar` de
#      la plantilla de composición, sobre el YAML RESUELTO por
#      `docker compose config` (mismo criterio que HEX-070: el formato
#      canónico es el resuelto, no el crudo — protege contra
#      reordenaciones o reescrituras de campos).
#
# POR QUÉ DOS FAMILIAS DE COMPROBACIÓN DISTINTAS: ENTRYPOINT y STOPSIGNAL son
# instrucciones de Dockerfile en tiempo de CONSTRUCCIÓN, invisibles para
# `docker compose config` (que solo resuelve el YAML de composición, nunca el
# contenido de un Dockerfile). Por eso esas dos comprobaciones leen el texto
# de los Dockerfiles directamente, y solo `stop_grace_period` pasa por el
# YAML resuelto de compose.
#
# LOS DOS DOCKERFILES SON RUTAS FIJAS (`Dockerfile`, `sidecar/Dockerfile`),
# no un argumento: son los dos únicos que existen en el repositorio y este
# guardia siempre corre desde la raíz del repositorio, igual que
# deploy/verificar_endurecimiento.sh.
#
# USO
#
#   deploy/verificar_senales.sh <ruta-plantilla>
#       Verifica los dos Dockerfiles fijos y la plantilla de composición
#       indicada. Sale 0 si pasa, distinto de 0 si falla (una línea
#       `FALLA: ...` por cada motivo).
#
#   deploy/verificar_senales.sh --autoprueba
#       Copia los archivos vigilados a un directorio temporal, flipa UNA de
#       las tres condiciones ancladas por vez (ENTRYPOINT en forma shell,
#       falta de STOPSIGNAL, falta de stop_grace_period) y verifica que el
#       guardia falla sobre cada copia mutada. Si alguna mutación pasa al
#       guardia, no es todavía un guardia y el script termina con código de
#       error. Este modo es la prueba de mutación exigida por AC-5 del
#       00-spec.yaml.
#
# DEPENDENCIAS
#
#   - bash, sed, grep, mktemp, rm                  (POSIX/Util-linux estándar)
#   - docker + docker compose                       (CLI v5.x verificado)
#   - python3 con PyYAML                            (ya validado por HEX-068)
#
# Un entorno sin docker/compose o sin PyYAML NO se declara verificado: el
# guardia falla con un mensaje explícito, porque "omitido" sería
# indistinguible de "pasa" y eso es exactamente el fallo que AC-5 existe
# para impedir.
# ============================================================================

set -u

DOCKERFILE_NUCLEO="Dockerfile"
DOCKERFILE_SIDECAR="sidecar/Dockerfile"

# --- Argumentos -------------------------------------------------------------

PLANTILLA="${1:-}"
MODO_AUTOPRUEBA=0

if [ "$PLANTILLA" = "--autoprueba" ]; then
    MODO_AUTOPRUEBA=1
    PLANTILLA="deploy/cell.compose.yml"
elif [ -z "$PLANTILLA" ]; then
    echo "Uso: $0 <ruta-plantilla> | --autoprueba" >&2
    exit 2
fi

if [ ! -f "$PLANTILLA" ]; then
    echo "FALLA: la plantilla [$PLANTILLA] no existe" >&2
    exit 1
fi

# --- Prerrequisitos ---------------------------------------------------------

if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
    echo "FALLA: docker compose no está disponible en este entorno; el guardia no puede correr y AC-4/AC-5 no se declaran verificadas" >&2
    exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "FALLA: python3 no está disponible; el guardia no puede parsear el YAML resuelto" >&2
    exit 1
fi

if ! python3 -c "import yaml" >/dev/null 2>&1; then
    echo "FALLA: PyYAML no está disponible en python3" >&2
    exit 1
fi

# --- Comprobación 1: ENTRYPOINT en forma exec -------------------------------

# verificar_entrypoint_exec <ruta-dockerfile>
verificar_entrypoint_exec() {
    local ruta="$1"
    if [ ! -f "$ruta" ]; then
        echo "FALLA: [$ruta] no existe"
        return 1
    fi
    if ! grep -qE '^ENTRYPOINT[[:space:]]*\[.*\][[:space:]]*$' "$ruta"; then
        echo "FALLA: [$ruta] no declara ENTRYPOINT en forma exec (arreglo JSON, sin shell)"
        return 1
    fi
    return 0
}

# --- Comprobación 2: STOPSIGNAL SIGTERM -------------------------------------

# verificar_stopsignal <ruta-dockerfile>
verificar_stopsignal() {
    local ruta="$1"
    if [ ! -f "$ruta" ]; then
        echo "FALLA: [$ruta] no existe"
        return 1
    fi
    if ! grep -qE '^STOPSIGNAL[[:space:]]+SIGTERM[[:space:]]*$' "$ruta"; then
        echo "FALLA: [$ruta] no declara la línea literal STOPSIGNAL SIGTERM"
        return 1
    fi
    return 0
}

# --- Comprobación 3: stop_grace_period sobre el YAML resuelto ---------------

# verificar_stop_grace_period <ruta-plantilla>
verificar_stop_grace_period() {
    local ruta="$1"

    local ruta_resuelto
    ruta_resuelto="$(mktemp -t hex075-resuelto.XXXXXX.yaml)"
    # shellcheck disable=SC2064  # expandir $ruta_resuelto ahora, no en la trampa
    trap "rm -f '$ruta_resuelto'" RETURN

    if ! docker compose --env-file deploy/celula.env.ejemplo -f "$ruta" config >"$ruta_resuelto" 2>/dev/null; then
        echo "FALLA: docker compose config no pudo resolver la plantilla [$ruta]"
        return 1
    fi

    export HEX075_RESUELTO="$ruta_resuelto"

    python3 - <<'PY'
import os, sys, yaml

with open(os.environ["HEX075_RESUELTO"]) as f:
    doc = yaml.safe_load(f)

services = doc.get("services") or {}
SERVICIOS_OBLIGADOS = ("nucleo", "sidecar")
fallas = []

for nombre in SERVICIOS_OBLIGADOS:
    svc = services.get(nombre)
    if svc is None:
        fallas.append(f"servicio [{nombre}] ausente en la plantilla resuelta")
        continue

    sgp = svc.get("stop_grace_period")
    if sgp != "30s":
        fallas.append(
            f"servicio [{nombre}]: stop_grace_period debe ser exactamente '30s', se obtuvo {sgp!r}"
        )

if fallas:
    for f in fallas:
        print(f"FALLA: {f}")
    sys.exit(1)

print("OK: stop_grace_period es '30s' en nucleo y sidecar")
sys.exit(0)
PY
}

# --- Verificación completa (modo normal) ------------------------------------

# verificar_todo <ruta-plantilla>
verificar_todo() {
    local plantilla="$1"
    local ok=1

    verificar_entrypoint_exec "$DOCKERFILE_NUCLEO" || ok=0
    verificar_stopsignal "$DOCKERFILE_NUCLEO" || ok=0
    verificar_entrypoint_exec "$DOCKERFILE_SIDECAR" || ok=0
    verificar_stopsignal "$DOCKERFILE_SIDECAR" || ok=0
    verificar_stop_grace_period "$plantilla" || ok=0

    if [ "$ok" -eq 1 ]; then
        echo "OK: ENTRYPOINT en forma exec, STOPSIGNAL SIGTERM y stop_grace_period: 30s están anclados en núcleo y sidecar"
        return 0
    fi
    return 1
}

if [ "$MODO_AUTOPRUEBA" -eq 0 ]; then
    if verificar_todo "$PLANTILLA"; then
        exit 0
    else
        exit 1
    fi
fi

# --- Modo --autoprueba (prueba de mutación) --------------------------------
#
# Por cada una de las tres condiciones ancladas se copia el archivo que la
# lleva a un directorio temporal, se le rompe esa condición y se verifica que
# el guardia falla sobre la copia mutada. Si el guardia pasara la copia
# mutada, la "prueba" no probó nada — por eso se imprime una línea
# PASA/FALLA por cada caso y se sale con código 0 solo si los tres casos
# fallaron.

DIR_TEMP=""
DIR_TEMP=$(mktemp -d -t hex075-guard.XXXXXX)
# shellcheck disable=SC2064  # expandir $DIR_TEMP ahora, no en la trampa
trap "rm -rf '$DIR_TEMP'" EXIT

echo "Modo --autoprueba: cada condición anclada, una por vez, debe ser detectada cuando se rompe."

TOTAL=0
ACIERTOS=0

# --- ENTRYPOINT en forma shell (rompe la forma exec) ------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/Dockerfile.entrypoint-shell"
cp "$DOCKERFILE_NUCLEO" "$COPIA"
sed -i -E 's/^ENTRYPOINT[[:space:]]*\[[[:space:]]*"([^"]+)"[[:space:]]*\][[:space:]]*$/ENTRYPOINT \1/' "$COPIA"
if ! verificar_entrypoint_exec "$COPIA" >/dev/null 2>&1; then
    echo "PASA: ENTRYPOINT en forma shell -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: ENTRYPOINT en forma shell -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- Falta STOPSIGNAL --------------------------------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/Dockerfile.sin-stopsignal"
cp "$DOCKERFILE_NUCLEO" "$COPIA"
sed -i '/^STOPSIGNAL[[:space:]]\+SIGTERM[[:space:]]*$/d' "$COPIA"
if ! verificar_stopsignal "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar STOPSIGNAL -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar STOPSIGNAL -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- Falta stop_grace_period -------------------------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/cell.compose.sin-stop_grace_period.yml"
cp "deploy/cell.compose.yml" "$COPIA"
sed -i '/^[[:space:]]*stop_grace_period:[[:space:]]*"30s"[[:space:]]*$/d' "$COPIA"
if ! verificar_stop_grace_period "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar stop_grace_period -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar stop_grace_period -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

echo ""
echo "Resumen autoprueba: $ACIERTOS/$TOTAL casos pasan (cada caso roto debe hacer fallar al guardia)"

if [ "$ACIERTOS" -eq "$TOTAL" ]; then
    echo "OK: el guardia falla bajo cada mutación"
    exit 0
else
    echo "FALLA: el guardia no detectó todas las mutaciones"
    exit 1
fi
