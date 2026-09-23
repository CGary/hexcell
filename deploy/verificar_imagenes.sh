#!/usr/bin/env bash
# USO
#   deploy/verificar_imagenes.sh                          (etiquetas locales,
#       las construye si faltan con los mismos contextos que CI: `.` +
#       ./Dockerfile para el núcleo, ./sidecar + ./sidecar/Dockerfile)
#   deploy/verificar_imagenes.sh <ref-nucleo> <ref-sidecar>
#       (imágenes ya construidas; CI pasa hexcell-nucleo:<sha12>
#       hexcell-sidecar:<sha12> cargadas con load:true. No construye aquí.)
#
# DEPENDENCIAS: bash, sed, mktemp, date; docker con daemon vivo; alpine:3
# descargable (imagen base ya usada por ambos Dockerfiles). Un entorno sin
# daemon NO se declara verificado: el script falla con mensaje explícito,
# porque "omitido" sería indistinguible de "pasa".
# ============================================================================

set -u

# Sufijo per-corrida: los objetos de una corrida nunca colisionan con los de
# otra corrida concurrente o anterior (misma regla que
# deploy/verificar_apagado_ordenado.sh documenta).
RUN_ID="$(date +%s)-$$"

# Objetos Docker creados por ESTA corrida; limpiar los recorre al salir.
CONTENEDORES=""
REDES=""
VOLUMENES=""
IMAGENES_DESCARTABLES=""
DIR_TEMP=""

limpiar() {
    echo ""
    echo "Limpiando: contenedores, redes, volúmenes, imagen de autoprueba y directorio temporal..."
    local objeto
    for objeto in $CONTENEDORES; do docker rm -f "$objeto" >/dev/null 2>&1 || true; done
    for objeto in $REDES; do docker network rm "$objeto" >/dev/null 2>&1 || true; done
    for objeto in $VOLUMENES; do docker volume rm "$objeto" >/dev/null 2>&1 || true; done
    for objeto in $IMAGENES_DESCARTABLES; do docker rmi -f "$objeto" >/dev/null 2>&1 || true; done
    if [ -n "$DIR_TEMP" ]; then rm -rf "$DIR_TEMP" || true; fi
}
trap limpiar EXIT

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    echo "FALLA: docker (o su daemon) no está disponible en este entorno; el guardia no puede correr y AC-4..AC-9 no se declaran verificadas" >&2
    exit 1
fi

IMAGEN_NUCLEO="${1:-hexcell-nucleo:local}"
IMAGEN_SIDECAR="${2:-hexcell-sidecar:local}"

# Solo en la invocación sin argumentos se construyen las etiquetas locales si
# faltan, con los MISMOS contextos y Dockerfiles que CI resuelve. CI siempre
# pasa dos referencias explícitas recién cargadas y no debe construir aquí.
if [ "$#" -eq 0 ]; then
    if ! docker image inspect "$IMAGEN_NUCLEO" >/dev/null 2>&1; then
        echo "Construyendo ${IMAGEN_NUCLEO} (contexto .) ..."
        docker build -t "$IMAGEN_NUCLEO" . || { echo "FALLA: no se pudo construir $IMAGEN_NUCLEO" >&2; exit 1; }
    fi
    if ! docker image inspect "$IMAGEN_SIDECAR" >/dev/null 2>&1; then
        echo "Construyendo ${IMAGEN_SIDECAR} (contexto ./sidecar) ..."
        docker build -t "$IMAGEN_SIDECAR" ./sidecar || { echo "FALLA: no se pudo construir $IMAGEN_SIDECAR" >&2; exit 1; }
    fi
fi

FALLAS=0

