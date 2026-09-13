#!/usr/bin/env bash
# ============================================================================
# Verificación MANUAL y en VIVO del aislamiento entre dos células
# (HEX-076, tarea 17 A-6)
# ============================================================================
# Levanta DOS células reales (A y B) desde deploy/cell.compose.yml, cada una
# en su propia red y su propio volumen nombrado, arrancando en frío desde
# volúmenes vacíos, y demuestra que ninguna alcanza a la otra:
#
#   1. Cruce de volumen: ni lectura ni escritura de A contra el volumen de B.
#   2. Alcance de red: A no llega al núcleo ni al sidecar de B, ni por nombre
#      de contenedor ni por IP cruda.
#   3. Socket IPC ajeno: el socket de A en /var/lib/hexcell/ipc/sidecar.sock
#      nunca es, físicamente, el socket real de B, aunque la ruta interna
#      sea idéntica en ambas células.
#   4. Ningún servicio de ninguna célula publica un puerto al host.
#
# ESTE SCRIPT NO ES UN GUARDIA MECÁNICO: levanta contenedores reales, tarda
# minutos y depende de un daemon Docker vivo. Por eso NUNCA se invoca desde
# .github/workflows/ci.yml ni desde verify.commands — deploy/verificar_aislamiento_estatica.sh
# es el único guardia mecánico de esta tarea, y este script tampoco implementa
# el modo de autoprueba de mutación (exclusivo de ese guardia).
#
# CÓMO SE INSPECCIONA DESDE "EL PUNTO DE VISTA DE A" SIN `docker exec`
#
# El endurecimiento de HEX-069 retira /bin/sh Y /bin/busybox de las dos
# imágenes finales (Dockerfile, sidecar/Dockerfile): un `docker exec` contra
# los contenedores `nucleo`/`sidecar` reales no tiene ningún binario que
# invocar salvo el propio ENTRYPOINT estático — ni `nc`, ni `stat`, ni `sh`.
# Por eso las comprobaciones de red y de volumen NO exec'an dentro de los
# contenedores endurecidos: levantan un contenedor auxiliar efímero
# `alpine:3` (la misma base ya usada por deploy/verificar_apagado_ordenado.sh
# para inspeccionar el WAL) con `--network container:<contenedor>` y
# `--volumes-from <contenedor>`. Eso comparte el espacio de nombres de red y
# los montajes exactos del contenedor objetivo —ve la misma red, las mismas
# rutas de volumen— sin modificar la imagen endurecida ni añadir una
# herramienta nueva a ella. Es la vía prescrita por el hallazgo D-48 de
# docs/bitacora-de-descartes.md.
#
# USO
#
#   deploy/verificar_aislamiento.sh [ruta-plantilla]
#       Por omisión usa deploy/cell.compose.yml. Sale 0 si las once
#       aserciones (AC-5..AC-11 del 00-spec.yaml) pasan, distinto de 0 si
#       alguna falla, con una línea `FALLA: ...` por cada motivo. Si un
#       cruce inesperado tiene éxito (AC-10), el resumen final señala que
#       deploy/cell.compose.yml requiere corrección en esta misma tarea.
#
# DEPENDENCIAS
#
#   - bash, sed, mktemp, date                      (POSIX/Util-linux estándar)
#   - docker + docker compose                       (CLI v5.x verificado)
#   - deploy/celula.env.ejemplo como referente de variables
#   - imagen alpine:3 (contenedor auxiliar efímero; ya usada por
#     deploy/verificar_apagado_ordenado.sh, ninguna herramienta nueva)
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
    echo "FALLA: docker compose no está disponible en este entorno; el cruce en vivo entre dos células no se puede verificar" >&2
    exit 1
fi

if [ ! -f deploy/celula.env.ejemplo ]; then
    echo "FALLA: deploy/celula.env.ejemplo no existe; no hay referente de variables para levantar las células" >&2
    exit 1
fi

# --- Nombres de un solo uso, por célula --------------------------------------

SUFIJO="$(date +%s)-$$"

