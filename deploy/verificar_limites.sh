#!/usr/bin/env bash
# ============================================================================
# Guardia estático de límites de recursos por contenedor (HEX-078, tarea 6 A-6)
# ============================================================================
# Verifica, sobre el YAML RESUELTO de deploy/cell.compose.yml, que los
# servicios `nucleo` y `sidecar` declaran los tres límites de recursos con
# los valores EXACTOS que fija deploy/celula.env.ejemplo (el referente que
# resuelve la plantilla):
#
#   mem_limit      — memoria máxima en bytes. `docker compose config` la
#                    resuelve como CADENA de bytes crudos ("50331648" para
#                    48m), nunca como "48m"; el guardia convierte el sufijo
#                    `<N>m` del referente a bytes (N * 1048576) antes de
#                    comparar (medido 2026-09-13, compose 5.5.1).
#   cpus           — fracción de CPU, resuelta como número.
#   ulimits.nofile — límite de descriptores de archivo, resuelto como entero.
#
# POR QUÉ igualdad EXACTA contra el referente y no "campo presente": medido
# en este proyecto (2026-09-13) que al quitar una línea mem_limit/cpus/nofile
# `docker compose config` simplemente OMITE el campo del YAML resuelto —no
# sintetiza un valor por omisión—, así que un guardia que solo comprobara la
# presencia ya no sería vacío. La comparación exacta es estrictamente más
# fuerte: también detecta un valor que se desvía en silencio del referente
# (una memoria que deja de sumar 80 MB, una CPU fuera de lo decidido, un
# nofile que cambia sin anotarlo), que es el modo de fallo que esta tarea
# existe para impedir.
#
# POR QUÉ docker compose config y no lectura cruda del YAML: mismo criterio
# que deploy/verificar_aislamiento_estatica.sh y deploy/verificar_senales.sh.
# Los límites permanecen parametrizados (${HEXCELL_<SERVICIO>_LIMITE_...});
# la inspección del YAML resuelto es lo único que demuestra que la
# composición los materializa de verdad.
#
# USO
#
#   deploy/verificar_limites.sh <ruta-plantilla>
#       Verifica la plantilla indicada. Sale 0 si pasa, distinto de 0 si
#       falla (una línea `FALLA: ...` por cada motivo).
#
#   deploy/verificar_limites.sh --autoprueba
#       Copia deploy/cell.compose.yml a un directorio temporal y corrompe UNA
#       de las seis condiciones por vez (mem_limit/cpus/ulimits.nofile sobre
#       cada uno de nucleo/sidecar), verificando que el guardia falla sobre
#       cada copia mutada bajo el MISMO `docker compose config` que el modo
#       normal. Además corrompe una séptima copia, esta vez de
#       deploy/celula.env.ejemplo (el referente), desviando la suma de
#       memoria del techo NFR-01, y verifica que verificar_techo_nfr01 la
#       atrapa. Si alguna de las siete mutaciones pasa al guardia, no es
#       todavía un guardia. Este modo es la prueba de mutación exigida por
#       AC-4 del 00-spec.yaml.
#
# DEPENDENCIAS
#
#   - bash, sed, grep, mktemp, rm                  (POSIX/Util-linux estándar)
#   - docker + docker compose                       (CLI v5.x verificado)
#   - python3 con PyYAML                            (ya validado por HEX-068)
#
# Un entorno sin docker/compose o sin PyYAML NO se declara verificado: el
# script falla con un mensaje explícito, porque "omitido" sería
# indistinguible de "pasa" y eso es exactamente el fallo que AC-4 existe
# para impedir.
# ============================================================================

set -u

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

if [ ! -f deploy/celula.env.ejemplo ]; then
    echo "FALLA: deploy/celula.env.ejemplo no existe; no hay referente de valores esperados de límites" >&2
    exit 1
fi

# --- Prerrequisitos ---------------------------------------------------------

if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
    echo "FALLA: docker compose no está disponible en este entorno; el guardia no puede correr y AC-3/AC-4 no se declaran verificadas" >&2
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

# --- Referente: valores esperados, EXACTOS, tomados de deploy/celula.env.ejemplo
# (el mismo env-file que resuelve la plantilla aquí y en el --autoprueba).

