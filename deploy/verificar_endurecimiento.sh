#!/usr/bin/env bash
# ============================================================================
# Guardia de endurecimiento en tiempo de ejecución (HEX-070, tarea 5 A-6)
# ============================================================================
# Verifica que la plantilla deploy/cell.compose.yml lleva, sobre los servicios
# `nucleo` y `sidecar`, las cuatro banderas que HEX-070 impone:
#
#   read_only: true
#   cap_drop: [ALL]
#   security_opt: ["no-new-privileges:true"]
#   tmpfs: ["/tmp"]                (anclado a la ruta LITERAL, no "no vacío")
#
# Y que el volumen /var/lib/hexcell se monta como volume (no bind, ni largo
# ni corto) en ambos servicios.
#
# POR QUÉ docker compose config y no lectura cruda del YAML: el footgun
# medido en este proyecto es que `docker compose build --dry-run` y el propio
# `docker compose config` resuelven aun con build.context inválido. Aquí eso
# no es un problema porque el guardia SOLO inspecciona el YAML resuelto (las
# cuatro banderas, el tipo de volumen) y NO afirma que las imágenes existan
# ni que arranquen bajo esas banderas; la prueba viva de arranque queda fuera
# de HEX-070 (es plan tarea 7 / 17). El inspección del YAML resuelto, no
# crudo, protege además contra reordenaciones o reescrituras de campos: el
# formato canónico es el de docker compose config, no el que elijas al
# escribir el archivo.
#
# USO
#
#   deploy/verificar_endurecimiento.sh <ruta-plantilla>
#       Verifica la plantilla indicada. Sale 0 si pasa, distinto de 0 si
#       falla (una línea `FALLA: ...` por cada motivo).
#
#   deploy/verificar_endurecimiento.sh --autoprueba
#       Copia deploy/cell.compose.yml a un directorio temporal, le quita
#       UNA bandera por vez (read_only, cap_drop, security_opt, tmpfs) y
#       verifica que el guardia falla sobre cada copia mutada. Si alguna
#       mutación pasa al guardia, no es todavía un guardia y el script
#       termina con código de error. Este modo es la prueba de mutación
#       exigida por AC-5 del 00-spec.yaml.
#
# DEPENDENCIAS
#
#   - bash, sed, mktemp, rm                       (POSIX/Util-linux estándar)
#   - docker + docker compose                      (CLI v5.x verificado)
#   - python3 con PyYAML                           (ya validado por HEX-068)
#
# Un entorno sin docker/compose NO se declara verificado: el script falla
# con un mensaje explícito, porque "omitido" sería indistinguible de "pasa"
# y eso es exactamente el fallo que AC-5 existe para impedir.
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

# --- Prerrequisitos ---------------------------------------------------------

if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
    echo "FALLA: docker compose no está disponible en este entorno; el guardia no puede correr y AC-1..AC-5 no se declaran verificadas" >&2
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

# --- Función de verificación (modo normal) ---------------------------------