ID_CELULA_A="hex076verifa${SUFIJO}"
RED_A="hex076-verif-red-a-${SUFIJO}"
VOLUMEN_A="hex076-verif-vol-a-${SUFIJO}"
PROYECTO_A="hex076verifa${SUFIJO}"
CONTENEDOR_NUCLEO_A="${ID_CELULA_A}-nucleo"
CONTENEDOR_SIDECAR_A="${ID_CELULA_A}-sidecar"
ENV_TEMP_A="$(mktemp -t hex076-env-a.XXXXXX)"

ID_CELULA_B="hex076verifb${SUFIJO}"
RED_B="hex076-verif-red-b-${SUFIJO}"
VOLUMEN_B="hex076-verif-vol-b-${SUFIJO}"
PROYECTO_B="hex076verifb${SUFIJO}"
CONTENEDOR_NUCLEO_B="${ID_CELULA_B}-nucleo"
CONTENEDOR_SIDECAR_B="${ID_CELULA_B}-sidecar"
ENV_TEMP_B="$(mktemp -t hex076-env-b.XXXXXX)"

# stderr del ÚLTIMO `docker run` auxiliar (efímero, uno a la vez: el script
# es secuencial) más su código de salida, para distinguir "el auxiliar nunca
# llegó a correr la comprobación" de un resultado negativo genuino. Los
# rellenan desde()/leer_volumen()/escribir_volumen(); ver fallo_de_auxiliar().
ERR_TEMP="$(mktemp -t hex076-err.XXXXXX)"
CODIGO_AUXILIAR=0
ERR_AUXILIAR=""

limpiar() {
    echo ""
    echo "Limpiando: contenedores, redes y volúmenes de un solo uso de A y B..."
    docker compose -p "$PROYECTO_A" --env-file "$ENV_TEMP_A" -f "$PLANTILLA" down --volumes --remove-orphans >/dev/null 2>&1 || true
    docker compose -p "$PROYECTO_B" --env-file "$ENV_TEMP_B" -f "$PLANTILLA" down --volumes --remove-orphans >/dev/null 2>&1 || true
    docker volume rm "$VOLUMEN_A" "$VOLUMEN_B" >/dev/null 2>&1 || true
    docker network rm "$RED_A" "$RED_B" >/dev/null 2>&1 || true
    rm -f "$ENV_TEMP_A" "$ENV_TEMP_B" "$ERR_TEMP"
}
trap limpiar EXIT

# Copia deploy/celula.env.ejemplo y sustituye solo el identificador, la red y
# el volumen por nombres únicos de esta corrida: ambas células arrancan en
# frío desde un volumen VACÍO cada vez, jamás reutilizado.
sed -E \
    -e "s/^HEXCELL_ID_CELULA=.*/HEXCELL_ID_CELULA=${ID_CELULA_A}/" \
    -e "s/^HEXCELL_RED_CELULA=.*/HEXCELL_RED_CELULA=${RED_A}/" \
    -e "s/^HEXCELL_VOLUMEN_CELULA=.*/HEXCELL_VOLUMEN_CELULA=${VOLUMEN_A}/" \
    deploy/celula.env.ejemplo >"$ENV_TEMP_A"

sed -E \
    -e "s/^HEXCELL_ID_CELULA=.*/HEXCELL_ID_CELULA=${ID_CELULA_B}/" \
    -e "s/^HEXCELL_RED_CELULA=.*/HEXCELL_RED_CELULA=${RED_B}/" \
    -e "s/^HEXCELL_VOLUMEN_CELULA=.*/HEXCELL_VOLUMEN_CELULA=${VOLUMEN_B}/" \
    deploy/celula.env.ejemplo >"$ENV_TEMP_B"

echo "Célula A: ${ID_CELULA_A} (red ${RED_A}, volumen ${VOLUMEN_A})"
echo "Célula B: ${ID_CELULA_B} (red ${RED_B}, volumen ${VOLUMEN_B})"
echo ""
echo "=== Arranque en frío de A y B desde volúmenes vacíos, sidecars sin emparejar ==="

if ! docker compose -p "$PROYECTO_A" --env-file "$ENV_TEMP_A" -f "$PLANTILLA" up -d --build; then
    echo "FALLA: docker compose up no pudo levantar la célula A"
    exit 1
fi