# verificar_usuario_no_root <imagen>  (AC-4)
#   Falla si .Config.User no es la cadena exacta 10001:10001 (incluida la
#   cadena vacía de una imagen sin USER). El literal es el mismo que fijan
#   ambos Dockerfiles (tarea 4 de A-6).
verificar_usuario_no_root() {
    local imagen="$1"
    local usuario
    if ! usuario="$(docker image inspect --format '{{.Config.User}}' "$imagen" 2>/dev/null)"; then
        echo "FALLA: [$imagen] no existe o no se pudo inspeccionar"
        return 1
    fi
    if [ "$usuario" != "10001:10001" ]; then
        echo "FALLA: [$imagen] .Config.User es [${usuario}] y debe ser exactamente 10001:10001"
        return 1
    fi
    echo "OK: [$imagen] .Config.User es 10001:10001"
    return 0
}

# caso_usuario_rechaza_imagen_root  (AC-5, autoprueba negativa)
#   Construye una imagen `FROM alpine:3` SIN USER en un directorio temporal y
#   exige que verificar_usuario_no_root la RECHAZE: si la acepta, la
#   comprobación del AC-4 sería una constante capturada, no un guardia.
caso_usuario_rechaza_imagen_root() {
    DIR_TEMP="$(mktemp -d -t hex086-raiz.XXXXXX)"
    local imagen_raiz="hex086-raiz-${RUN_ID}"
    printf 'FROM alpine:3\n' > "$DIR_TEMP/Dockerfile"
    if ! docker build -q -t "$imagen_raiz" "$DIR_TEMP" >/dev/null 2>&1; then
        echo "FALLA: no se pudo construir la imagen raíz de la autoprueba"
        return 1
    fi
    IMAGENES_DESCARTABLES="$IMAGENES_DESCARTABLES $imagen_raiz"
    local salida
    if salida="$(verificar_usuario_no_root "$imagen_raiz" 2>&1)"; then
        echo "FALLA: el guardia de usuario aceptó una imagen FROM alpine:3 sin USER"
        return 1
    fi
    # El rechazo debe ser por el usuario vacío de ESA imagen, no por una
    # imagen ausente o un inspect roto: si no, la autoprueba pasa en vacío.
    if [ "$salida" != "FALLA: [$imagen_raiz] .Config.User es [] y debe ser exactamente 10001:10001" ]; then
        echo "FALLA: el guardia de usuario rechazó la imagen raíz por otro motivo: ${salida}"
        return 1
    fi
    echo "OK: el guardia de usuario rechazó una imagen sin USER (autoprueba)"
    return 0
}

