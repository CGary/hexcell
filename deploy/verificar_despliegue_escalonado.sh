#!/usr/bin/env bash
# ============================================================================
# Guardia estático del despliegue escalonado de whatsmeow (HEX-089, tarea 19 A-6)
# Comprueba que ninguna vía —ni la CI ni la CLI— actualiza toda la cartera en
# un solo paso (criterio de aceptación de la tarea 19 de la etapa A-6: «Una
# actualización de whatsmeow no llega a ninguna célula de cliente…»).
# CUATRO COMPROBACIONES (ids fijos; FALLA[<id>]/OK[<id>]):
#   CI-COMPOSE     — ningún trabajo distinto de `imagenes` (igualdad de clave,
#                    nunca patrón; construye con push:false) ejecuta línea
#                    lógica con `docker`+`compose` (o `docker-compose`) y un
#                    token posterior `up`/`pull`/`restart`.
#   CI-SCRIPT      — los deploy/*.sh nombrados fuera de `imagenes` (con `bash`,
#                    `sh`, `./` o sin prefijo) no se llaman con tallo `deploy`/
#                    `desplegar`/`update`/`actualizar` ni contienen CI-COMPOSE.
#   CLI-SUBCOMANDO — argumentos.rs sin literales `update`/`deploy`/`actualizar`/
#                    `desplegar` en líneas con `=>` o `==`.
#   CLI-USO        — TEXTO_DE_USO: sin esas palabras al inicio de línea ni tras
#                    `hexcell-admin <grupo>`.
# POR QUÉ IGUALDAD EXACTA DE TOKENS y no grep de contención: la contención
# marca `docker compose config` o `update_estado`; la igualdad exacta sobre
# líneas lógicas tokenizadas (continuaciones con `\`, sin comentarios, partidas
# por espacios/comillas/`;`/`&`/`|`, en minúsculas) distingue `up` de `config`.
# POR QUÉ COPIA Y MUTACIÓN EN --autoprueba: un guardia que nunca se vio fallar
# no es un guardia; --autoprueba muta COPias en mktemp, prueba con `cmp` que
# cada mutación cambió su copia y exige rojo con el id EXACTO y sin otros.
# USO: <script> [<dir-workflows> <ruta-argumentos.rs>] | <script> --autoprueba
# DEPENDENCIAS: bash, grep, awk, sed, cmp, mktemp, cp, rm (ubuntu-latest).
# Se escanean los flujos *.yml y *.yaml.
# PUNTOS CIEGOS (estático por líneas, no parser de YAML): verbos por variable,
# `uses:` de terceros, scripts indirectos y escalares plegados (`run: >`) que
# parten `docker compose` de su verbo en líneas físicas distintas (solo se unen
# las continuaciones con `\`); aceptados sin toolchain.
# ============================================================================

set -u

# Palabras vigiladas en variables SEPARADAS: la regla dispara cuando una línea
# lógica reúne `docker`+`compose` (o `docker-compose`) con un verbo después.
# CI-SCRIPT escanea este mismo script (el trabajo guardas-despliegue lo invoca),
# así que ninguna línea no comentada puede reunir esas palabras; toda mención
# se compone en tiempo de ejecución.
TOKEN_DOCKER="docker"
TOKEN_COMPOSE="compose"
TOKEN_DOCKER_COMPOSE="docker-compose"
VERBO_SUBIR="up"
VERBO_BAJAR="pull"
VERBO_REINICIAR="restart"
PALABRA_UPDATE="update"
PALABRA_DEPLOY="deploy"
PALABRA_ACTUALIZAR="actualizar"
PALABRA_DESPLEGAR="desplegar"

# tokenizar_linea <linea>: minúsculas, parte por espacios, comillas, `;`, `&`
# y `|`; deja el resultado en TOKENS. Igualdad SIEMPRE exacta, nunca contención.
declare -a TOKENS=()
tokenizar_linea() {
    local linea="${1,,}"
    TOKENS=()
    [ -z "$linea" ] && return 0
    local ifs_guardado="$IFS"
    IFS=$' \t\n"'"'"';&|'
    read -r -a TOKENS <<< "$linea"
    IFS="$ifs_guardado"
}

