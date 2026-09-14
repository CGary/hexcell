#!/usr/bin/env bash
# ============================================================================
# Guardia estático de aislamiento por célula (HEX-076, tarea 17 A-6)
# ============================================================================
# Verifica, sobre el YAML RESUELTO de deploy/cell.compose.yml, que la
# plantilla declara exactamente:
#
#   - una red propia por célula (`networks.red.name`), con el nombre EXACTO
#     que ya trae `deploy/celula.env.ejemplo` — no un nombre por omisión de
#     `docker compose` ni un literal compartido entre células.
#   - un volumen propio por célula (`volumes.datos.name`), con el mismo
#     criterio de igualdad exacta.
#   - ningún servicio (`nucleo`, `sidecar`) con la clave `ports:` presente,
#     ni vacía ni con mapeos: publicar un puerto al host rompe el
#     aislamiento de red que esta tarea existe para probar.
#
# POR QUÉ igualdad EXACTA contra el valor del referente y no "no vacío":
# medido en este proyecto (2026-09-13) que, al quitar el override
# `name: ${VAR}` de una red o volumen, `docker compose config` NO deja el
# campo `name` ausente: sintetiza un nombre por omisión con el prefijo del
# proyecto (p. ej. `tmp_red`). Un guardia que solo comprobara "el campo name
# existe" pasaría igual sobre esa mutación — sería un guardia vacío. Por eso
# se compara contra el literal que trae `deploy/celula.env.ejemplo`, el mismo
# env-file que resuelve la plantilla en este guardia y en el `--autoprueba`.
#
# POR QUÉ docker compose config y no lectura cruda del YAML: mismo footgun ya
# medido para deploy/verificar_endurecimiento.sh — `docker compose config`
# resuelve aun con `build.context` inválido, así que esta inspección NO
# afirma que las imágenes existan ni que arranquen; solo que la composición
# declara red y volumen propios y ningún puerto publicado. La prueba viva con
# contenedores reales es deploy/verificar_aislamiento.sh (manual, fuera de
# CI).
#
# USO
#
#   deploy/verificar_aislamiento_estatica.sh <ruta-plantilla>
#       Verifica la plantilla indicada. Sale 0 si pasa, distinto de 0 si
#       falla (una línea `FALLA: ...` por cada motivo).
#
#   deploy/verificar_aislamiento_estatica.sh --autoprueba
#       Copia deploy/cell.compose.yml a un directorio temporal, le rompe UNA
#       propiedad de aislamiento por vez (quita el override de nombre de la
#       red, quita el override de nombre del volumen, agrega una publicación
#       de puerto al host) y verifica que el guardia falla sobre cada copia
#       mutada, corriendo bajo el MISMO `docker compose config` que el modo
#       normal. Si alguna mutación pasa al guardia, no es todavía un
#       guardia y el script termina con código de error. Este modo es la
#       prueba de mutación exigida por AC-3 del 00-spec.yaml.
#
# DEPENDENCIAS
#
#   - bash, sed, mktemp, rm                       (POSIX/Util-linux estándar)
#   - docker + docker compose                      (CLI v5.x verificado)
#   - python3 con PyYAML                           (ya validado por HEX-068)
#
# Un entorno sin docker/compose NO se declara verificado: el script falla
# con un mensaje explícito, porque "omitido" sería indistinguible de "pasa"
# y eso es exactamente el fallo que AC-3 existe para impedir.
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
    echo "FALLA: deploy/celula.env.ejemplo no existe; no hay referente de valores esperados de red/volumen" >&2
    exit 1
fi

# --- Prerrequisitos ---------------------------------------------------------

if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
    echo "FALLA: docker compose no está disponible en este entorno; el guardia no puede correr y AC-1/AC-2/AC-3 no se declaran verificadas" >&2
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

# Valores esperados: EXACTOS, tomados del mismo referente que resuelve la
# plantilla (--env-file deploy/celula.env.ejemplo). Si alguno falta, el
# referente cambió de forma incompatible con este guardia.
RED_ESPERADA="$(sed -n 's/^HEXCELL_RED_CELULA=//p' deploy/celula.env.ejemplo)"
VOLUMEN_ESPERADO="$(sed -n 's/^HEXCELL_VOLUMEN_CELULA=//p' deploy/celula.env.ejemplo)"

if [ -z "$RED_ESPERADA" ] || [ -z "$VOLUMEN_ESPERADO" ]; then
    echo "FALLA: no se pudo leer HEXCELL_RED_CELULA / HEXCELL_VOLUMEN_CELULA de deploy/celula.env.ejemplo" >&2
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
    ruta_resuelto="$(mktemp -t hex076-resuelto.XXXXXX.yaml)"
    # shellcheck disable=SC2064  # expandir $ruta_resuelto ahora, no en la trampa
    trap "rm -f '$ruta_resuelto'" RETURN

    if ! docker compose --env-file deploy/celula.env.ejemplo -f "$ruta" config >"$ruta_resuelto" 2>/dev/null; then
        echo "FALLA: docker compose config no pudo resolver la plantilla [$ruta]"
        return 1
    fi

    # Exportar rutas y valores esperados para que el python embebido los lea
    # sin quoting arriesgado.
    export HEX076_RESUELTO="$ruta_resuelto"
    export HEX076_RED_ESPERADA="$RED_ESPERADA"
    export HEX076_VOLUMEN_ESPERADO="$VOLUMEN_ESPERADO"

    python3 - <<'PY'
