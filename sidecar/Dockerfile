# ============================================================================
# Imagen del sidecar Go de la célula (sidecar/), multi-etapa.
# ============================================================================

# Argumento de build que fija la pseudoversión de whatsmeow pinneada en
# sidecar/go.mod. Se declara ANTES del primer FROM como ARG de "pre-build":
# queda registrado como variable de build pero no se propaga implícitamente a
# ninguna etapa. Cada etapa que lo necesite debe redeclararlo después de su
# FROM (Docker no comparte ARG entre etapas por construcción). El valor por
# omisión es la pseudoversión actual de go.mod; un valor distinto en build
# (`--build-arg ARG_VERSION_WHATSMEOW=...`) debe coincidir con el pin de
# go.mod o el build se detiene en la compuerta de la etapa constructora.
ARG ARG_VERSION_WHATSMEOW=v0.0.0-20260722203353-e9a033b24933

# --- Etapa constructora sobre la misma serie menor de Alpine/musl ----------
#
# POR QUÉ golang:1.26.5-alpine EXACTAMENTE: sidecar/go.mod declara `go 1.26.5`
# y la coincidencia con el builder evita que la herramienta tenga que
# reconciliar o descargar un patch distinto dentro del contenedor. El build
# solo depende entonces de la red para el proxy de módulos y los repos apk.
# Si esta etiqueta exacta dejara de publicarse en Docker Hub, la sustitución
# correcta es el parche más cercano dentro de la misma serie 1.26.x, no el
# rodillo `golang:1.26-alpine`, porque reintroduciría el riesgo de
# reconciliación que el anclaje exacto elimina.
#
# POR QUÉ Alpine y no Debian/Ubuntu: la etapa final corre sobre Alpine (misma
# serie menor, `alpine:3`), así que el binario debe estar ligado estáticamente
# contra musl. Alpine también aquí en el builder asegura que la libc de
# compilación sea musl y no glibc; sin esa coincidencia, un binario Go
# "estático" compilado contra glibc puede llevar dependencias dinámicas que
# la imagen mínima no traería.
FROM golang:1.26.5-alpine AS constructor

# Redeclaración del ARG dentro de esta etapa: sin esto, ${ARG_VERSION_WHATSMEOW}
# no es visible para RUN, COPY ni LABEL de esta etapa aunque exista como ARG
# global pre-FROM.
ARG ARG_VERSION_WHATSMEOW

# POR QUÉ CGO_ENABLED=0 antes de cualquier RUN/COPY: apaga el puente a C en
# el proceso de compilación. Las dependencias declaradas en go.mod que
# necesitan compilación nativa (modernc.org/sqlite y sus transitivas puras
# modernc.org/libc, modernc.org/mathutil, modernc.org/memory) son Go puro por
# diseño y no requieren cgo; ese dato está confirmado leyendo go.mod, no
# asumido. Con CGO deshabilitado, la ausencia deliberada de gcc/musl-dev en
# esta etapa no es un riesgo sino la prueba: si un binario Go se compila
# sin toolchain C en absoluto, su afirmación de "estático" no es declaración
# sino consecuencia verificable.
ENV CGO_ENABLED=0

WORKDIR /app

# Se copia solo el grafo de módulos y se descargan las dependencias ANTES de
# traer el código fuente. Esto desacopla la caché de `go mod download` de los
# cambios en main.go o internal/: una modificación del código no fuerza
# redescarga del grafo entero, solo re-compilación. go.mod y go.sum van
# primero porque `go mod download` los necesita para verificar el grafo.
COPY go.mod go.sum ./

# Compuerta de build: verifica que el valor del ARG coincida con la versión
# pinneada por commit en go.mod para go.mau.fi/whatsmeow. El match es
# deliberadamente dirigido (un único grep+awk sobre la línea conocida del
# require directo), no un parser general de go.mod/SemVer. Si el ARG no
# coincide con el pin, el build falla aquí, ANTES de descargar módulos y
# ANTES de compilar: una etiqueta stale o mal tecleada no puede llegar a la
# imagen final sin detección.
#
# POR QUÉ falla cerrado: el canario de 72 horas de la etapa A-3 necesita
# saber qué build de whatsmeow lleva la imagen en producción sin abrir
# capas; este ARG es la única fuente de verdad que sobrevive al build, y
# silenciar su desajuste escondería exactamente el fallo que el canario
# está diseñado para detectar.
RUN PIN=$(grep -E '^[[:space:]]*go\.mau\.fi/whatsmeow[[:space:]]' go.mod | awk '{print $2}') && \
    if [ -z "$PIN" ]; then \
        echo "ERROR_HEX065: no se encontro la linea require go.mau.fi/whatsmeow en go.mod" >&2; \
        exit 1; \
    fi && \
    if [ "$PIN" != "${ARG_VERSION_WHATSMEOW}" ]; then \
        echo "ERROR_HEX065: la pseudoversion de whatsmeow pinneada en go.mod ($PIN) no coincide con el ARG de build (${ARG_VERSION_WHATSMEOW})" >&2; \
        exit 1; \
    fi && \
    echo "verificado: whatsmeow pinneado en go.mod = $PIN"

