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
# POR QUE un comodín y no la lista de archivos: hasta el 2026-09-10 esta línea
# decía `COPY main.go ./`, enumerando los fuentes de la raíz por nombre. HEX-066
# agregó `arranque.go` al paquete `main` y nadie tocó este COPY, así que el
# archivo no entraba al contexto y la imagen dejó de compilar con `undefined:
# abrirRecursosDeArranque`. El defecto sobrevivió a `go build`, a `go vet` y a
# `go test` —los tres ven el árbol completo, no el contexto de Docker— y solo
# aparece al construir la imagen. Enumerar rompe la imagen en silencio cada vez
# que nace un archivo en la raíz del módulo; el comodín elimina esa clase entera.
# Los `*_test.go` que arrastra no llegan al binario ni a la imagen final: se
# quedan en la etapa constructora, que se descarta.
COPY *.go ./
COPY internal/ ./internal/

# Compilación del único paquete compilable del módulo (el `main` raíz, cuyo
# binario se llama hexcell-sidecar por el contrato IPC).
#
# POR QUE se quitan los símbolos (`-s -w`) pese a que encarece diagnosticar un
# pánico en producción: la construcción es reproducible. La pseudoversión de
# whatsmeow está fijada en `ARG_VERSION_WHATSMEOW`, afirmada contra `go.mod` por
# la guarda de más arriba y publicada en la etiqueta OCI de la imagen, así que un
# pánico sin símbolos se resimboliza reconstruyendo el mismo commit. Sin esa
# reproducibilidad, quitar símbolos sería una pérdida neta y no se haría.
# El costo queda escrito, no aceptado en silencio: hasta que alguien reconstruya,
# un pánico de campo llega sin nombres de función ni números de línea.
#
# POR QUE `-trimpath`: borra del binario las rutas absolutas de la máquina de
# compilación. Sirve a la reproducibilidad y evita filtrar la ruta del build en
# una imagen que se publica.
#
# Tamaños medidos el 2026-09-10, ambos con `docker image inspect --format
# '{{.Size}}'`:
#   antes (mismo fuente, sin las dos flags): 39.959.335 bytes
#   despues (con ambas flags):                30.374.876 bytes
#   reduccion:                                 9.584.459 bytes (24,0 %)
# Ambas medidas se tomaron sobre EL MISMO arbol de fuentes, reconstruyendo la
# linea de arriba sin flags, en vez de comparar contra los 39.960.738 bytes que
# midio HEX-065: aquella imagen no contenia `arranque.go`, asi que restarle el
# tamanio de hoy mezclaria el efecto de las flags con el de un fuente distinto.
# Son medidas observadas, no un techo: NO existe ninguna compuerta que falle si
# la imagen crece. El plan de la etapa A-6 no fija un tope y no se inventa uno
# aquí, porque una actualización legítima de whatsmeow rompería la CI sin que
# exista defecto alguno. El registro formal de tamaños es la tarea 16.
RUN go build -trimpath -ldflags="-s -w" -o /usr/local/bin/hexcell-sidecar .

# --- Etapa final mínima ------------------------------------------------------
#
# POR QUÉ `alpine:3` y no `scratch`: el sidecar es Go puro y técnicamente
# podría correr sobre scratch, pero se elige la misma serie menor de la
# imagen del núcleo deliberadamente. La simetría entre las dos imágenes del
# producto (núcleo + sidecar sobre la misma base) da un modelo mental
# compartido —mismas rutas, mismas reglas de capa— y permite instalar
# ca-certificates vía apk, que el sidecar necesita para delegar la
# verificación TLS en el almacén del sistema. Es una decisión de claridad,
# no de tamaño.
#
# Costo aceptado de esta elección, declarado para no sorprender: tras el
# endurecimiento del shell (más abajo) ya NO quedan herramientas de
# diagnóstico dentro del contenedor y `docker exec <célula> sh` deja de
# funcionar. La readiness se sondea por HTTP desde un contenedor hermano
# (tarea 11), y la CLI gobierna Docker por su socket y el protocolo IPC,
# nunca por exec.
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

