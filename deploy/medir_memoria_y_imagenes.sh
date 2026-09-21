#!/usr/bin/env bash
# ============================================================================
# Medición MANUAL y en VIVO de la memoria (cgroup v2) y del tamaño de imagen
# de la célula compuesta (HEX-079, tarea 16 A-6)
# ============================================================================
# Instrumento de medición, no un guardia mecánico: levanta la célula
# compuesta de la tarea 5 (deploy/cell.compose.yml) con el adaptador whatsmeow
# bajo los límites de cgroup de la tarea 6, y lee:
#
#   1. La memoria AGREGADA de ambos contenedores (núcleo + sidecar) desde los
#      directorios cgroup v2 de cada contenedor bajo el árbol sysfs del
#      anfitrión (/sys/fs/cgroup), en reposo y bajo carga.
#   2. El tamaño de ambas imágenes con docker image inspect.
#
# ESTE SCRIPT NO ES UN GUARDIA MECÁNICO: levanta contenedores reales, tarda
# minutos y depende de un daemon Docker vivo. Por eso NUNCA se invoca desde
# .github/workflows/ci.yml ni desde verify.commands — la decisión de HEX-067
# derogó la compuerta de tamaño en CI, y este instrumento tampoco implementa
# el modo de autoprueba de mutación, exclusivo de los guardias mecánicos.
# Entrega el INSTRUMENTO; los números reales los registra a mano el operador
# en la sección «Valores de referencia de memoria y tamaño de imágenes» de
# docs/plantilla-celula.md, en una corrida manual posterior (AC-6), igual que
# se manejó deploy/verificar_aislamiento.sh.
#
# POR QUÉ se lee la memoria desde el cgroup v2 del ANFITRIÓN y no desde
# dentro del contenedor: el endurecimiento de HEX-069 retira /bin/sh Y
# /bin/busybox de las dos imágenes finales (Dockerfile, sidecar/Dockerfile),
# de modo que no queda ningún binario que invocar dentro del contenedor salvo
# el ENTRYPOINT estático; el subcomando exec de la CLI de docker no puede
# ejecutar ni siquiera `cat`. La lectura se hace desde el árbol /sys/fs/cgroup
# del anfitrión, que es la contabilidad del kernel para la cgroup de cada
# contenedor — cgroup2fs NO es /proc, y jamás se lee memoria del /proc del
# anfitrión ni se confía en `docker stats` (su informe por omisión puede
# derivar de /proc).
#
# POR QUÉ `anon` y `memory.current` se informan como dos cifras SEPARADAS:
# memory.current incluye la caché de página y la memoria del kernel; el
# equivalente a RSS es la clave `anon` de memory.stat. Informar solo
# memory.current sobrestimaría la cifra frente al techo de 80 MB de NFR-01, e
# informar solo anon subestimaría la presión de cgroup que dispara el OOM:
# un único número cuyo significado es ambiguo no es una medición.
#
# LIMITACION DEL GENERADOR DE CARGA (AC-3 del 00-spec.yaml)
#
# El escenario de crates/hexcell/tests/carga.rs NO se reutiliza como generador
# externo y NO se modifica para hacerlo embebible, por cuatro razones
# independientes, verificadas sobre el archivo:
#   1. Construye el `Motor` EN-PROCESO: no tiene ningún cliente de red ni de
#      IPC que apuntar contra una célula viva; no existe la pieza que lo
#      convierta en un generador externo.
#   2. Conduce `AdaptadorSimulado`, el adaptador SIMULADO: medir con él
#      contradice la exigencia de esta tarea, que es la célula compuesta con
#      el adaptador whatsmeow.
#   3. Usa `RelojDePrueba`, un reloj falso, para hacer determinista la
#      admisión GCRA: en un contenedor en marcha no existe un reloj falso.
#   4. Mide la memoria residente del proceso (`leer_vm_rss_kb`) leyendo el
#      archivo de estado del proceso en /proc: exactamente la fuente que el
#      invariante central de esta tarea prohíbe.
# En su lugar, la carga se genera con un contenedor auxiliar efímero alpine:3
# (la misma base que ya usan deploy/verificar_aislamiento.sh y
# deploy/verificar_apagado_ordenado.sh; ninguna herramienta nueva) unido al
# espacio de nombres de red del núcleo (--network container:<nucleo>), que
# dispara una ráfaga ACOTADA y paralela de GETs contra el listener de salud
# (0.0.0.0:8081 dentro de la red de la célula). NO se publica ningún puerto al
# host, preservando la invariante de aislamiento de HEX-076.
#
# ADVERTENCIA SOBRE LA CIFRA BAJO CARGA: el generador sustituto es SUPERFICIAL
# frente a la carga real del canal. Golpear /health/ready ejercita el listener
# HTTP, el runtime tokio, la rotación de conexiones y el asignador, pero NO la
# admisión GCRA, NO el pipeline de inferencia, NO el motor de conocimiento y
# NO la ruta whatsmeow del sidecar. La cifra bajo carga es, por tanto, una COTA
# INFERIOR, no el peor caso, y debe registrarse como tal en docs/; un valor
# bajo carga no puede ratificar por sí solo los valores provisionales de
# adr-0007.
#
# USO
#
#   deploy/medir_memoria_y_imagenes.sh [ruta-plantilla]
#       Por omisión usa deploy/cell.compose.yml. Requiere AMBAS imágenes
#       construidas (docker image inspect las comprueba como precondición) y
#       un daemon Docker vivo con cgroup v2. Sale 0 si la medición se
#       completó, distinto de 0 con una línea `FALLA: ...` por cada motivo.
#       NO aprueba ni reprueba los valores medidos: no hay umbral, es un instrumento.
#
# DEPENDENCIAS
#
#   - bash, sed, mktemp, date, find, cat        (POSIX/Util-linux estándar)
#   - docker + docker compose                    (CLI v5.x verificado)
#   - deploy/celula.env.ejemplo como referente de variables
#   - imagen alpine:3 (contenedor auxiliar efímero de carga; ninguna herramienta nueva)
#   - cgroup v2 (cgroup2fs) activo en el anfitrión
#
# Un entorno sin docker/compose, sin cgroup v2 o sin las imágenes NO se
# declara medido: el script falla con un mensaje explícito.
# ============================================================================