es_palabra_de_comando() {
    local t="$1"
    [ "$t" = "$PALABRA_UPDATE" ] || [ "$t" = "$PALABRA_DEPLOY" ] \
        || [ "$t" = "$PALABRA_ACTUALIZAR" ] || [ "$t" = "$PALABRA_DESPLEGAR" ]
}

# lineas_logicas <archivo> [si]: "JOB\037NUM\037LINEA" por línea lógica no
# comentada (une `\`; JOB="" sin trabajos). Con "si": detecta claves
# `  <clave>:` tras `jobs:`; los nombres `name:` los filtra el llamante.
# El separador \037 (no es espacio) evita que `read` colapse el campo JOB vacío.
lineas_logicas() {
    local archivo="$1" con_trabajos="${2:-}"
    awk -v con_trabajos="$con_trabajos" '
        BEGIN { en_jobs = 0; job = ""; acum = "" }
        { linea = $0
          if (linea ~ /^[[:space:]]*#/) { if (acum != "") print job "\037" ini "\037" acum; acum = ""; next }
          if (con_trabajos == "si" && linea == "jobs:") { en_jobs = 1; next }
          if (con_trabajos == "si" && en_jobs && linea ~ /^  [A-Za-z0-9_-]+:[[:space:]]*$/) {
              if (acum != "") print job "\037" ini "\037" acum; acum = ""
              job = linea; sub(/^  /, "", job); sub(/:.*$/, "", job); next
          }
          if (linea ~ /\\[[:space:]]*$/) { if (acum == "") ini = NR; sub(/\\[[:space:]]*$/, "", linea); acum = acum " " linea; next }
          if (acum != "") { print job "\037" ini "\037" acum " " linea; acum = "" }
          else print job "\037" NR "\037" linea
        }
        END { if (acum != "") print job "\037" ini "\037" acum }
    ' "$archivo"
}

# comprobar_regla_compose <archivo> <num> <linea> <id>: FALLA[<id>] si reúne
# docker+compose (o docker-compose) con up/pull/restart después.
comprobar_regla_compose() {
    local archivo="$1" num="$2" linea="$3" id="$4"
    tokenizar_linea "$linea"
    local i n
    n=${#TOKENS[@]}
    for (( i = 0; i < n; i++ )); do
        local t="${TOKENS[$i]}" inicio=0 es_compose=0
        if [ "$t" = "$TOKEN_DOCKER" ] && [ $((i + 1)) -lt "$n" ] && [ "${TOKENS[$((i + 1))]}" = "$TOKEN_COMPOSE" ]; then
            es_compose=1
            inicio=$((i + 2))
        elif [ "$t" = "$TOKEN_DOCKER_COMPOSE" ]; then
            es_compose=1
            inicio=$((i + 1))
        fi
        if [ "$es_compose" -eq 1 ]; then
            local j v
            for (( j = inicio; j < n; j++ )); do
                v="${TOKENS[$j]}"
                if [ "$v" = "$VERBO_SUBIR" ] || [ "$v" = "$VERBO_BAJAR" ] || [ "$v" = "$VERBO_REINICIAR" ]; then
                    echo "FALLA[$id]: $archivo:$num: línea lógica con ${TOKEN_DOCKER} ${TOKEN_COMPOSE} (o ${TOKEN_DOCKER_COMPOSE}) y el verbo «$v»"
                    return 1
                fi
            done
        fi
    done
    return 0
}

# listar_flujos <dir>: deja en FLUJOS los *.yml y *.yaml existentes (GitHub
# Actions carga ambas extensiones).
declare -a FLUJOS=()
listar_flujos() {
    local f
    FLUJOS=()
    for f in "$1"/*.yml "$1"/*.yaml; do
        [ -e "$f" ] && FLUJOS+=("$f")
    done
}

verificar_flujos_de_trabajo() {
    local dir_workflows="$1"
    listar_flujos "$dir_workflows"
    if [ "${#FLUJOS[@]}" -eq 0 ]; then
        echo "FALLA[CI-COMPOSE]: no hay flujos de trabajo (*.yml, *.yaml) en $dir_workflows"
        return 1
    fi
    local fallas=0 archivo job num linea
    for archivo in "${FLUJOS[@]}"; do
        while IFS=$'\037' read -r job num linea; do
            [ "$job" = "imagenes" ] && continue
            [[ "$linea" =~ ^[[:space:]]*(-[[:space:]]*)?name: ]] && continue
            if ! comprobar_regla_compose "$archivo" "$num" "$linea" "CI-COMPOSE"; then
                fallas=$((fallas + 1))
            fi
        done < <(lineas_logicas "$archivo" "si")
    done
    [ "$fallas" -eq 0 ]
}

# extraer_referencias <dir-workflows>: deploy/*.sh nombrados por cualquier token
# `deploy/<x>.sh` o `./deploy/<x>.sh` (da igual el token previo: bash, sh, -x,
# ninguno) fuera de `imagenes`, sin `./` y sin duplicados.
declare -a SCRIPTS_EXTRAIDOS=()
extraer_referencias() {
    local dir_workflows="$1"
    SCRIPTS_EXTRAIDOS=()
    local archivo job num linea i n t
    listar_flujos "$dir_workflows"
    for archivo in "${FLUJOS[@]}"; do
        while IFS=$'\037' read -r job num linea; do
            [ "$job" = "imagenes" ] && continue
            [[ "$linea" =~ ^[[:space:]]*(-[[:space:]]*)?name: ]] && continue
            tokenizar_linea "$linea"
            n=${#TOKENS[@]}
            for (( i = 0; i < n; i++ )); do
                t="${TOKENS[$i]#./}"
                if [[ "$t" == deploy/*.sh ]]; then
                    SCRIPTS_EXTRAIDOS+=("$t")
                fi
            done
        done < <(lineas_logicas "$archivo" "si")
    done
    local -a unicos=()
    local s u ya
    for s in "${SCRIPTS_EXTRAIDOS[@]}"; do
        ya=0
        for u in "${unicos[@]}"; do
            if [ "$u" = "$s" ]; then ya=1; break; fi
        done
        [ "$ya" -eq 0 ] && unicos+=("$s")
    done
    SCRIPTS_EXTRAIDOS=("${unicos[@]}")
}

verificar_scripts_invocados() {
    local dir_workflows="$1"
    local fallas=0 raiz ref ruta tallo job num linea
    extraer_referencias "$dir_workflows"
    raiz="$(dirname "$(dirname "$dir_workflows")")"
    for ref in "${SCRIPTS_EXTRAIDOS[@]}"; do
        tallo="$(basename "$ref" .sh)"
        if [ "$tallo" = "$PALABRA_DEPLOY" ] || [ "$tallo" = "$PALABRA_DESPLEGAR" ] \
            || [ "$tallo" = "$PALABRA_UPDATE" ] || [ "$tallo" = "$PALABRA_ACTUALIZAR" ]; then
            echo "FALLA[CI-SCRIPT]: «$ref» se llama con una palabra de despliegue"
            fallas=$((fallas + 1))
            continue
        fi
        ruta="$raiz/$ref"
        if [ ! -f "$ruta" ]; then
            echo "FALLA[CI-SCRIPT]: «$ref» no existe bajo $raiz"
            fallas=$((fallas + 1))
            continue
        fi
        while IFS=$'\037' read -r job num linea; do
            if ! comprobar_regla_compose "$ruta" "$num" "$linea" "CI-SCRIPT"; then
                fallas=$((fallas + 1))
            fi
        done < <(lineas_logicas "$ruta")
    done
    [ "$fallas" -eq 0 ]
}

# verificar_cli <ruta-argumentos.rs>: CLI-SUBCOMANDO y CLI-USO en una pasada.
verificar_cli() {
    local ruta="$1"
    if [ ! -f "$ruta" ]; then
        echo "FALLA[CLI-SUBCOMANDO]: no existe el archivo $ruta"
        echo "FALLA[CLI-USO]: no existe el archivo $ruta"
        return 1
    fi
    local fallas_sub=0 fallas_uso=0 num=0 linea t lleva i n grupo despues
    local en_bloque=0 visto_inicio=0 cierra patron_cierre='";[[:space:]]*$'
    while IFS= read -r linea || [ -n "$linea" ]; do
        num=$((num + 1))
        [[ "$linea" =~ ^[[:space:]]*(#|//) ]] && continue
        tokenizar_linea "$linea"
        lleva=0
        for t in "${TOKENS[@]}"; do
            if [ "$t" = "=>" ] || [ "$t" = "==" ]; then lleva=1; break; fi
        done
        if [ "$lleva" -eq 1 ]; then
            for t in "${TOKENS[@]}"; do
                if es_palabra_de_comando "$t"; then
                    echo "FALLA[CLI-SUBCOMANDO]: $ruta:$num: «$t» como literal en línea con => o =="
                    fallas_sub=$((fallas_sub + 1))
                fi
            done
        fi
        if [ "$en_bloque" -eq 0 ]; then
            if [[ "$linea" =~ pub[[:space:]]+const[[:space:]]+TEXTO_DE_USO ]]; then
                en_bloque=1
                visto_inicio=1
            fi
            continue
        fi
        # TEXTO_DE_USO: primer token y token tras `hexcell-admin <grupo>` (y el
        # grupo mismo) sin palabras de despliegue.
        n=${#TOKENS[@]}
        if [ "$n" -gt 0 ] && es_palabra_de_comando "${TOKENS[0]}"; then
            echo "FALLA[CLI-USO]: $ruta:$num: la línea del texto de uso comienza con «${TOKENS[0]}»"
            fallas_uso=$((fallas_uso + 1))
        fi
        for (( i = 0; i < n; i++ )); do
            if [ "${TOKENS[$i]}" = "hexcell-admin" ] && [ $((i + 2)) -lt "$n" ]; then
                grupo="${TOKENS[$((i + 1))]}"
                despues="${TOKENS[$((i + 2))]}"
                if es_palabra_de_comando "$grupo" || es_palabra_de_comando "$despues"; then
                    echo "FALLA[CLI-USO]: $ruta:$num: «hexcell-admin» va seguido de «$grupo»/«$despues»"
                    fallas_uso=$((fallas_uso + 1))
                fi
            fi
        done
        cierra=0
        [[ "$linea" =~ $patron_cierre ]] && cierra=1
        [ "$cierra" -eq 1 ] && en_bloque=0
    done < "$ruta"
    if [ "$visto_inicio" -eq 0 ]; then
        echo "FALLA[CLI-USO]: no se encontró la constante TEXTO_DE_USO en $ruta"
        fallas_uso=$((fallas_uso + 1))
    fi
    [ "$fallas_sub" -eq 0 ] && echo "OK[CLI-SUBCOMANDO]"
    [ "$fallas_uso" -eq 0 ] && echo "OK[CLI-USO]"
    [ "$fallas_sub" -eq 0 ] && [ "$fallas_uso" -eq 0 ]
}

verificar_todo() {
    local dir_workflows="$1" ruta_argumentos="$2" fallas=0
    verificar_flujos_de_trabajo "$dir_workflows" && echo "OK[CI-COMPOSE]" || fallas=$((fallas + 1))
    verificar_scripts_invocados "$dir_workflows" && echo "OK[CI-SCRIPT]" || fallas=$((fallas + 1))
    verificar_cli "$ruta_argumentos" || fallas=$((fallas + 1))
    [ "$fallas" -eq 0 ]
}

MODO_AUTOPRUEBA=0
if [ "${1:-}" = "--autoprueba" ]; then
    MODO_AUTOPRUEBA=1
fi
if [ "$MODO_AUTOPRUEBA" -eq 0 ]; then
    DIR_WORKFLOWS="${1:-.github/workflows}"
    RUTA_ARGUMENTOS="${2:-crates/hexcell-admin/src/argumentos.rs}"
    if verificar_todo "$DIR_WORKFLOWS" "$RUTA_ARGUMENTOS"; then
        echo "OK: no existe ninguna vía —ni CI ni CLI— que actualice toda la cartera en un solo paso"
        exit 0
    fi
    echo "FALLA: el guardia de despliegue escalonado encontró violaciones" >&2
    exit 1
fi

# --- Modo --autoprueba (prueba de mutación) -----------------------------------
# Muta COPias en mktemp, prueba con `cmp` que cada mutación cambió su copia y
# exige rojo con FALLA[<id_esperado>] y sin ningún otro id (el exit code nunca
# basta). Leer las líneas PASA/FALLA, no solo el exit code.
DIR_TEMP=""
DIR_TEMP="$(mktemp -d -t hex089-guard.XXXXXX)"
trap "rm -rf '$DIR_TEMP'" EXIT
ORIGINAL_WORKFLOW=".github/workflows/ci.yml"
ORIGINAL_ARGUMENTOS="crates/hexcell-admin/src/argumentos.rs"
BASE="$DIR_TEMP/base"
mkdir -p "$BASE/.github/workflows" "$BASE/deploy"
cp "$ORIGINAL_WORKFLOW" "$BASE/.github/workflows/ci.yml"
cp "$ORIGINAL_ARGUMENTOS" "$BASE/argumentos.rs"
extraer_referencias "$BASE/.github/workflows"
for ref in "${SCRIPTS_EXTRAIDOS[@]}"; do
    [ -f "deploy/$(basename "$ref")" ] && cp "deploy/$(basename "$ref")" "$BASE/deploy/"
done

preparar_caso() {
    local nombre="$1"
    local destino="$DIR_TEMP/$nombre"
    mkdir -p "$destino"
    cp -r "$BASE/." "$destino/"
    printf '%s' "$destino"
}

# insertar_paso_en_trabajo <archivo> <job> <bloque>: inserta <bloque> (paso
# YAML) al final del trabajo <job>; si no existe, no inserta nada — `cmp` lo
# detecta.
insertar_paso_en_trabajo() {
    local archivo="$1" job="$2" bloque="$3" tmp
    tmp="$(mktemp -t hex089-insercion.XXXXXX)"
    awk -v job="$job" -v bloque="$bloque" '
        /^  [A-Za-z0-9_-]+:[[:space:]]*$/ {
            if (en_job && !insertado) { print bloque; insertado = 1 }
            en_job = ($0 == "  " job ":")
        }
        { print }
        END { if (en_job && !insertado) print bloque }
    ' "$archivo" > "$tmp"
    mv "$tmp" "$archivo"
}

# insertar_antes_de <archivo> <patron-awk> <linea>: inserta <linea> antes de
# la primera línea que casa con <patron-awk>.
insertar_antes_de() {
    local archivo="$1" patron="$2" linea="$3" tmp
    tmp="$(mktemp -t hex089-insercion.XXXXXX)"
    awk -v patron="$patron" -v linea="$linea" '
        $0 ~ patron { print linea }
        { print }
    ' "$archivo" > "$tmp"
    mv "$tmp" "$archivo"
}

# Cargas de mutación. Se arman con fragmentos LIT_* INDEPENDIENTES de las
# constantes de detección (TOKEN_*, VERBO_*, PALABRA_*): si una constante se
# corrompe, la carga sigue siendo la real y la autoprueba se pone roja. Además,
# ninguna línea no comentada de este script reúne las palabras juntas (CI-SCRIPT
# lo escanea), así que no necesita autoexención.
LIT_DOCKER="do""cker"
LIT_COMPOSE="com""pose"
LIT_DOCKER_COMPOSE="${LIT_DOCKER}-${LIT_COMPOSE}"
LIT_UP="u""p"
LIT_PULL="pu""ll"
LIT_RESTART="re""start"
LIT_UPDATE="up""date"
LIT_DESPLEGAR="des""plegar"

# paso_yaml <comando>: paso YAML (a 6 espacios) que ejecuta <comando>.
paso_yaml() {
    printf '      - name: Paso de prueba (mutación)\n        run: %s\n' "$1"
}
COMANDO_COMPOSE_BASE="-f deploy/cell.compose.yml"
BLOQUE_CONFIG="      # comentario con ${LIT_DOCKER} ${LIT_COMPOSE} ${LIT_UP} (no debe disparar)
      - name: Ejecutar ${LIT_DOCKER} ${LIT_COMPOSE} ${LIT_UP} (no dispara)
        run: ${LIT_DOCKER} ${LIT_COMPOSE} config"
LINEA_ARM_MUTADO='        "deploy" => Subcomando::Estado,'
LINEA_USO_MUTADO='  actualizar     --id <cell_id>                Subcomando mutado de prueba.'
# Script mutado invocado por CI-SCRIPT (nombre sin palabra de despliegue).
SCRIPT_MUTADO="verificar_despliegue_mutado.sh"
TOTAL=0
ACIERTOS=0

# correr_caso <etiqueta> [id_esperado] <dir_caso> [archivo_mutado] [archivo_pristino]
#   Sin id_esperado: exige verde. Con id_esperado: exige rojo con
#   FALLA[<id_esperado>] y sin ningún otro id (el exit code nunca basta); si
#   hay archivo mutado, `cmp` debe probar antes que la mutación cambió la copia.
correr_caso() {
    local etiqueta="$1" esperado="${2:-}" dir_caso="${3:-}" mutado="${4:-}" pristino="${5:-}"
    TOTAL=$((TOTAL + 1))
    if [ -n "$mutado" ]; then
        if [ ! -s "$mutado" ] || { [ -e "$pristino" ] && cmp -s "$mutado" "$pristino"; }; then
            echo "FALLA: $etiqueta -> la mutación no cambió el archivo; no es una prueba"
            return
        fi
    fi
    local salida estado ok=1 o
    salida="$(verificar_todo "$dir_caso/.github/workflows" "$dir_caso/argumentos.rs" 2>&1)"
    estado=$?
    if [ -n "$esperado" ]; then
        if [ "$estado" -eq 0 ]; then
            echo "FALLA: $etiqueta -> el guardia PASÓ la copia mutada (no es un guardia)"
            ok=0
        fi
        if ! printf '%s\n' "$salida" | grep -q "FALLA\[$esperado\]"; then
            echo "FALLA: $etiqueta -> la salida no identifica FALLA[$esperado]:"
            printf '%s\n' "$salida"
            ok=0
        fi
        for o in CI-COMPOSE CI-SCRIPT CLI-SUBCOMANDO CLI-USO; do
            [ "$o" = "$esperado" ] && continue
            if printf '%s\n' "$salida" | grep -q "FALLA\[$o\]"; then
                echo "FALLA: $etiqueta -> apareció un id ajeno: FALLA[$o]"
                ok=0
            fi
        done
        if [ "$ok" -eq 1 ]; then
            echo "PASA: $etiqueta -> el guardia falla con FALLA[$esperado] y ningún otro id"
            ACIERTOS=$((ACIERTOS + 1))
        fi
    else
        if [ "$estado" -eq 0 ]; then
            echo "PASA: $etiqueta -> el guardia sigue en verde (caso que no debe disparar)"
            ACIERTOS=$((ACIERTOS + 1))
        else
            echo "FALLA: $etiqueta -> el guardia se puso rojo en un caso que no debe disparar:"
            printf '%s\n' "$salida"
        fi
    fi
}

echo "Modo --autoprueba: cada comprobación, una por vez, debe detectar su mutación con su id exacto; los casos exentos deben quedar verdes."
CI="$ORIGINAL_WORKFLOW"
correr_caso "copias limpias (espejo base)" "" "$BASE"

# caso_compose <nombre> <etiqueta> <comando>: inserta un paso con <comando> en
# guardas-limites de ci.yml y exige rojo CI-COMPOSE.
caso_compose() {
    local nombre="$1" etiqueta="$2" comando="$3"
    CASO="$(preparar_caso "$nombre")"
    insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "guardas-limites" "$(paso_yaml "$comando")"
    correr_caso "$etiqueta" "CI-COMPOSE" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"
}
caso_compose "m1-up" "M1: ${LIT_DOCKER} ${LIT_COMPOSE} ${LIT_UP} en guardas-limites" "${LIT_DOCKER} ${LIT_COMPOSE} $COMANDO_COMPOSE_BASE ${LIT_UP} -d"
caso_compose "m1-pull" "M1: ${LIT_DOCKER} ${LIT_COMPOSE} ${LIT_PULL} en guardas-limites" "${LIT_DOCKER} ${LIT_COMPOSE} $COMANDO_COMPOSE_BASE ${LIT_PULL}"
caso_compose "m1-restart" "M1: ${LIT_DOCKER} ${LIT_COMPOSE} ${LIT_RESTART} en guardas-limites" "${LIT_DOCKER} ${LIT_COMPOSE} $COMANDO_COMPOSE_BASE ${LIT_RESTART}"
caso_compose "m1-guion" "M1: ${LIT_DOCKER_COMPOSE} ${LIT_UP} (forma con guion) en guardas-limites" "${LIT_DOCKER_COMPOSE} $COMANDO_COMPOSE_BASE ${LIT_UP} -d"

# Segundo flujo de trabajo con extensión .yaml: GitHub Actions también lo carga.
CASO="$(preparar_caso "m1-yaml")"
printf 'name: otro\non: push\njobs:\n  otro:\n    runs-on: ubuntu-latest\n    steps:\n%s' "$(paso_yaml "${LIT_DOCKER} ${LIT_COMPOSE} $COMANDO_COMPOSE_BASE ${LIT_UP} -d")" > "$CASO/.github/workflows/otro.yaml"
correr_caso "M1: ${LIT_DOCKER} ${LIT_COMPOSE} ${LIT_UP} en un segundo flujo otro.yaml" "CI-COMPOSE" "$CASO" "$CASO/.github/workflows/otro.yaml" "$BASE/.github/workflows/otro.yaml"

CASO="$(preparar_caso "m2-cli-subcomando")"; insertar_antes_de "$CASO/argumentos.rs" '^[[:space:]]*"status"[[:space:]]*=>' "$LINEA_ARM_MUTADO"
correr_caso "M2: inyectar un brazo de subcomando en el match" "CLI-SUBCOMANDO" "$CASO" "$CASO/argumentos.rs" "$ORIGINAL_ARGUMENTOS"
CASO="$(preparar_caso "m3-cli-uso")"; insertar_antes_de "$CASO/argumentos.rs" '^[[:space:]]*status[[:space:]]+--id' "$LINEA_USO_MUTADO"
correr_caso "M3: inyectar una línea que comienza con palabra de despliegue en el texto de uso" "CLI-USO" "$CASO" "$CASO/argumentos.rs" "$ORIGINAL_ARGUMENTOS"

# caso_script <nombre> <etiqueta> <invocacion>: crea un script mutado con la
# regla compose y lo invoca desde guardas-limites con <invocacion>.
caso_script() {
    local nombre="$1" etiqueta="$2" invocacion="$3"
    CASO="$(preparar_caso "$nombre")"; mkdir -p "$CASO/deploy"
    printf '#!/usr/bin/env bash\n# script de prueba mutado para CI-SCRIPT\n%s %s %s %s -d\n' \
        "$LIT_DOCKER" "$LIT_COMPOSE" "$COMANDO_COMPOSE_BASE" "$LIT_UP" > "$CASO/deploy/$SCRIPT_MUTADO"
    insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "guardas-limites" "$(paso_yaml "$invocacion deploy/$SCRIPT_MUTADO")"
    correr_caso "$etiqueta" "CI-SCRIPT" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"
}
caso_script "m4-bash" "M4: bash deploy/<script> cuyo contenido ejecuta la regla compose" "bash"
caso_script "m4-sh" "M4: sh deploy/<script> cuyo contenido ejecuta la regla compose" "sh"
CASO="$(preparar_caso "m4-punto")"; mkdir -p "$CASO/deploy"
printf '#!/usr/bin/env bash\n%s %s %s %s -d\n' "$LIT_DOCKER" "$LIT_COMPOSE" "$COMANDO_COMPOSE_BASE" "$LIT_UP" > "$CASO/deploy/$SCRIPT_MUTADO"
insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "guardas-limites" "$(paso_yaml "./deploy/$SCRIPT_MUTADO")"
correr_caso "M4: ./deploy/<script> cuyo contenido ejecuta la regla compose" "CI-SCRIPT" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"

# Palabras update y desplegar: brazo del match (CLI-SUBCOMANDO), línea de uso
# (CLI-USO) y tallo de script (CI-SCRIPT). En el caso del tallo el script SÍ
# existe y está limpio en la copia: solo la comprobación del tallo puede ponerlo
# rojo (si no, «no existe» taparía la deriva de PALABRA_UPDATE/PALABRA_DESPLEGAR).
for LIT_PALABRA in "$LIT_UPDATE" "$LIT_DESPLEGAR"; do
    CASO="$(preparar_caso "m2-$LIT_PALABRA")"
    insertar_antes_de "$CASO/argumentos.rs" '^[[:space:]]*"status"[[:space:]]*=>' "        \"$LIT_PALABRA\" => Subcomando::Estado,"
    correr_caso "M2: brazo de subcomando \"$LIT_PALABRA\" en el match" "CLI-SUBCOMANDO" "$CASO" "$CASO/argumentos.rs" "$ORIGINAL_ARGUMENTOS"
    CASO="$(preparar_caso "m3-$LIT_PALABRA")"
    insertar_antes_de "$CASO/argumentos.rs" '^[[:space:]]*status[[:space:]]+--id' "  $LIT_PALABRA     --id <cell_id>                Subcomando mutado de prueba."
    correr_caso "M3: línea de uso que comienza con $LIT_PALABRA" "CLI-USO" "$CASO" "$CASO/argumentos.rs" "$ORIGINAL_ARGUMENTOS"
    CASO="$(preparar_caso "m4-tallo-$LIT_PALABRA")"; mkdir -p "$CASO/deploy"
    printf '#!/usr/bin/env bash\necho limpio\n' > "$CASO/deploy/$LIT_PALABRA.sh"
    insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "guardas-limites" "$(paso_yaml "bash deploy/$LIT_PALABRA.sh")"
    correr_caso "M4: script existente y limpio con tallo de despliegue ($LIT_PALABRA.sh)" "CI-SCRIPT" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"
done

# Casos que NO deben disparar.
CASO="$(preparar_caso "exento-imagenes")"; insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "imagenes" "$(paso_yaml "${LIT_DOCKER} ${LIT_COMPOSE} $COMANDO_COMPOSE_BASE ${LIT_UP} -d")"
correr_caso "exención: paso de compose dentro del trabajo imagenes" "" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"
CASO="$(preparar_caso "config-no-dispara")"; insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "guardas-limites" "$BLOQUE_CONFIG"
correr_caso "config: línea de compose sin verbo y comentario con las palabras" "" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"
CASO="$(preparar_caso "palabra-en-otro-contexto")"
insertar_paso_en_trabajo "$CASO/.github/workflows/ci.yml" "guardas-limites" "      # despliegue: la palabra deployment y actualizar_estado no son verbos
      - name: Reportar deployment y actualizar_estado (no dispara)
        run: echo actualizar_estado deployment"
correr_caso "palabras de despliegue en comentario, name: y identificador ajeno" "" "$CASO" "$CASO/.github/workflows/ci.yml" "$CI"
CASO="$(preparar_caso "cli-identificador-ajeno")"
insertar_antes_de "$CASO/argumentos.rs" '^[[:space:]]*"status"[[:space:]]*=>' '        "estado" if actualizar_estado == deployment => Subcomando::Estado,'
correr_caso "CLI: identificador actualizar_estado y palabra deployment en línea con ==" "" "$CASO" "$CASO/argumentos.rs" "$ORIGINAL_ARGUMENTOS"

echo ""
echo "Resumen autoprueba: $ACIERTOS/$TOTAL casos pasan (cada mutación debe hacer fallar al guardia con su id exacto)"
if [ "$ACIERTOS" -eq "$TOTAL" ]; then
    echo "OK: el guardia falla bajo cada mutación y no dispara en los casos exentos"
    exit 0
fi
echo "FALLA: el guardia no detectó todas las mutaciones (o disparó en un caso exento)"
exit 1