import os, sys, yaml

with open(os.environ["HEX076_RESUELTO"]) as f:
    doc = yaml.safe_load(f)

red_esperada = os.environ["HEX076_RED_ESPERADA"]
volumen_esperado = os.environ["HEX076_VOLUMEN_ESPERADO"]

fallas = []

# --- AC-1: red propia por célula --------------------------------------------
redes = doc.get("networks") or {}
if set(redes.keys()) != {"red"}:
    fallas.append(
        f"top-level networks debe declarar exactamente la clave 'red', se obtuvo {sorted(redes.keys())!r}"
    )
else:
    nombre_red = (redes.get("red") or {}).get("name")
    if nombre_red != red_esperada:
        fallas.append(
            f"networks.red.name debe ser exactamente {red_esperada!r} (el valor de "
            f"HEXCELL_RED_CELULA en deploy/celula.env.ejemplo), se obtuvo {nombre_red!r}"
        )

# --- AC-1: volumen propio por célula ----------------------------------------
volumenes = doc.get("volumes") or {}
if set(volumenes.keys()) != {"datos"}:
    fallas.append(
        f"top-level volumes debe declarar exactamente la clave 'datos', se obtuvo {sorted(volumenes.keys())!r}"
    )
else:
    nombre_vol = (volumenes.get("datos") or {}).get("name")
    if nombre_vol != volumen_esperado:
        fallas.append(
            f"volumes.datos.name debe ser exactamente {volumen_esperado!r} (el valor de "
            f"HEXCELL_VOLUMEN_CELULA en deploy/celula.env.ejemplo), se obtuvo {nombre_vol!r}"
        )

# --- AC-2: ningún servicio publica un puerto al host ------------------------
services = doc.get("services") or {}
SERVICIOS_OBLIGADOS = ("nucleo", "sidecar")

for nombre in SERVICIOS_OBLIGADOS:
    svc = services.get(nombre)
    if svc is None:
        fallas.append(f"servicio [{nombre}] ausente en la plantilla resuelta")
        continue
    if "ports" in svc:
        fallas.append(
            f"servicio [{nombre}]: declara la clave 'ports' (publica un puerto al "
            f"host); se obtuvo {svc.get('ports')!r}"
        )

if fallas:
    for f in fallas:
        print(f"FALLA: {f}")
    sys.exit(1)

print(
    "OK: red y volumen propios por célula con los nombres esperados, "
    "y ningún servicio publica un puerto al host"
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
# Por cada una de las tres propiedades de aislamiento se copia la plantilla
# a un scratch, se le rompe UNA propiedad por vez y se verifica que el
# guardia falla. Si el guardia pasara la copia mutada, la "prueba" no probó
# nada — por eso se imprime una línea PASA/FALLA por cada caso y se sale con
# código 0 solo si los tres casos fallaron. El implementador DEBE leer las
# tres líneas PASA/FALLA —no solo el exit code— antes de dar AC-3 por
# satisfecha.

DIR_TEMP=""
DIR_TEMP=$(mktemp -d -t hex076-guard.XXXXXX)
# shellcheck disable=SC2064  # expandir $DIR_TEMP ahora, no en la trampa
trap "rm -rf '$DIR_TEMP'" EXIT

ORIGINAL="$PLANTILLA"

echo "Modo --autoprueba: cada propiedad de aislamiento, una por vez, debe ser detectada al romperse."

TOTAL=0
ACIERTOS=0

# --- quitar el override de nombre de la red ---------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/sin-red-propia.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/name: \${HEXCELL_RED_CELULA}/d' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar el nombre per-célula de la red -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar el nombre per-célula de la red -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- quitar el override de nombre del volumen -------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/sin-volumen-propio.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/name: \${HEXCELL_VOLUMEN_CELULA}/d' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar el nombre per-célula del volumen -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar el nombre per-célula del volumen -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- agregar una publicación de puerto al host ------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/con-puerto-publicado.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/container_name: \${HEXCELL_ID_CELULA}-nucleo/a\    ports:\n      - "18081:8081"' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: publicar un puerto al host en nucleo -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: publicar un puerto al host en nucleo -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

echo ""
echo "Resumen autoprueba: $ACIERTOS/$TOTAL casos pasan (cada propiedad rota debe hacer fallar al guardia)"

if [ "$ACIERTOS" -eq "$TOTAL" ]; then
    echo "OK: el guardia falla bajo cada mutación"
    exit 0
else
    echo "FALLA: el guardia no detectó todas las mutaciones"
    exit 1
fi