# --- Endurecimiento: identidad no privilegiada (tarea 4 de la etapa A-6) -------
#
# POR QUÉ un UID/GID numérico FIJADO a 10001 y no el que adduser asignaría por
# omisión: los dos contenedores de la célula (núcleo y sidecar) escriben en el
# MISMO volumen por célula. Si el UID difiriera entre las dos imágenes, una de
# ellas perdería el acceso de escritura al volumen de la otra o el aislamiento
# por célula (NFR-05) se rompería en silencio. Por eso el número es un literal
# idéntico en ambas imágenes, no un nombre resuelto en tiempo de ejecución ni
# un valor que el allocator de adduser pudiera elegir distinto entre
# reconstrucciones.
#
# POR QUÉ 10001 y no 1000: 1000 es justo lo que `adduser` asigna por omisión
# cuando se omite `-u`; si ese flag se cayera por error, la imagen produciría
# un UID visiblemente distinto y la comprobación de USER 10001:10001 fallaría
# con ruido, en vez de auto-cumplirse por accidente. 10001 tampoco choca con el
# primer usuario humano del anfitrión (uid 1000), de modo que un bind mount mal
# especificado no puede entregar a una célula la identidad del operador del
# host. Medido sobre alpine:3 = 3.24.1: el UID de sistema más alto por debajo
# de 1000 es 405 (guest) y 65534 es `nobody`; 10001 no colisiona con ninguno y
# deja margen si un bump de la imagen base añade un usuario de sistema.
#
# POR QUÉ /var/lib/hexcell: es el punto de montaje real del volumen de la
# célula y el padre de cada ruta por omisión del sidecar
# (RutaSqlstorePorOmision = /var/lib/hexcell/sqlstore.db, RutaIdentidadPorOmision
# = /var/lib/hexcell/identidad.db, RutaOutboxPorOmision = /var/lib/hexcell/outbox.db)
# y del socket IPC (RutaSocketPorOmision = /var/lib/hexcell/ipc/sidecar.sock),
# cuyo directorio crea el propio sidecar con os.MkdirAll(dir, 0755) al escuchar.
# El mkdir+chown es la pieza que hace posible el arranque en frío sobre un
# volumen vacío: medido el 2026-09-10, un volumen NOMBRADO recién creado hereda
# el dueño y el modo del directorio de montaje de esta imagen, mientras que un
# bind mount no lo hace (ver el bloque de flags diferidas más abajo).
RUN addgroup -g 10001 -S hexcell \
    && adduser -u 10001 -S -G hexcell -H -D -s /sbin/nologin hexcell \
    && mkdir -p /var/lib/hexcell \
    && chown 10001:10001 /var/lib/hexcell \
    && chmod 0700 /var/lib/hexcell

# --- Defensa del rootfs de solo lectura: redirigir el derrame de SQLite -------
#
# POR QUÉ TMPDIR y SQLITE_TMPDIR apuntan al volumen: bajo las flags diferidas a
# HEX-068 (read_only) todo el rootfs es de solo lectura salvo /var/lib/hexcell.
# El sidecar usa modernc.org/sqlite y no fija PRAGMA temp_store=MEMORY ni
# SQLITE_TMPDIR en el código, así que un hipotético derrame a disco caería en
# /var/tmp, /usr/tmp, /tmp o el directorio de trabajo, todos de solo lectura.
# Redirigir el fallback hacia el volumen es una mitigación defensiva a nivel de
# imagen que no cuesta nada y no toca fuente. Es defensiva, no una prueba: no se
# conoce ninguna ruta de consulta que hoy dispare un derrame, y fijar temp_store
# en el código es una tarea aparte, no esta.
ENV TMPDIR=/var/lib/hexcell \
    SQLITE_TMPDIR=/var/lib/hexcell