if ! docker compose -p "$PROYECTO_B" --env-file "$ENV_TEMP_B" -f "$PLANTILLA" up -d --build; then
    echo "FALLA: docker compose up no pudo levantar la célula B"
    exit 1
fi

FALLAS=0

registrar_falla() {
    echo "FALLA: $1"
    FALLAS=$((FALLAS + 1))
}

registrar_ok() {
    echo "OK: $1"
}

# --- Señal de vida: línea de arranque conocida, NO /health/ready -----------
# Mismo criterio que deploy/verificar_apagado_ordenado.sh: una célula
# deliberadamente sin emparejar nunca alcanza el estado "sesión de canal
# activa", así que esperar un 200 de /health/ready colgaría el script.

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

for par in "$CONTENEDOR_NUCLEO_A:nucleo A" "$CONTENEDOR_NUCLEO_B:nucleo B" ; do
    contenedor="${par%%:*}"
    etiqueta="${par#*:}"
    if esperar_arranque "$contenedor" 'hexcell: canal configurado: whatsmeow'; then
        registrar_ok "el ${etiqueta} señaló arranque (canal whatsmeow configurado)"
    else
        registrar_falla "el ${etiqueta} no señaló arranque en 30 s"
    fi
done

for par in "$CONTENEDOR_SIDECAR_A:sidecar A" "$CONTENEDOR_SIDECAR_B:sidecar B" ; do
    contenedor="${par%%:*}"
    etiqueta="${par#*:}"
    if esperar_arranque "$contenedor" '"evento":"sidecar.arrancado"'; then
        registrar_ok "el ${etiqueta} señaló arranque (sidecar.arrancado)"
    else
        registrar_falla "el ${etiqueta} no señaló arranque en 30 s"
    fi
done

if [ "$FALLAS" -gt 0 ]; then
    echo "FALLA: alguna célula no llegó a un estado vivo verificable; se aborta antes de ejercer los vectores de cruce (AC-5)"
    exit 1
fi

# --- Auxiliar: correr un comando "desde el punto de vista de" un contenedor -
#
# Comparte la red y los montajes del contenedor dado sin exec'ar dentro de
# él (ver nota de cabecera / D-48): --network container:<X> pone al
# auxiliar en el MISMO espacio de nombres de red que X (mismas interfaces,
# misma tabla de rutas — ni más ni menos alcance que X), y --volumes-from
# <X> monta exactamente los mismos volúmenes de X, en las mismas rutas.
desde() {
    local contenedor="$1"
    shift
    docker run --rm --network "container:${contenedor}" --volumes-from "${contenedor}" alpine:3 sh -c "$*" 2>"$ERR_TEMP"
    CODIGO_AUXILIAR=$?
    ERR_AUXILIAR="$(cat "$ERR_TEMP" 2>/dev/null)"
    [ -n "$ERR_AUXILIAR" ] && echo "$ERR_AUXILIAR" >&2
    return "$CODIGO_AUXILIAR"
}

# Auxiliar de solo lectura sobre un volumen ajeno, sin tocar red ni montar
# nada del contenedor: mismo patrón que el chequeo del WAL en
# deploy/verificar_apagado_ordenado.sh.
leer_volumen() {
    local volumen="$1"
    shift
    docker run --rm -v "${volumen}:/datos-ajenos:ro" alpine:3 sh -c "$*" 2>"$ERR_TEMP"
    CODIGO_AUXILIAR=$?
    ERR_AUXILIAR="$(cat "$ERR_TEMP" 2>/dev/null)"
    [ -n "$ERR_AUXILIAR" ] && echo "$ERR_AUXILIAR" >&2
    return "$CODIGO_AUXILIAR"
}

escribir_volumen() {
    local volumen="$1"
    shift
    docker run --rm -v "${volumen}:/datos-ajenos" alpine:3 sh -c "$*" 2>"$ERR_TEMP"
    CODIGO_AUXILIAR=$?
    ERR_AUXILIAR="$(cat "$ERR_TEMP" 2>/dev/null)"
    [ -n "$ERR_AUXILIAR" ] && echo "$ERR_AUXILIAR" >&2
    return "$CODIGO_AUXILIAR"
}

