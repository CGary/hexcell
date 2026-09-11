# ============================================================================
# Imagen del núcleo de la célula (crates/hexcell), multi-etapa.
# ============================================================================

# --- Etapa constructora sobre Alpine/musl -----------------------------------
#
# POR QUÉ Alpine + musl: la etapa final corre sobre Alpine, así que el binario debe
# ser un ejecutable ligado estáticamente contra musl. No puede depender de la glibc
# de la máquina anfitriona ni de una librería compartida que la imagen mínima no
# traería.
#
# POR QUÉ rust:1.92-alpine exactamente: es el canal que fija rust-toolchain.toml
# (1.92.0). Al coincidir, rustup no necesita reconciliar ninguna descarga de
# toolchain dentro del contenedor y el build no depende de la red más allá de los
# dos repositorios de paquetes y de crates.io.
FROM rust:1.92-alpine AS constructor

# musl-dev: la compilación enlazada de C de `rusqlite` (feature "bundled") y de `ring`
# (traído por rustls/hyper-rustls) necesita los encabezados de la libc musl en tiempo
# de compilación. El compilador C ya viene en la imagen; los encabezados son la pieza
# que se garantiza aquí y no se asume del futuro de la imagen base.
RUN apk add --no-cache musl-dev

WORKDIR /app

# El contexto está filtrado por .dockerignore: todo lo que cargo no necesita para
# resolver el workspace (docs, sidecar, target/, .git, bases, secretos...) queda fuera.
COPY . .

# Se compila solo el binario de la célula y su grafo de dependencias por ruta
# (hexcell-core, hexcell-storage, hexcell-canal-simulado, hexcell-canal-whatsmeow);
# los demás crates del workspace ni se compilan. El perfil [profile.release] del
# Cargo.toml raíz se usa tal cual (opt-level="z", lto, codegen-units=1, strip,
# panic="abort"): el retuneo de enlazado y tamaño es otra tarea de esta etapa, no esta.
RUN cargo build --release -p hexcell \
    && cp target/release/hexcell /usr/local/bin/hexcell

# --- Etapa final mínima ------------------------------------------------------
#
# `alpine:3` es la etiqueta real de la serie 3.x que fija el plan de la etapa A-6;
# se deja en el rodillo de la serie menor y no se pinnea un parche para recibir las
# correcciones de seguridad de la distribución sin reconstruir por una sola etiqueta.
FROM alpine:3 AS final

# Único artefacto que sale de la constructora: el binario. Ni el toolchain, ni el
# registro de crates, ni los objetos intermedios de target/ cruzan esta frontera.
COPY --from=constructor /usr/local/bin/hexcell /usr/local/bin/hexcell

# POR QUÉ no se instala ca-certificates: las raíces de confianza TLS van compiladas
# dentro del binario (rustls + hyper-rustls con webpki-tokio). El paquete de CA del
# sistema sería peso muerto contra el presupuesto por célula de NFR-01.
#
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
# célula y el padre de la ruta por omisión del socket IPC del núcleo
# (RUTA_SOCKET_IPC_POR_DEFECTO = /var/lib/hexcell/ipc/sidecar.sock) y de cada
# ruta por omisión del sidecar (sqlstore, identidad y outbox). El mkdir+chown
# es la pieza que hace posible el arranque en frío sobre un volumen vacío:
# medido el 2026-09-10, un volumen NOMBRADO recién creado hereda el dueño y el
# modo del directorio de montaje de esta imagen, mientras que un bind mount no
# lo hace (ver el bloque de flags diferidas más abajo).
RUN addgroup -g 10001 -S hexcell \
    && adduser -u 10001 -S -G hexcell -H -D -s /sbin/nologin hexcell \
    && mkdir -p /var/lib/hexcell \
    && chown 10001:10001 /var/lib/hexcell \
    && chmod 0700 /var/lib/hexcell

# --- Defensa del rootfs de solo lectura: redirigir el derrame de SQLite -------
#
# POR QUÉ TMPDIR y SQLITE_TMPDIR apuntan al volumen: bajo las flags diferidas a
# HEX-068 (read_only) todo el rootfs es de solo lectura salvo /var/lib/hexcell.
# Ni el núcleo ni el sidecar fijan PRAGMA temp_store=MEMORY ni SQLITE_TMPDIR en
# el código, así que un hipotético derrame a disco de SQLite caería en /var/tmp,
# /usr/tmp, /tmp o el directorio de trabajo, todos de solo lectura. Redirigir el
# fallback hacia el volumen es una mitigación defensiva a nivel de imagen que no
# cuesta nada y no toca fuente. Es defensiva, no una prueba: no se conoce
# ninguna ruta de consulta que hoy dispare un derrame, y fijar temp_store en el
# código es una tarea aparte, no esta.
ENV TMPDIR=/var/lib/hexcell \
    SQLITE_TMPDIR=/var/lib/hexcell

# --- Retirada del shell hasta donde alpine:3 lo permite ----------------------
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
#
# EXCEPCIÓN: `hexcell respaldar --directorio <ruta>` es un subcomando manual del
# MISMO binario, nunca el ENTRYPOINT, y escribe los cinco archivos de respaldo en
# un directorio absoluto suministrado por el operador FUERA de la raíz de datos.
# Bajo un rootfs de solo lectura ese destino debe ser a su vez un mount
# escribible, o el procedimiento de respaldo A-2 fallará solo en producción.

# POR QUÉ USER numérico en ambos lados del colon: sin resolución de
# /etc/passwd en tiempo de ejecución y sin posibilidad de que el valor derive
# con un nombre. El valor es el mismo literal 10001:10001 que fija la imagen
# del sidecar.
USER 10001:10001

# POR QUÉ ENTRYPOINT sin CMD: el binario lee TODA su configuración de variables de
# entorno al arrancar —HEXCELL_ID_CELULA y HEXCELL_RUTA_DATOS son obligatorias; el
# resto tiene valores por defecto de loopback—. No se hornea ningún valor de
# configuración ni credencial en la imagen; todo llega en tiempo de ejecución.
ENTRYPOINT ["/usr/local/bin/hexcell"]