# --- Retirada del shell hasta donde alpine:3 lo permite ----------------------
#
# POR QUÉ este RUN va DESPUÉS de `apk add --no-cache ca-certificates`: el
# disparador post-instalación de ese paquete (update-ca-certificates) es un
# script de shell, y borrar el shell antes lo rompería.
#
# POR QUÉ no se intenta `apk del busybox`: es rechazado con "not removed due
# to: busybox: alpine-baselayout" (medido 2026-09-10), porque alpine-baselayout
# depende duramente de él. Tampoco `apk del busybox-binsh` retira el shell: en
# alpine:3 = 3.24.1 es IGUALMENTE rechazado (alpine-baselayout depende del
# fichero /bin/sh, que provee busybox-binsh) y devuelve exit 0 sin retirar nada.
# Se mantiene la llamada como documentación del camino prescrito por el
# contrato, pero la retirada real es `rm -f /bin/sh` (el enlace del shell) y
# `rm -f /bin/busybox` (el binario multi-call del que cuelgan TODOS los applets).
#
# Costo aceptado, declarado para no sorprender: quedan enlaces simbólicos
# colgantes (/bin/ls, /bin/cp, ...) apuntando a /bin/busybox. Son inocuos en
# tiempo de ejecución —el ENTRYPOINT es un binario estático en forma exec y
# ningún applet se invoca jamás—, pero un escáner de imágenes puede señalarlos.
# La limpieza `find -lname '*busybox'` sugerida no es viable aquí: el find de
# BusyBox no implementa `-lname`, y tras borrar /bin/busybox no queda find que
# ejecutar. No se retiran apk-tools ni /lib/apk/db: es el catálogo de paquetes
# instalados que un escáner de CVE (Trivy, Grype, ...) necesita leer para
# enumerar el software de la imagen. Ninguna tarea de la etapa A-6 programa
# hoy ese escaneo; conservar la base no es una promesa de auditoría futura,
# es simplemente no cegar a un escáner que el plan todavía no agenda, a
# cambio de un costo de imagen despreciable.
RUN apk del --no-network busybox-binsh \
    && rm -f /bin/sh /bin/busybox

# ============================================================================
# FLAGS DE EJECUCIÓN DIFERIDAS A HEX-068 (plantilla de composición, tarea 8)
# ============================================================================
# Esta imagen es COMPATIBLE con, pero NO IMPONE, las flags de endurecimiento en
# tiempo de ejecución. La plantilla deploy/cell.compose.yml (etapa A-6 tarea 8,
# HEX-068) es quien debe aplicarlas para que el endurecimiento tenga efecto:
#
#   read_only: true
#   cap_drop: [ALL]
#   security_opt: [no-new-privileges:true]
#   volumes: - <volumen_nombrado>:/var/lib/hexcell
#   tmpfs (opcional): solo si un operador quiere un /tmp escribible; el
#       ENTRYPOINT no lo necesita.
#
# ADVERTENCIA al autor de la plantilla: el volumen DEBE ser un volumen NOMBRADO
# de Docker, no un bind mount. Un volumen nombrado recién creado hereda el dueño
# y el modo del directorio de montaje de esta imagen (10001:10001, 0700); un
# bind mount NO, y fallará con EACCES a menos que el directorio del host se
# pre-propietarice a 10001:10001. Medido 2026-09-10.
#
# El chmod 0700 de arriba protege el volumen contra OTROS UID del host; NO es la
# garantía de aislamiento NFR-05. Como una sola imagen sirve a todas las células,
# todas corren como el mismo 10001 y la separación entre célula A y célula B
# descansa en la topología de montaje y la red por célula (tarea 5); solo la
# prueba de aislamiento (tarea 17) la demuestra.

# POR QUÉ USER numérico en ambos lados del colon: sin resolución de
# /etc/passwd en tiempo de ejecución y sin posibilidad de que el valor derive
# con un nombre. El valor es el mismo literal 10001:10001 que fija la imagen
# del núcleo.
USER 10001:10001

# POR QUÉ ENTRYPOINT sin CMD: el binario lee TODA su configuración de
# variables de entorno al arrancar. Cada una de las ~30 variables
# HEXCELL_* que conoce el paquete internal/configuracion tiene un valor
# por omisión en Cargar; HEXCELL_VENTANA_ZONA es la única estrictamente
# requerida (HEX-033). Nada de configuración de negocio ni credencial se
# hornea en la imagen: todo llega en tiempo de ejecución. Sin CMD se
# evita que un `docker run <imagen> <cmd>` inadvertido sobrescriba el
# proceso principal del sidecar con algo que no es el sidecar.
ENTRYPOINT ["/usr/local/bin/hexcell-sidecar"]