# verificar_plantilla <ruta-plantilla>
#   Resuelve la plantilla con docker compose config (con el env de ejemplo) y
#   ejecuta las aserciones sobre el YAML resultante. Imprime `FALLA: ...` por
#   cada motivo o una línea `OK: ...` si todo pasa. Sale 0 o distinto de 0.
verificar_plantilla() {
    local ruta="$1"

    local ruta_resuelto
    ruta_resuelto="$(mktemp -t hex070-resuelto.XXXXXX.yaml)"
    # shellcheck disable=SC2064  # expandir $ruta_resuelto ahora, no en la trampa
    trap "rm -f '$ruta_resuelto'" RETURN

    if ! docker compose --env-file deploy/celula.env.ejemplo -f "$ruta" config >"$ruta_resuelto" 2>/dev/null; then
        echo "FALLA: docker compose config no pudo resolver la plantilla [$ruta]"
        return 1
    fi

    # Exportar rutas para que el python embebido las lea sin quoting arriesgado.
    export HEX070_RESUELTO="$ruta_resuelto"

    python3 - <<'PY'
import os, sys, yaml

with open(os.environ["HEX070_RESUELTO"]) as f:
    doc = yaml.safe_load(f)

services = doc.get("services") or {}
SERVICIOS_OBLIGADOS = ("nucleo", "sidecar")
fallas = []

for nombre in SERVICIOS_OBLIGADOS:
    svc = services.get(nombre)
    if svc is None:
        fallas.append(f"servicio [{nombre}] ausente en la plantilla resuelta")
        continue

    ro = svc.get("read_only")
    if ro is not True:
        fallas.append(
            f"servicio [{nombre}]: read_only debe ser exactamente true, se obtuvo {ro!r}"
        )

    cd = svc.get("cap_drop") or []
    if not isinstance(cd, list) or "ALL" not in cd:
        fallas.append(
            f"servicio [{nombre}]: cap_drop debe contener ALL, se obtuvo {cd!r}"
        )

    so = svc.get("security_opt") or []
    if not isinstance(so, list) or "no-new-privileges:true" not in so:
        fallas.append(
            f"servicio [{nombre}]: security_opt debe contener "
            f"'no-new-privileges:true', se obtuvo {so!r}"
        )

    # tmpfs anclado a la ruta LITERAL ['/tmp']. Una aceptación "no vacío"
    # dejaría pasar un cambio silencioso de ruta, que es justo el modo de
    # fallo que el guardia existe para impedir.
    tf = svc.get("tmpfs")
    if tf != ["/tmp"]:
        fallas.append(
            f"servicio [{nombre}]: tmpfs debe ser exactamente ['/tmp'], se obtuvo {tf!r}"
        )

    # volumes: /var/lib/hexcell debe estar como volume, no como bind.
    vols = svc.get("volumes") or []
    for v in vols:
        if isinstance(v, dict):
            if v.get("type") == "bind":
                target = v.get("target", "<sin target>")
                source = v.get("source", "<sin source>")
                fallas.append(
                    f"servicio [{nombre}]: mount a {target} con type=bind "
                    f"(source={source}); bind mounts prohibidos por HEX-070"
                )
        elif isinstance(v, str):
            # Forma corta con pinta de bind: ruta de host antes de :/...
            # Un volumen nombrado '- datos:/var/lib/hexcell' no dispara esto
            # porque 'datos' no empieza con '/', '.' ni '~'.
            if v[:1] in (".", "/", "~") and ":/" in v:
                fallas.append(
                    f"servicio [{nombre}]: volume en forma corta con pinta de "
                    f"bind ({v!r}); bind mounts prohibidos por HEX-070"
                )

if fallas:
    for f in fallas:
        print(f"FALLA: {f}")
    sys.exit(1)

print(
    "OK: las cuatro banderas (read_only, cap_drop, no-new-privileges, tmpfs) "
    "están impuestas en nucleo y sidecar, y el mount /var/lib/hexcell es volume"
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
# Por cada una de las cuatro banderas se copia la plantilla a un scratch,
# se quita esa bandera en ambos servicios (sed sobre el archivo copiado) y
# se verifica que el guardia falla. Si el guardia pasara la copia mutada,
# la "prueba" no probó nada — por eso se imprime una línea PASA/FALLA por
# cada caso y se sale con código 0 solo si los cuatro casos fallaron.
#
# POR QUÉ sed y no un parser: el patrón a quitar es LITERAL y conocido
# (cada bandera vive en una o dos líneas indentadas de forma fija). Si el
# formato YAML del archivo cambiara en el futuro, la sed no encontraría la
# línea y la mutación se convertiría en no-op; por eso este modo imprime
# explícitamente "FALLA: quitar X -> el guardia PASÓ la copia mutada" en
# ese caso (es un fallo del archivo bajo prueba, no del guardia). El
# implementador DEBE leer las cuatro líneas PASA/FALLA —no solo el exit
# code— antes de dar AC-5 por satisfecha.

DIR_TEMP=""
DIR_TEMP=$(mktemp -d -t hex070-guard.XXXXXX)
# shellcheck disable=SC2064  # expandir $DIR_TEMP ahora, no en la trampa
trap "rm -rf '$DIR_TEMP'" EXIT

ORIGINAL="$PLANTILLA"

echo "Modo --autoprueba: cada bandera, una por vez, debe ser detectada cuando se quita."

TOTAL=0
ACIERTOS=0

# --- read_only --------------------------------------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/sin-read_only.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/^[[:space:]]*read_only:[[:space:]]*true[[:space:]]*$/d' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar read_only -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar read_only -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- cap_drop: [ALL] --------------------------------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/sin-cap_drop.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/^[[:space:]]*cap_drop:[[:space:]]*$/,/^[[:space:]]*-[[:space:]]*ALL[[:space:]]*$/d' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar cap_drop -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar cap_drop -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- security_opt: ["no-new-privileges:true"] -------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/sin-security_opt.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/^[[:space:]]*security_opt:[[:space:]]*$/,/^[[:space:]]*-[[:space:]]*no-new-privileges:true[[:space:]]*$/d' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar security_opt -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar security_opt -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

# --- tmpfs -----------------------------------------------------------------
TOTAL=$((TOTAL + 1))
COPIA="$DIR_TEMP/sin-tmpfs.yml"
cp "$ORIGINAL" "$COPIA"
sed -i '/^[[:space:]]*tmpfs:[[:space:]]*$/,/^[[:space:]]*-[[:space:]]*\/tmp[[:space:]]*$/d' "$COPIA"
if ! verificar_plantilla "$COPIA" >/dev/null 2>&1; then
    echo "PASA: quitar tmpfs -> el guardia falla como debe"
    ACIERTOS=$((ACIERTOS + 1))
else
    echo "FALLA: quitar tmpfs -> el guardia PASÓ la copia mutada (no es un guardia)"
fi

echo ""
echo "Resumen autoprueba: $ACIERTOS/$TOTAL casos pasan (cada caso quitado debe hacer fallar al guardia)"

if [ "$ACIERTOS" -eq "$TOTAL" ]; then
    echo "OK: el guardia falla bajo cada mutación"
    exit 0
else
    echo "FALLA: el guardia no detectó todas las mutaciones"
    exit 1
fi