set -u

PLANTILLA="${1:-deploy/cell.compose.yml}"

# --- Precondiciones ----------------------------------------------------------

if [ ! -f "$PLANTILLA" ]; then
    echo "FALLA: la plantilla [$PLANTILLA] no existe" >&2
    exit 1
fi

if ! command -v docker >/dev/null 2>&1 || ! docker compose version >/dev/null 2>&1; then
    echo "FALLA: docker compose no está disponible en este entorno; la medición en vivo no se puede hacer" >&2
    exit 1
fi

if [ ! -f deploy/celula.env.ejemplo ]; then
    echo "FALLA: deploy/celula.env.ejemplo no existe; no hay referente de variables para levantar la célula" >&2
    exit 1
fi

if [ "$(stat -fc %T /sys/fs/cgroup 2>/dev/null)" != "cgroup2fs" ]; then
    echo "FALLA: el anfitrión no expone cgroup v2 (cgroup2fs) en /sys/fs/cgroup; la lectura de memory.current/memory.stat es imposible" >&2
    exit 1
fi

# Referente de variables: se leen del ejemplo, igual que hace
# deploy/verificar_limites.sh con los límites. Son los valores REALES bajo
# los que corre la célula de esta corrida, y el informe los vuelve a imprimir
# para que una cifra registrada nunca quede huérfana del techo contra el que
# se midió.
LIMITE_MEMORIA_NUCLEO="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_MEMORIA=//p' deploy/celula.env.ejemplo)"
LIMITE_MEMORIA_SIDECAR="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_MEMORIA=//p' deploy/celula.env.ejemplo)"
LIMITE_CPUS_NUCLEO="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_CPUS=//p' deploy/celula.env.ejemplo)"
LIMITE_CPUS_SIDECAR="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_CPUS=//p' deploy/celula.env.ejemplo)"
LIMITE_NOFILE_NUCLEO="$(sed -n 's/^HEXCELL_NUCLEO_LIMITE_NOFILE=//p' deploy/celula.env.ejemplo)"
LIMITE_NOFILE_SIDECAR="$(sed -n 's/^HEXCELL_SIDECAR_LIMITE_NOFILE=//p' deploy/celula.env.ejemplo)"
IMAGEN_NUCLEO="$(sed -n 's/^HEXCELL_IMAGEN_NUCLEO=//p' deploy/celula.env.ejemplo)"
IMAGEN_SIDECAR="$(sed -n 's/^HEXCELL_IMAGEN_SIDECAR=//p' deploy/celula.env.ejemplo)"