# Distingue "el auxiliar efímero nunca llegó a ejecutar la comprobación"
# (docker run fallido: imagen no disponible, --network container:<X> en
# carrera, límite de recursos) de un resultado negativo genuino (nc/test
# saliendo con 1). Docker usa 125 por convención cuando el propio `docker
# run` no pudo arrancar el contenedor; se complementa con un match de stderr
# por si el daemon reporta el fallo con otro código. Mismo criterio que ya
# usan el NOEXISTE de AC-8 y la precondición de resolución de IP de AC-7: no
# confundir "no se pudo probar" con "se probó y dio bien".
fallo_de_auxiliar() {
    local codigo="$1"
    local err="$2"
    if [ "$codigo" -eq 125 ]; then
        return 0
    fi
    case "$err" in
        *"Error response from daemon"*) return 0 ;;
    esac
    return 1
}

# --- AC-6: cruce de volumen (lectura y escritura) ---------------------------

echo ""
echo "=== AC-6: cruce de volumen entre A y B ==="

MARCADOR_B="marcador-aislamiento-B.txt"
MARCADOR_A="marcador-aislamiento-A.txt"

if ! escribir_volumen "$VOLUMEN_B" "echo secreto-de-B > /datos-ajenos/${MARCADOR_B}"; then
    registrar_falla "no se pudo plantar el marcador de B; el vector de lectura no se puede probar"
elif desde "$CONTENEDOR_NUCLEO_A" "test -f /var/lib/hexcell/${MARCADOR_B}"; then
    registrar_falla "A pudo LEER el marcador plantado en el volumen de B (cruce de volumen roto)"
elif fallo_de_auxiliar "$CODIGO_AUXILIAR" "$ERR_AUXILIAR"; then
    registrar_falla "el contenedor auxiliar no pudo ejecutarse contra A (código ${CODIGO_AUXILIAR}); el vector de lectura no se puede probar"
else
    registrar_ok "A no pudo leer el marcador plantado en el volumen de B"
fi

if ! desde "$CONTENEDOR_NUCLEO_A" "echo escrito-desde-A > /var/lib/hexcell/${MARCADOR_A}"; then
    registrar_falla "A no pudo escribir en su propio volumen; el vector de escritura no se puede probar"
elif leer_volumen "$VOLUMEN_B" "test -f /datos-ajenos/${MARCADOR_A}"; then
    registrar_falla "el archivo que A escribió apareció en el volumen de B (cruce de volumen roto)"
elif fallo_de_auxiliar "$CODIGO_AUXILIAR" "$ERR_AUXILIAR"; then
    registrar_falla "el contenedor auxiliar no pudo ejecutarse contra el volumen de B (código ${CODIGO_AUXILIAR}); el vector de escritura no se puede probar"
else
    registrar_ok "lo que A escribió nunca apareció en el volumen de B"
fi

# --- AC-7: alcance de red (núcleo y sidecar de B, por nombre y por IP) ------

echo ""
echo "=== AC-7: alcance de red desde A hacia B ==="

IP_NUCLEO_B="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$CONTENEDOR_NUCLEO_B" 2>/dev/null)"
IP_SIDECAR_B="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$CONTENEDOR_SIDECAR_B" 2>/dev/null)"

if [ -z "$IP_NUCLEO_B" ] || [ -z "$IP_SIDECAR_B" ]; then
    registrar_falla "no se pudo obtener la IP de B vía docker inspect; los vectores por IP cruda no se pueden probar"