REFERENTE="deploy/celula.env.ejemplo"

MEMORIA_NUCLEO="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_MEMORIA=//p' "$REFERENTE")"
CPUS_NUCLEO="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_CPUS=//p' "$REFERENTE")"
NOFILE_NUCLEO="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_NOFILE=//p' "$REFERENTE")"
MEMORIA_SIDECAR="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_MEMORIA=//p' "$REFERENTE")"
CPUS_SIDECAR="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_CPUS=//p' "$REFERENTE")"
NOFILE_SIDECAR="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_NOFILE=//p' "$REFERENTE")"

# convertir_memoria_a_bytes <valor>
#   "48m" -> "50331648", "32m" -> "33554432". Solo maneja el sufijo `m` que la
#   plantilla y el referente usan; cualquier otra forma se devuelve tal cual.
convertir_memoria_a_bytes() {
    local valor="$1"
    case "$valor" in
        *m) printf '%s' "$(( ${valor%m} * 1048576 ))" ;;
        *) printf '%s' "$valor" ;;
    esac
}

MEMORIA_NUCLEO_BYTES="$(convertir_memoria_a_bytes "$MEMORIA_NUCLEO")"
MEMORIA_SIDECAR_BYTES="$(convertir_memoria_a_bytes "$MEMORIA_SIDECAR")"

if [ -z "$MEMORIA_NUCLEO_BYTES" ] || [ -z "$MEMORIA_SIDECAR_BYTES" ] \
    || [ -z "$CPUS_NUCLEO" ] || [ -z "$CPUS_SIDECAR" ] \
    || [ -z "$NOFILE_NUCLEO" ] || [ -z "$NOFILE_SIDECAR" ]; then
    echo "FALLA: no se pudieron leer los seis HEXCELL_*_LIMITE_* de $REFERENTE" >&2
    exit 1
fi

# --- Techo NFR-01: la SUMA de memoria del referente, anclada como constante -
#
# El bloque anterior compara la plantilla resuelta CONTRA $REFERENTE, pero
# $REFERENTE es el mismo archivo que `docker compose --env-file` usa para
# resolver la plantilla: ambos lados se mueven juntos. Ese diseño atrapa una
# plantilla que se desvía de su referente, pero es estructuralmente incapaz
# de atrapar al referente desviándose del techo NFR-01 (medido 2026-09-13:
# mutar HEXCELL_NUCLEO_LIMITE_MEMORIA=48m -> 128m en deploy/celula.env.ejemplo
# y correr este guardia en modo directo seguía saliendo OK con código 0).
#
# Por eso esta aserción ancla una CONSTANTE fuera del referente: la SUMA de
# HEXCELL_NUCLEO_LIMITE_MEMORIA + HEXCELL_SIDECAR_LIMITE_MEMORIA debe ser
# exactamente 83886080 bytes (80 MiB = 80 * 1048576, el techo de NFR-01 del
# PRD). Se ancla la SUMA y no los literales 48m/32m: ese reparto es
# explícitamente provisional (00-spec.yaml, tarea 16 de la etapa A-6 puede
# re-repartirlo legítimamente) y anclar los literales pondría este guardia en
# rojo ante un cambio legítimo. Anclar la suma sobrevive a un re-reparto
# legítimo y sigue atrapando al referente desviándose del techo real.
NFR01_TECHO_BYTES=83886080