if [ -z "$LIMITE_MEMORIA_NUCLEO" ] || [ -z "$LIMITE_MEMORIA_SIDECAR" ] \
    || [ -z "$LIMITE_CPUS_NUCLEO" ] || [ -z "$LIMITE_CPUS_SIDECAR" ] \
    || [ -z "$LIMITE_NOFILE_NUCLEO" ] || [ -z "$LIMITE_NOFILE_SIDECAR" ] \
    || [ -z "$IMAGEN_NUCLEO" ] || [ -z "$IMAGEN_SIDECAR" ]; then
    echo "FALLA: no se pudieron leer los límites y las imágenes del referente deploy/celula.env.ejemplo" >&2
    exit 1
fi

# Ambas imágenes deben estar construidas: la medición de tamaño (AC-4) mide la
# imagen REAL, y una imagen ausente no reporta un tamaño vacío ni cero.
if ! docker image inspect "$IMAGEN_NUCLEO" >/dev/null 2>&1; then
    echo "FALLA: la imagen [$IMAGEN_NUCLEO] no está construida; constrúyela antes de medir (docker compose build)" >&2
    exit 1
fi
if ! docker image inspect "$IMAGEN_SIDECAR" >/dev/null 2>&1; then
    echo "FALLA: la imagen [$IMAGEN_SIDECAR] no está construida; constrúyela antes de medir (docker compose build)" >&2
    exit 1
fi

# --- Nombres de un solo uso --------------------------------------------------

SUFIJO="$(date +%s)-$$"
ID_CELULA="hex079medir${SUFIJO}"
RED="hex079-medir-red-${SUFIJO}"
VOLUMEN="hex079-medir-vol-${SUFIJO}"
PROYECTO="hex079medir${SUFIJO}"
CONTENEDOR_NUCLEO="${ID_CELULA}-nucleo"
CONTENEDOR_SIDECAR="${ID_CELULA}-sidecar"
AUXILIAR_CARGA="${ID_CELULA}-carga"
ENV_TEMP="$(mktemp -t hex079-env.XXXXXX)"

# Duración de la ráfaga de carga en segundos: acotada y fija, nunca un bucle
# sin límite. El contenedor auxiliar corre unos segundos más que la ventana de
# muestreo para que el pico se tome con el generador aún vivo.
DURACION_CARGA=20

limpiar() {
    echo ""
    echo "Limpiando: contenedores, red, volumen de un solo uso y auxiliar de carga..."
    docker rm -f "$AUXILIAR_CARGA" >/dev/null 2>&1 || true
    docker compose -p "$PROYECTO" --env-file "$ENV_TEMP" -f "$PLANTILLA" down --volumes --remove-orphans >/dev/null 2>&1 || true
    docker volume rm "$VOLUMEN" >/dev/null 2>&1 || true
    docker network rm "$RED" >/dev/null 2>&1 || true
    rm -f "$ENV_TEMP"
}
trap limpiar EXIT

# Copia deploy/celula.env.ejemplo y sustituye solo el identificador, la red y
# el volumen por nombres únicos de esta corrida: la célula arranca en frío
# desde un volumen VACÍO cada vez, jamás reutilizado ni presembrado — un
# volumen con datos previos no probaría nada.
sed -E \
    -e "s/^HEXCELL_ID_CELULA=.*/HEXCELL_ID_CELULA=${ID_CELULA}/" \
    -e "s/^HEXCELL_RED_CELULA=.*/HEXCELL_RED_CELULA=${RED}/" \
    -e "s/^HEXCELL_VOLUMEN_CELULA=.*/HEXCELL_VOLUMEN_CELULA=${VOLUMEN}/" \
    deploy/celula.env.ejemplo >"$ENV_TEMP"

echo "Célula de medición: ${ID_CELULA} (red ${RED}, volumen ${VOLUMEN})"
echo ""
echo "=== Arranque en frío desde volumen vacío, sidecar sin emparejar ==="

if ! docker compose -p "$PROYECTO" --env-file "$ENV_TEMP" -f "$PLANTILLA" up -d --build; then
    echo "FALLA: docker compose up no pudo levantar la célula"
    exit 1
fi

# --- Señal de vida: línea de arranque conocida, NO /health/ready -----------
# Mismo criterio que deploy/verificar_aislamiento.sh y
# deploy/verificar_apagado_ordenado.sh: una célula deliberadamente sin
# emparejar nunca alcanza el estado "sesión de canal activa", así que esperar
# un 200 de /health/ready colgaría el script o lo haría fallar por una razón
# ajena a esta medición. La espera es ACOTADA (30 intentos de 1 s), no un
# sueño sin límite.

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
    exit 1