else
    # Se prueba contra el NOMBRE DE CONTENEDOR de B (globalmente único por el
    # sufijo de esta corrida), no contra el alias de servicio genérico
    # "nucleo"/"sidecar": ese alias solo resuelve, dentro de la red de A, al
    # PROPIO contenedor de A que lleva ese nombre de servicio — probarlo
    # daría un falso "éxito" al conectar con A mismo, no con B.
    for vector in \
        "nucleo B por nombre:${CONTENEDOR_NUCLEO_B}:8081" \
        "nucleo B por IP:${IP_NUCLEO_B}:8081" \
        "sidecar B por nombre:${CONTENEDOR_SIDECAR_B}:8081" \
        "sidecar B por IP:${IP_SIDECAR_B}:8081"
    do
        etiqueta="${vector%%:*}"
        resto="${vector#*:}"
        destino="${resto%%:*}"
        puerto="${resto#*:}"
        if desde "$CONTENEDOR_NUCLEO_A" "nc -w 2 -z ${destino} ${puerto}"; then
            registrar_falla "A alcanzó a ${etiqueta} (${destino}:${puerto}) — alcance de red roto"
        elif fallo_de_auxiliar "$CODIGO_AUXILIAR" "$ERR_AUXILIAR"; then
            registrar_falla "el contenedor auxiliar no pudo ejecutarse contra A para probar ${etiqueta} (código ${CODIGO_AUXILIAR}); el vector no se puede probar"
        else
            registrar_ok "A no alcanzó a ${etiqueta} (${destino}:${puerto})"
        fi
    done
fi

# --- AC-8: socket IPC ajeno --------------------------------------------------

echo ""
echo "=== AC-8: socket IPC de B desde A ==="

RUTA_SOCKET="/var/lib/hexcell/ipc/sidecar.sock"

# El sidecar crea el socket al escuchar; se espera hasta 10 s a que aparezca
# en el propio montaje de A antes de comparar identidades.
DISPOSITIVO_A="NOEXISTE"
intentos=10
while [ "$intentos" -gt 0 ]; do
    DISPOSITIVO_A="$(desde "$CONTENEDOR_NUCLEO_A" "stat -c %d ${RUTA_SOCKET} 2>/dev/null || echo NOEXISTE")"
    if [ "$DISPOSITIVO_A" != "NOEXISTE" ]; then
        break
    fi
    intentos=$((intentos - 1))
    sleep 1
done

DISPOSITIVO_B="$(leer_volumen "$VOLUMEN_B" "stat -c %d /datos-ajenos/ipc/sidecar.sock 2>/dev/null || echo NOEXISTE")"

if [ "$DISPOSITIVO_A" = "NOEXISTE" ]; then
    registrar_falla "el socket IPC de A nunca apareció; el vector de socket ajeno no se puede probar"
elif [ "$DISPOSITIVO_B" = "NOEXISTE" ]; then
    registrar_falla "el socket IPC de B nunca apareció; el vector de socket ajeno no se puede probar"
elif [ "$DISPOSITIVO_A" = "$DISPOSITIVO_B" ]; then
    registrar_falla "el socket de A y el socket real de B viven en el MISMO dispositivo de archivos (dispositivo ${DISPOSITIVO_A}) — el aislamiento de volumen que respalda el socket IPC está roto"
else
    registrar_ok "el socket de A (dispositivo ${DISPOSITIVO_A}) y el socket real de B (dispositivo ${DISPOSITIVO_B}) son objetos de archivo distintos: la ruta idéntica de A jamás puede ser el socket de B"
fi

# --- AC-9: ningún servicio publica un puerto al host ------------------------

echo ""
echo "=== AC-9: ningún puerto publicado al host ==="

for contenedor in "$CONTENEDOR_NUCLEO_A" "$CONTENEDOR_SIDECAR_A" "$CONTENEDOR_NUCLEO_B" "$CONTENEDOR_SIDECAR_B"; do
    PUBLICADOS="$(docker port "$contenedor" 2>/dev/null)"
    if [ -n "$PUBLICADOS" ]; then
        registrar_falla "el contenedor [$contenedor] publica un puerto al host: ${PUBLICADOS}"
    else
        registrar_ok "el contenedor [$contenedor] no publica ningún puerto al host"
    fi
done

# --- Resumen -----------------------------------------------------------------

echo ""
if [ "$FALLAS" -eq 0 ]; then
    echo "OK: aislamiento verificado entre A y B — AC-5 a AC-9 pasan"
    exit 0
else
    echo "FALLA: ${FALLAS} aserción(es) fallaron; ver detalle arriba"
    echo "AC-10: si el motivo es un cruce real (no un problema ambiental de esta corrida), corresponde corregir deploy/cell.compose.yml en esta misma tarea y volver a correr este script hasta que las once aserciones pasen"
    exit 1
fi