# verificar_techo_nfr01 <ruta-referente>
#   Lee HEXCELL_{NUCLEO,SIDECAR}_LIMITE_MEMORIA del referente indicado (no
#   necesariamente el global $REFERENTE: el modo --autoprueba lo llama sobre
#   una copia mutada) y falla si la suma en bytes no es exactamente
#   $NFR01_TECHO_BYTES.
verificar_techo_nfr01() {
    local referente="$1"
    local mem_nucleo mem_sidecar bytes_nucleo bytes_sidecar suma

    mem_nucleo="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_MEMORIA=//p' "$referente")"
    mem_sidecar="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_MEMORIA=//p' "$referente")"

    if [ -z "$mem_nucleo" ] || [ -z "$mem_sidecar" ]; then
        echo "FALLA: no se pudieron leer HEXCELL_NUCLEO_LIMITE_MEMORIA / HEXCELL_SIDECAR_LIMITE_MEMORIA de $referente para el techo NFR-01"
        return 1
    fi

    bytes_nucleo="$(convertir_memoria_a_bytes "$mem_nucleo")"
    bytes_sidecar="$(convertir_memoria_a_bytes "$mem_sidecar")"
    suma=$((bytes_nucleo + bytes_sidecar))

    if [ "$suma" -ne "$NFR01_TECHO_BYTES" ]; then
        echo "FALLA: la suma de HEXCELL_NUCLEO_LIMITE_MEMORIA + HEXCELL_SIDECAR_LIMITE_MEMORIA en $referente es $suma bytes, debe ser exactamente $NFR01_TECHO_BYTES bytes (80m, techo NFR-01 del PRD)"
        return 1
    fi

    return 0
}

if ! verificar_techo_nfr01 "$REFERENTE"; then
    exit 1
fi

# --- Función de verificación (modo normal) ---------------------------------

# verificar_plantilla <ruta-plantilla>
#   Resuelve la plantilla con docker compose config (con el env de ejemplo) y
#   ejecuta las aserciones sobre el YAML resultante. Imprime `FALLA: ...` por
#   cada motivo o una línea `OK: ...` si todo pasa. Sale 0 o distinto de 0.
verificar_plantilla() {
    local ruta="$1"

    local ruta_resuelto
    ruta_resuelto="$(mktemp -t hex078-resuelto.XXXXXX.yaml)"
    # shellcheck disable=SC2064  # expandir $ruta_resuelto ahora, no en la trampa
    trap "rm -f '$ruta_resuelto'" RETURN

    if ! docker compose --env-file "$REFERENTE" -f "$ruta" config >"$ruta_resuelto" 2>/dev/null; then
        echo "FALLA: docker compose config no pudo resolver la plantilla [$ruta]"
        return 1
    fi

    # Exportar valores esperados para que el python embebido los lea sin
    # quoting arriesgado.
    export HEX078_RESUELTO="$ruta_resuelto"
    export HEX078_MEMORIA_NUCLEO="$MEMORIA_NUCLEO_BYTES"
    export HEX078_MEMORIA_SIDECAR="$MEMORIA_SIDECAR_BYTES"
    export HEX078_CPUS_NUCLEO="$CPUS_NUCLEO"
    export HEX078_CPUS_SIDECAR="$CPUS_SIDECAR"
    export HEX078_NOFILE_NUCLEO="$NOFILE_NUCLEO"
    export HEX078_NOFILE_SIDECAR="$NOFILE_SIDECAR"

    python3 - <<'PY'
import os, sys, yaml

with open(os.environ["HEX078_RESUELTO"]) as f:
    doc = yaml.safe_load(f)

SERVICIOS_OBLIGADOS = ("nucleo", "sidecar")
esperado = {
    "nucleo": {
        "memoria": os.environ["HEX078_MEMORIA_NUCLEO"],
        "cpus": os.environ["HEX078_CPUS_NUCLEO"],
        "nofile": os.environ["HEX078_NOFILE_NUCLEO"],
    },
    "sidecar": {
        "memoria": os.environ["HEX078_MEMORIA_SIDECAR"],
        "cpus": os.environ["HEX078_CPUS_SIDECAR"],
        "nofile": os.environ["HEX078_NOFILE_SIDECAR"],
    },
}

fallas = []
services = doc.get("services") or {}

for nombre in SERVICIOS_OBLIGADOS:
    svc = services.get(nombre)
    if svc is None:
        fallas.append(f"servicio [{nombre}] ausente en la plantilla resuelta")
        continue

    exp = esperado[nombre]

    ml = svc.get("mem_limit")
    if ml is None:
        fallas.append(f"servicio [{nombre}]: mem_limit ausente en la plantilla resuelta")
    elif str(ml) != exp["memoria"]:
        fallas.append(
            f"servicio [{nombre}]: mem_limit debe ser exactamente "
            f"{exp['memoria']} bytes (el valor del referente), se obtuvo {ml!r}"
        )

    cp = svc.get("cpus")
    if cp is None:
        fallas.append(f"servicio [{nombre}]: cpus ausente en la plantilla resuelta")
    elif float(cp) != float(exp["cpus"]):
        fallas.append(
            f"servicio [{nombre}]: cpus debe ser exactamente {exp['cpus']}, se obtuvo {cp!r}"
        )

    ul = svc.get("ulimits") or {}
    nf = ul.get("nofile") if isinstance(ul, dict) else None
    if nf is None:
        fallas.append(
            f"servicio [{nombre}]: ulimits.nofile ausente en la plantilla resuelta"
        )
    elif int(nf) != int(exp["nofile"]):
        fallas.append(
            f"servicio [{nombre}]: ulimits.nofile debe ser exactamente "
            f"{exp['nofile']}, se obtuvo {nf!r}"
        )

if fallas:
    for f in fallas:
        print(f"FALLA: {f}")
    sys.exit(1)

print(
    "OK: mem_limit, cpus y ulimits.nofile coinciden exactamente con el "
    "referente en nucleo y sidecar"
)
sys.exit(0)
PY
}