fi

if esperar_arranque "$CONTENEDOR_SIDECAR" '"evento":"sidecar.arrancado"'; then
    echo "OK: el sidecar señaló arranque (sidecar.arrancado)"
else
    echo "FALLA: el sidecar no señaló arranque en 30 s"
    exit 1
fi

# --- Localizar los directorios cgroup v2 de ambos contenedores --------------

# localizar_cgroup_v2 <contenedor> -> imprime el directorio cgroup v2
#   Se resuelve el id COMPLETO del contenedor con docker inspect y se busca
#   ese id bajo /sys/fs/cgroup, de modo que funcionen tanto el driver systemd
#   (system.slice/docker-<id>.scope) como el driver cgroupfs (docker/<id>) y
#   los anfitriones rootless (rebanadas de usuario). Ninguna ruta fija cubre
#   las tres; la búsqueda por el id completo, sí. Si no aparece, FALLA: nunca
#   se cae a una cifra del /proc del anfitrión ni a `docker stats`.
localizar_cgroup_v2() {
    local contenedor="$1"
    local id_completo directorio
    id_completo="$(docker inspect -f '{{.Id}}' "$contenedor" 2>/dev/null)"
    if [ -z "$id_completo" ]; then
        echo "FALLA: no se pudo obtener el id completo de [$contenedor] vía docker inspect"
        return 1
    fi
    directorio="$(find /sys/fs/cgroup -maxdepth 8 -type d \( -name "docker-${id_completo}.scope" -o -name "${id_completo}" \) 2>/dev/null | head -n 1)"
    if [ -z "$directorio" ]; then
        echo "FALLA: no se localizó el directorio cgroup v2 del contenedor [$contenedor] (id ${id_completo}) bajo /sys/fs/cgroup"
        return 1
    fi
    printf '%s' "$directorio"
}

DIR_CGROUP_NUCLEO="$(localizar_cgroup_v2 "$CONTENEDOR_NUCLEO")" || exit 1
DIR_CGROUP_SIDECAR="$(localizar_cgroup_v2 "$CONTENEDOR_SIDECAR")" || exit 1
echo "OK: cgroup v2 del núcleo en ${DIR_CGROUP_NUCLEO}"
echo "OK: cgroup v2 del sidecar en ${DIR_CGROUP_SIDECAR}"

# --- Lectura de memoria desde cgroup v2 -------------------------------------

# leer_memoria_cgroup <directorio-cgroup> -> imprime "actual anon"
#   Lee memory.current (carga total en bytes) y la clave `anon` de memory.stat
#   (equivalente a RSS). Si el archivo no existe, está vacío o no es numérico,
#   FALLA: nunca se imprime 0 ni una cifra vacía como si fuera una medición —
#   un cero que podría significar "no medido" es un resultado vacuo.
leer_memoria_cgroup() {
    local directorio="$1"
    local actual anon
    actual="$(cat "${directorio}/memory.current" 2>/dev/null)"
    anon="$(sed -n 's/^anon[[:space:]]//p' "${directorio}/memory.stat" 2>/dev/null)"
    case "$actual" in
        ''|*[!0-9]*)
            echo "FALLA: memory.current en [$directorio] no es un entero no negativo ('${actual}')"
            return 1
            ;;
    esac
    case "$anon" in
        ''|*[!0-9]*)
            echo "FALLA: la clave anon de memory.stat en [$directorio] no es un entero no negativo ('${anon}')"
            return 1
            ;;
    esac
    printf '%s %s\n' "$actual" "$anon"
}

# muestrear_agregado -> imprime "actual_agregado anon_agregado"
#   Suma la lectura de ambos contenedores: el agregado de la célula COMPUESTA,
#   no de un contenedor suelto.
muestrear_agregado() {
    local lectura_nucleo lectura_sidecar actual anon
    lectura_nucleo="$(leer_memoria_cgroup "$DIR_CGROUP_NUCLEO")" || return 1
    lectura_sidecar="$(leer_memoria_cgroup "$DIR_CGROUP_SIDECAR")" || return 1
    set -- $lectura_nucleo
    actual=$(( $1 ))
    anon=$(( $2 ))
    set -- $lectura_sidecar
    actual=$(( actual + $1 ))
    anon=$(( anon + $2 ))
    printf '%s %s\n' "$actual" "$anon"
}