# Descarga de módulos con verificación de sumas (go.sum presente). Sin esta
# línea la compilación posterior funcionaría pero la imagen no estaría
# certificada contra el grafo declarado en go.sum.
RUN go mod download

# Ahora sí, el resto del código. El .dockerignore ya excluyó caches,
# documentación, secretos y datos de inquilino, así que este COPY entra
# limpio.
COPY main.go ./
COPY internal/ ./internal/

# Compilación del único paquete compilable del módulo (el `main` raíz,
# cuyo binario se llama hexcell-sidecar por el contrato IPC). Sin
# `-ldflags`, `-trimpath`, `-s -w` ni otras tunelizaciones de tamaño: ese
# retoque es tarea 3 de la etapa A-6 y se aplica por igual a las dos
# imágenes; aquí el objetivo es que el binario arranque y se pueda medir.
RUN go build -o /usr/local/bin/hexcell-sidecar .

# --- Etapa final mínima ------------------------------------------------------
#
# POR QUÉ `alpine:3` y no `scratch`: el sidecar es Go puro y técnicamente
# podría correr sobre scratch, pero se elige la misma serie menor de la
# imagen del núcleo deliberadamente. La simetría entre las dos imágenes del
# producto (núcleo + sidecar sobre la misma base) hace un modelo mental
# compartido: mismas rutas, mismas herramientas de diagnóstico disponibles
# en la etapa A-6 cuando se endurezca el shell, mismas reglas de capa. Es
# una decisión de claridad, no de tamaño.
#
# El tag rodante de la serie menor (no un parche pinneado) recibe las
# correcciones de seguridad de la distribución sin reconstruir por una sola
# etiqueta, igual que en la imagen del núcleo.
FROM alpine:3 AS final

# Redeclaración del ARG en esta etapa: Docker no propaga ARG entre etapas.
# Sin redeclarar, ${ARG_VERSION_WHATSMEOW} sería cadena vacía dentro de
# esta etapa y el LABEL registraría una versión vacía, lo cual es un
# fallo silencioso peor que la falta de etiqueta.
ARG ARG_VERSION_WHATSMEOW

# POR QUÉ ca-certificates SÍ se instala aquí: a diferencia del núcleo
# (rustls + hyper-rustls con webpki-tokio, cuyas raíces de confianza TLS
# están compiladas dentro del binario), el sidecar delega la verificación
# TLS hacia los servidores de WhatsApp en el almacén del sistema operativo.
# Omitir este paquete sería un fallo silencioso de TLS en tiempo de
# ejecución en el primer handshake contra Meta, no un ahorro de tamaño.
# La instalación es la operación inversa a la documentada en la imagen del
# núcleo y por eso merece explicación explícita: una imagen hermana con
# una decisión contraria sobre el mismo paquete se lee con sorpresa si no
# se justifica.
RUN apk add --no-cache ca-certificates

# Único artefacto que sale de la constructora: el binario. Ni el toolchain
# Go, ni el caché de módulos, ni los objetos intermedios cruzan esta
# frontera.
COPY --from=constructor /usr/local/bin/hexcell-sidecar /usr/local/bin/hexcell-sidecar

# Etiqueta OCI que expone la pseudoversión de whatsmeow que lleva la imagen.
# Se consulta con `docker inspect` sin abrir capas: el canario de 72 horas
# de la etapa A-3 necesita auditar qué build de whatsmeow corre en cada
# célula sin tener que entrar al contenedor. La clave sigue la nomenclatura
# estándar de org.opencontainers.image.* para que cualquier herramienta
# que entienda OCI la recoja sin configuración adicional.
LABEL org.opencontainers.image.version="${ARG_VERSION_WHATSMEOW}" \
      org.opencontainers.image.title="hexcell-sidecar" \
      org.opencontainers.image.description="Sidecar Go de la celula HexCell sobre canal propio (whatsmeow)"

# POR QUÉ ENTRYPOINT sin CMD: el binario lee TODA su configuración de
# variables de entorno al arrancar. Cada una de las ~30 variables
# HEXCELL_* que conoce el paquete internal/configuracion tiene un valor
# por omisión en Cargar; HEXCELL_VENTANA_ZONA es la única estrictamente
# requerida (HEX-033). Nada de configuración de negocio ni credencial se
# hornea en la imagen: todo llega en tiempo de ejecución. Sin CMD se
# evita que un `docker run <imagen> <cmd>` inadvertido sobrescriba el
# proceso principal del sidecar con algo que no es el sidecar.
ENTRYPOINT ["/usr/local/bin/hexcell-sidecar"]