# verificar_arranque_en_frio_nucleo <rotulo> <con_volumen>  (AC-6 y AC-8)
#   Arranca el núcleo con las cuatro banderas de endurecimiento de la plantilla
#   sobre una red y un volumen (si con_volumen=1) nombrados y VACÍOS, con
#   HEXCELL_ID_CELULA=ci, HEXCELL_RUTA_DATOS=/var/lib/hexcell (punto de montaje
#   del volumen) y HEXCELL_DIRECCION_SALUD=0.0.0.0:8081, y un contenedor hermano
#   alpine:3 en la misma red sondea GET /health/live con 200 en 30 s.
#
#   POR QUÉ SE SONDEA EL NÚCLEO SOLO, SIN EL SIDECAR: en
#   crates/hexcell/src/salud.rs, atender_peticion_de_salud responde /health/live
#   con un 200 fijo de cuerpo "viva" sin leer pools ni estado de sesión del
#   canal (a diferencia de /health/ready). Verificado el 2026-09-23 arrancando
#   la imagen real con exactamente estas banderas: 200 en menos de 2 s; el
#   sidecar no participa en esta comprobación.
#
#   El AC-8 (núcleo sin volumen) reutiliza esta misma función y exige que
#   reporte FALLA: el contenedor muere solo (~2 s, ExitCode=1, SQLite no puede
#   abrir sessions.db contra un rootfs de solo lectura sin ruta de datos
#   escribible; verificado el 2026-09-23), por lo que esta función lee
#   `docker inspect` State.Running/ExitCode en cada vuelta y NO espera el
#   timeout de 30 s de la sonda.
verificar_arranque_en_frio_nucleo() {
    local rotulo="$1"
    local con_volumen="$2"
    local contenedor="hex086-${rotulo}-${RUN_ID}"
    local red="hex086-red-${rotulo}-${RUN_ID}"
    local volumen="hex086-vol-${rotulo}-${RUN_ID}"

    if ! docker network create "$red" >/dev/null 2>&1; then
        echo "FALLA: no se pudo crear la red $red"
        return 1
    fi
    REDES="$REDES $red"

    local args=(run -d --name "$contenedor" --read-only --tmpfs /tmp \
        --cap-drop ALL --security-opt no-new-privileges:true \
        -e HEXCELL_ID_CELULA=ci \
        -e HEXCELL_RUTA_DATOS=/var/lib/hexcell \
        -e HEXCELL_DIRECCION_SALUD=0.0.0.0:8081 \
        --network "$red")
    if [ "$con_volumen" -eq 1 ]; then
        if ! docker volume create "$volumen" >/dev/null 2>&1; then
            echo "FALLA: no se pudo crear el volumen $volumen"
            return 1
        fi
        VOLUMENES="$VOLUMENES $volumen"
        args+=(-v "$volumen:/var/lib/hexcell")
    fi
    args+=("$IMAGEN_NUCLEO")

    if ! docker "${args[@]}" >/dev/null 2>&1; then
        echo "FALLA: no se pudo arrancar el contenedor $contenedor"
        return 1
    fi
    CONTENEDORES="$CONTENEDORES $contenedor"

    # Sonda con lectura de State.Running en cada vuelta (el AC-8 depende de
    # no esperar el timeout de 30 s cuando el contenedor muere solo). La sonda
    # lleva --rm y además nombre rastreado por limpiar: ninguna sonda huérfana
    # sobrevive a un corte del script a mitad de comprobación.
    local limite=30 inicio ahora corriendo codigo sonda intento=0
    inicio="$(date +%s)"
    while true; do
        corriendo="$(docker inspect --format '{{.State.Running}}' "$contenedor" 2>/dev/null)"
        if [ "$corriendo" != "true" ]; then
            codigo="$(docker inspect --format '{{.State.ExitCode}}' "$contenedor" 2>/dev/null)"
            echo "FALLA: [$contenedor] dejó de correr antes de responder /health/live (ExitCode=$codigo)"
            return 1
        fi
        intento=$((intento + 1))
        sonda="hex086-sonda-${rotulo}-${RUN_ID}-${intento}"
        if docker run --rm --name "$sonda" --network "$red" alpine:3 \
            wget -q -O - --timeout=5 "http://${contenedor}:8081/health/live" 2>/dev/null \
            | grep -q "viva"; then
            echo "OK: [$contenedor] respondió /health/live con 200 bajo endurecimiento"
            return 0
        fi
        CONTENEDORES="$CONTENEDORES $sonda"
        ahora="$(date +%s)"
        if [ $((ahora - inicio)) -ge "$limite" ]; then
            echo "FALLA: [$contenedor] no respondió /health/live con 200 en ${limite} s"
            return 1
        fi
        sleep 1
    done
}

# caso_arranque_en_frio_nucleo_sin_volumen_falla  (AC-8, autoprueba negativa)
#   El MISMO guardia, aplicado a un núcleo --read-only SIN su volumen de
#   datos, debe reportar FALLA. Si reportara PASA, la comprobación de arranque
#   sería una constante capturada y no un guardia.
caso_arranque_en_frio_nucleo_sin_volumen_falla() {
    local contenedor="hex086-nucleo-sinvolumen-${RUN_ID}"
    if verificar_arranque_en_frio_nucleo "nucleo-sinvolumen" 0 >/dev/null 2>&1; then
        echo "FALLA: el guardia de arranque aceptó un núcleo --read-only sin volumen de datos"
        return 1
    fi
    local codigo
    codigo="$(docker inspect --format '{{.State.ExitCode}}' "$contenedor" 2>/dev/null)"
    # El contenedor debe haber existido y muerto con código distinto de 0: si
    # nunca arrancó (imagen ausente, run fallido) la autoprueba pasa en vacío.
    if [ -z "$codigo" ] || [ "$codigo" = "0" ]; then
        echo "FALLA: el núcleo sin volumen no llegó a arrancar y morir (ExitCode=[${codigo}])"
        return 1
    fi
    echo "OK: el guardia de arranque rechazó el núcleo --read-only sin volumen (ExitCode=$codigo, autoprueba)"
    return 0
}