# --- Medición en reposo ------------------------------------------------------
# Se toma después de que ambos contenedores señalaron arranque y con una pausa
# corta y acotada para que las páginas del asignador se asienten (mismo
# criterio que rss_linea_base.rs, que espera 500 ms antes de leer la memoria
# residente; aquí la señal de arranque ya es la espera acotada y la pausa solo
# deja asentar el asignador).
echo ""
echo "=== Asentamiento del asignador (3 s) antes de la medición en reposo ==="
sleep 3

echo "=== Medición en reposo ==="
lectura_reposo="$(muestrear_agregado)" || exit 1
set -- $lectura_reposo
REPOSO_ACTUAL=$1
REPOSO_ANON=$2
echo "OK: en reposo — anon agregado=${REPOSO_ANON} B, memory.current agregado=${REPOSO_ACTUAL} B"

# --- Carga sustituta y medición bajo carga -----------------------------------

echo ""
echo "=== Generador de carga sustituto (${DURACION_CARGA} s, ráfaga paralela contra el listener de salud) ==="
echo "(ver LIMITACION en la cabecera: la cifra bajo carga es una COTA INFERIOR, no el peor caso)"

# Contenedor auxiliar efímero alpine:3 unido al espacio de nombres de red del
# NÚCLEO: ve el listener de salud como 127.0.0.1:8081. Ráfaga ACOTADA de GETs
# paralelos durante la duración fija; el estado HTTP (200/503) se ignora a
# propósito: una célula fría y sin emparejar NO está lista y devolver 503 es el
# estado ESPERADO, no un defecto — gatear sobre 200 haría fallar el script por
# el estado esperado. El código 125 de docker run o un mensaje del daemon
# señalan que el auxiliar nunca llegó a correr: entonces la cifra bajo carga
# no existe y se falla, no se fabrica.
docker run --rm --name "$AUXILIAR_CARGA" \
    -e DURACION_CARGA="$(( DURACION_CARGA + 10 ))" \
    --network "container:${CONTENEDOR_NUCLEO}" \
    alpine:3 sh -c '
        fin=$(( $(date +%s) + DURACION_CARGA ))
        while [ "$(date +%s)" -lt "$fin" ]; do
            wget -q -O /dev/null http://127.0.0.1:8081/health/ready || true &
            wget -q -O /dev/null http://127.0.0.1:8081/health/live || true &
            sleep 0.2
        done
        wait
    ' &
PID_GENERADOR=$!

# El auxiliar debe estar vivo 1 s después de lanzarlo; si no, el generador
# falló en el arranque (p. ej. imagen alpine:3 no disponible) y la medición
# bajo carga sería vacua.
sleep 1
if [ "$(docker inspect -f '{{.State.Running}}' "$AUXILIAR_CARGA" 2>/dev/null)" != "true" ]; then
    echo "FALLA: el contenedor auxiliar de carga no está corriendo; la cifra bajo carga no se puede medir"
    exit 1
fi

# Muestreo cada segundo durante la ventana fija, conservando el PICO de cada
# agregado: nunca una lectura instantánea única, y nunca la suma de picos
# individuales (sobrestimaría); el pico se toma de la SUMA por muestra.
PICO_ACTUAL=0
PICO_ANON=0
MUESTRAS=0
while [ "$MUESTRAS" -lt "$DURACION_CARGA" ]; do
    sleep 1
    MUESTRAS=$((MUESTRAS + 1))
    lectura="$(muestrear_agregado)" || exit 1
    set -- $lectura
    if [ "$1" -gt "$PICO_ACTUAL" ]; then
        PICO_ACTUAL=$1
    fi
    if [ "$2" -gt "$PICO_ANON" ]; then
        PICO_ANON=$2
    fi
done

wait "$PID_GENERADOR"
CODIGO_GENERADOR=$?
if [ "$CODIGO_GENERADOR" -eq 125 ]; then
    echo "FALLA: docker run del generador de carga terminó con código 125; la cifra bajo carga no es una medición"
    exit 1
fi
CARGA_ACTUAL="$PICO_ACTUAL"
CARGA_ANON="$PICO_ANON"
echo "OK: bajo carga — pico anon agregado=${CARGA_ANON} B, pico memory.current agregado=${CARGA_ACTUAL} B (${MUESTRAS} muestras)"

# --- Tamaño de las imágenes (AC-4) -------------------------------------------