# --- Modo normal ------------------------------------------------------------

if [ "$MODO_AUTOPRUEBA" -eq 0 ]; then
    if verificar_plantilla "$PLANTILLA"; then
        exit 0
    else
        exit 1
    fi
fi

# --- Modo --autoprueba (prueba de mutación) --------------------------------
#
# Por cada una de las seis condiciones (mem_limit/cpus/ulimits.nofile sobre
# nucleo/sidecar) se copia la plantilla a un scratch y se CORROMPE su valor
# sustituyendo la referencia de la variable por un literal distinto, en vez
# de borrar la línea: borrar mem_limit/nofile deja el campo ausente y el
# guardia fallaría igual, pero corromper el valor mantiene el YAML resoluble
# y ejerce la comparación EXACTA, que es la aserción fuerte de este guardia.
# Antes de verificar, este modo comprueba que la mutación CAMBIÓ el archivo:
# si el patrón no matcheara (p. ej. un cambio de formato), la copia seguiría
# intacta, el guardia pasaría y el caso se reportaría como FALLA —el guardia
# debe ser capaz de detectar también el no-op de su propia mutación. Se
# imprime una línea PASA/FALLA por cada caso y se sale con código 0 solo si
# los siete casos fallaron (los seis anteriores más el séptimo, que corrompe
# el referente mismo para probar verificar_techo_nfr01). El implementador
# DEBE leer las siete líneas PASA/FALLA —no solo el exit code— antes de dar
# AC-4 por satisfecha.

DIR_TEMP=""
DIR_TEMP=$(mktemp -d -t hex078-guard.XXXXXX)
# shellcheck disable=SC2064  # expandir $DIR_TEMP ahora, no en la trampa
trap "rm -rf '$DIR_TEMP'" EXIT

ORIGINAL="$PLANTILLA"

echo "Modo --autoprueba: cada límite, uno por vez, debe ser detectado cuando se corrompe."

TOTAL=0
ACIERTOS=0

# mutar_y_verificar <etiqueta> <patron_sed> <patron_plano> <sustitucion>
#   Copia la plantilla, sustituye <patron_sed> (con \$ escapado para sed) por
#   <sustitucion>, comprueba con grep -F sobre <patron_plano> (el texto sin
#   escapar, que es el que hay en el archivo) que el archivo CAMBIÓ y, si
#   cambió, verifica que el guardia falla sobre la copia mutada.
mutar_y_verificar() {
    local etiqueta="$1"
    local patron_sed="$2"
    local patron_plano="$3"
    local sustitucion="$4"

    TOTAL=$((TOTAL + 1))
    local copia
    copia="$DIR_TEMP/mutado-${TOTAL}.yml"
    cp "$ORIGINAL" "$copia"

    sed -i "s|${patron_sed}|${sustitucion}|" "$copia"

    if grep -qF -- "$patron_plano" "$copia"; then
        echo "FALLA: ${etiqueta} -> la mutación no cambió el archivo (patrón no encontrado); no es una prueba"
        return
    fi

    if ! verificar_plantilla "$copia" >/dev/null 2>&1; then
        echo "PASA: ${etiqueta} -> el guardia falla como debe"
        ACIERTOS=$((ACIERTOS + 1))
    else
        echo "FALLA: ${etiqueta} -> el guardia PASÓ la copia mutada (no es un guardia)"
    fi
}