# verificar_arranque_en_frio_sidecar  (AC-7)
#   Arranca el sidecar con las mismas cuatro banderas de endurecimiento que el
#   núcleo y un volumen vacío propio. Única variable de entorno:
#   HEXCELL_VENTANA_ZONA, la única estrictamente requerida por el sidecar
#   (HEX-033; verificado el 2026-09-23 contra la imagen real: con solo esta
#   variable alcanza Running en menos de 10 s sin las dos subcadenas
#   prohibidas). A los 10 s exige State.Running=true y logs sin las dos
#   subcadenas prohibidas.
verificar_arranque_en_frio_sidecar() {
    local contenedor="hex086-sidecar-${RUN_ID}"
    local red="hex086-red-sidecar-${RUN_ID}"
    local volumen="hex086-vol-sidecar-${RUN_ID}"

    if ! docker network create "$red" >/dev/null 2>&1; then
        echo "FALLA: no se pudo crear la red $red"
        return 1
    fi
    REDES="$REDES $red"

    if ! docker volume create "$volumen" >/dev/null 2>&1; then
        echo "FALLA: no se pudo crear el volumen $volumen"
        return 1
    fi
    VOLUMENES="$VOLUMENES $volumen"

    if ! docker run -d --name "$contenedor" --read-only --tmpfs /tmp \
        --cap-drop ALL --security-opt no-new-privileges:true \
        -v "$volumen:/var/lib/hexcell" \
        -e HEXCELL_VENTANA_ZONA=America/Argentina/Buenos_Aires \
        --network "$red" "$IMAGEN_SIDECAR" >/dev/null 2>&1; then
        echo "FALLA: no se pudo arrancar el contenedor $contenedor"
        return 1
    fi
    CONTENEDORES="$CONTENEDORES $contenedor"

    sleep 10

    local corriendo logs
    corriendo="$(docker inspect --format '{{.State.Running}}' "$contenedor" 2>/dev/null)"
    logs="$(docker logs "$contenedor" 2>&1)"
    if [ "$corriendo" != "true" ]; then
        echo "FALLA: [$contenedor] no está Running a los 10 s"
        return 1
    fi
    if printf '%s' "$logs" | grep -qE 'read-only file system|permission denied'; then
        echo "FALLA: [$contenedor] registró una escritura prohibida bajo rootfs de solo lectura"
        return 1
    fi
    echo "OK: [$contenedor] arrancó endurecido y sigue Running a los 10 s, sin escrituras prohibidas"
    return 0
}

# --- Ejecución ---------------------------------------------------------------

echo "Imágenes bajo verificación: ${IMAGEN_NUCLEO} y ${IMAGEN_SIDECAR}"
echo ""

verificar_usuario_no_root "$IMAGEN_NUCLEO" || FALLAS=$((FALLAS + 1))
verificar_usuario_no_root "$IMAGEN_SIDECAR" || FALLAS=$((FALLAS + 1))
caso_usuario_rechaza_imagen_root || FALLAS=$((FALLAS + 1))
verificar_arranque_en_frio_nucleo "nucleo" 1 || FALLAS=$((FALLAS + 1))
caso_arranque_en_frio_nucleo_sin_volumen_falla || FALLAS=$((FALLAS + 1))
verificar_arranque_en_frio_sidecar || FALLAS=$((FALLAS + 1))

echo ""
if [ "$FALLAS" -eq 0 ]; then
    echo "OK: usuario no root y arranque en frío de solo lectura verificados — AC-4..AC-9 pasan"
    exit 0
else
    echo "FALLA: ${FALLAS} comprobación(es) fallaron; ver detalle arriba"
    exit 1
fi