# medir_tamano_imagen <imagen> -> imprime el tamaño en bytes
#   docker image inspect, nunca docker stats. Una imagen ausente o un tamaño
#   vacío/no numérico es FALLA: no se reporta un cero ni una cifra vacía.
medir_tamano_imagen() {
    local imagen="$1"
    local tamano
    tamano="$(docker image inspect --format '{{.Size}}' "$imagen" 2>/dev/null)"
    case "$tamano" in
        ''|*[!0-9]*)
            echo "FALLA: no se pudo leer el tamaño de la imagen [$imagen] con docker image inspect"
            return 1
            ;;
    esac
    if [ "$tamano" -eq 0 ]; then
        echo "FALLA: la imagen [$imagen] reporta tamaño 0; eso no es una medición"
        return 1
    fi
    printf '%s' "$tamano"
}

TAMANO_NUCLEO="$(medir_tamano_imagen "$IMAGEN_NUCLEO")" || exit 1
TAMANO_SIDECAR="$(medir_tamano_imagen "$IMAGEN_SIDECAR")" || exit 1

# --- Informe consolidado ------------------------------------------------------

informar() {
    echo ""
    echo "=== Informe de la medición (tarea 16 A-6, HEX-079) ==="
    echo ""
    echo "Fecha: $(date -u +%Y-%m-%d)  Commit: $(git rev-parse --short HEAD 2>/dev/null || echo 'desconocido')  Host: $(hostname 2>/dev/null || echo 'desconocido')"
    echo ""
    echo "Límites de cgroup bajo los que se midió (referente deploy/celula.env.ejemplo):"
    echo "  HEXCELL_NUCLEO_LIMITE_MEMORIA=${LIMITE_MEMORIA_NUCLEO}   HEXCELL_SIDECAR_LIMITE_MEMORIA=${LIMITE_MEMORIA_SIDECAR}"
    echo "  HEXCELL_NUCLEO_LIMITE_CPUS=${LIMITE_CPUS_NUCLEO}   HEXCELL_SIDECAR_LIMITE_CPUS=${LIMITE_CPUS_SIDECAR}"
    echo "  HEXCELL_NUCLEO_LIMITE_NOFILE=${LIMITE_NOFILE_NUCLEO}   HEXCELL_SIDECAR_LIMITE_NOFILE=${LIMITE_NOFILE_SIDECAR}"
    echo ""
    echo "Memoria en reposo (agregado núcleo+sidecar, cgroup v2):"
    echo "  anon (equivalente a RSS): ${REPOSO_ANON} B  ( $(( REPOSO_ANON / 1048576 )) MiB )"
    echo "  memory.current (incluye caché de página): ${REPOSO_ACTUAL} B  ( $(( REPOSO_ACTUAL / 1048576 )) MiB )"
    echo ""
    echo "Memoria bajo carga (pico de ${MUESTRAS} muestras, agregado núcleo+sidecar, cgroup v2):"
    echo "  anon (equivalente a RSS): ${CARGA_ANON} B  ( $(( CARGA_ANON / 1048576 )) MiB )"
    echo "  memory.current (incluye caché de página): ${CARGA_ACTUAL} B  ( $(( CARGA_ACTUAL / 1048576 )) MiB )"
    echo ""
    echo "Tamaño de las imágenes (docker image inspect):"
    echo "  ${IMAGEN_NUCLEO}: ${TAMANO_NUCLEO} B  ( $(( TAMANO_NUCLEO / 1048576 )) MiB )"
    echo "  ${IMAGEN_SIDECAR}: ${TAMANO_SIDECAR} B  ( $(( TAMANO_SIDECAR / 1048576 )) MiB )"
    echo ""
    echo "LIMITACION: la cifra bajo carga es una COTA INFERIOR, no el peor caso."
    echo "  crates/hexcell/tests/carga.rs no se reutilizó como generador externo (cuatro razones"
    echo "  en la cabecera) y no se modificó. El generador sustituto golpea /health/ready y"
    echo "  /health/live: ejercita el listener HTTP, el runtime tokio y el asignador, pero NO la"
    echo "  admisión GCRA, NO el pipeline de inferencia, NO el motor de conocimiento y NO la ruta"
    echo "  whatsmeow del sidecar. El único camino de entrada genuino es un mensaje real de"
    echo "  WhatsApp sobre el websocket saliente, que requiere un dispositivo emparejado."
    echo ""
    echo "Transcribir estos valores a la tabla de docs/plantilla-celula.md (valores de referencia)."
}

informar

echo ""
echo "OK: medición completada — transcribir los valores a docs/plantilla-celula.md"
exit 0