mutar_y_verificar \
    "corromper mem_limit de nucleo" \
    'mem_limit: \${HEXCELL_NUCLEO_LIMITE_MEMORIA}' \
    'mem_limit: ${HEXCELL_NUCLEO_LIMITE_MEMORIA}' \
    'mem_limit: 999m'

mutar_y_verificar \
    "corromper cpus de nucleo" \
    'cpus: \${HEXCELL_NUCLEO_LIMITE_CPUS}' \
    'cpus: ${HEXCELL_NUCLEO_LIMITE_CPUS}' \
    'cpus: 9'

mutar_y_verificar \
    "corromper ulimits.nofile de nucleo" \
    'nofile: \${HEXCELL_NUCLEO_LIMITE_NOFILE}' \
    'nofile: ${HEXCELL_NUCLEO_LIMITE_NOFILE}' \
    'nofile: 999'

mutar_y_verificar \
    "corromper mem_limit de sidecar" \
    'mem_limit: \${HEXCELL_SIDECAR_LIMITE_MEMORIA}' \
    'mem_limit: ${HEXCELL_SIDECAR_LIMITE_MEMORIA}' \
    'mem_limit: 999m'

mutar_y_verificar \
    "corromper cpus de sidecar" \
    'cpus: \${HEXCELL_SIDECAR_LIMITE_CPUS}' \
    'cpus: ${HEXCELL_SIDECAR_LIMITE_CPUS}' \
    'cpus: 9'

mutar_y_verificar \
    "corromper ulimits.nofile de sidecar" \
    'nofile: \${HEXCELL_SIDECAR_LIMITE_NOFILE}' \
    'nofile: ${HEXCELL_SIDECAR_LIMITE_NOFILE}' \
    'nofile: 999'

# mutar_referente_y_verificar_techo <etiqueta> <patron_plano> <sustitucion>
#   Como mutar_y_verificar, pero corrompe una COPIA de $REFERENTE (no de la
#   plantilla) y ejerce verificar_techo_nfr01 directamente sobre esa copia:
#   este caso prueba el techo NFR-01 (la suma anclada), no la comparación
#   plantilla-contra-referente que cubren los seis casos anteriores.
mutar_referente_y_verificar_techo() {
    local etiqueta="$1"
    local patron_plano="$2"
    local sustitucion="$3"

    TOTAL=$((TOTAL + 1))
    local copia
    copia="$DIR_TEMP/referente-mutado-${TOTAL}.env"
    cp "$REFERENTE" "$copia"

    sed -i "s|${patron_plano}|${sustitucion}|" "$copia"

    if grep -qF -- "$patron_plano" "$copia"; then
        echo "FALLA: ${etiqueta} -> la mutación no cambió el archivo (patrón no encontrado); no es una prueba"
        return
    fi

    if ! verificar_techo_nfr01 "$copia" >/dev/null 2>&1; then
        echo "PASA: ${etiqueta} -> el guardia falla como debe"
        ACIERTOS=$((ACIERTOS + 1))
    else
        echo "FALLA: ${etiqueta} -> el guardia PASÓ la copia mutada (no es un guardia)"
    fi
}

mutar_referente_y_verificar_techo \
    "corromper el techo NFR-01 en el referente (la suma deja de ser 80m)" \
    'HEXCELL_NUCLEO_LIMITE_MEMORIA=48m' \
    'HEXCELL_NUCLEO_LIMITE_MEMORIA=128m'

echo ""
echo "Resumen autoprueba: $ACIERTOS/$TOTAL casos pasan (cada límite roto debe hacer fallar al guardia)"

if [ "$ACIERTOS" -eq "$TOTAL" ]; then
    echo "OK: el guardia falla bajo cada mutación"
    exit 0
else
    echo "FALLA: el guardia no detectó todas las mutaciones"
    exit 1
fi