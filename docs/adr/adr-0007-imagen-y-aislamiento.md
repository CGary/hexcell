# ADR-0007 — Imagen, aislamiento y límites de recursos de la célula

* **Estado:** Vigente (2026-09-13).
* **Etapa que lo produce:** A-6 (tareas 4, 5 y 6 del plan de la etapa A-6:
  `docs/plan/fase-a-6-empaquetado-cli.md`).
* **Relación con otros ADR:** ninguno que superseder. Es una **transcripción** de decisiones ya
  tomadas e implementadas por HEX-069 (imágenes base y endurecimiento en tiempo de construcción),
  HEX-070 (composición de dos contenedores y modelo de propiedad del volumen) y HEX-078 (esta tarea:
  límites de recursos). No introduce ninguna decisión de producto ni de arquitectura nueva. El
  catálogo de la célula lo cierran `adr-0011` (arquitectura de sidecar e IPC) y `adr-0015`
  (política de convivencia con el baneo).

## Contexto

El plan de la etapa A-6 divide el empaquetado de la célula en tareas que se ejecutaron por
separado: la 4 endurece las imágenes en tiempo de construcción, la 5 compone la célula (red local,
volumen compartido, socket IPC), la 6 fija los límites de recursos y la 8 parametriza la plantilla
de arranque. Cada una cerró su parte con un guardia mecánico propio y probado por mutación. Este
ADR reúne por primera vez, en un solo registro normativo, el resultado de las tres primeras,
porque comparten el mismo objeto —la imagen y su aislamiento— y porque el resto del plan los
referencia como un bloque.

Tres hechos medidos condicionan las decisiones que se transcriben aquí:

1. **El volumen nombrado hereda la propiedad de la imagen; el bind mount no.** Medido el
   2026-09-10: un volumen nombrado recién creado copia el dueño y el modo del directorio de
   montaje de la imagen (`/var/lib/hexcell`, 10001:10001, 0700) y arranca en frío sin preparar
   nada; un bind mount no hereda esa propiedad y falla con `Permission denied` al primer `open()`
   del binario, a menos que el directorio del host se pre-propietarice desde fuera de la célula.
2. **`docker compose config` resuelve `mem_limit: ${VAR}` a una cadena de bytes crudos**, no a la
   forma "48m" del env-file. Medido el 2026-09-13 contra compose 5.5.1: `48m` resuelve a
   `"50331648"`, y al quitar la línea `mem_limit` de un servicio el campo simplemente **desaparece**
   del YAML resuelto —no se sintetiza un valor por omisión. El guardia de límites compara entonces
   contra el valor convertido a bytes, y la comparación es de igualdad exacta, no de presencia.
3. **`docker compose config` resuelve aun con `build.context` inválido** (footgun ya documentado
   por HEX-068 y confirmado por HEX-070): la inspección del YAML resuelto afirma qué declara la
   composición, no que las imágenes existan ni que arranquen.

## Decisión

### a) Imágenes base y endurecimiento en tiempo de construcción (HEX-069)

1. **Imágenes base mínimas `alpine:3` para ambas etapas finales** —núcleo Rust y sidecar Go—,
   con etapas de construcción `rust:1.92-alpine` y `golang:1.26.5-alpine`, ambos pinneados a las
   versiones que fijan `rust-toolchain.toml` y `sidecar/go.mod`. El sidecar (Go puro) podría
   teóricamente vivir sobre `scratch`, pero la base `alpine:3` se mantiene porque la retirada
   completa del shell no es alcanzable en esta serie (medido 2026-09-10: `apk del busybox` deja la
   imagen sin utilidades, y la adición de `alpine-baselayout` reintroduciría dependencias).
2. **Usuario sin privilegios fijo `10001:10001` en ambas imágenes**, creado con `addgroup`/
   `adduser` numéricos y declarado con `USER 10001:10001`. El UID/GID es idéntico en las dos
   imágenes porque ambos contenedores escriben al MISMO volumen compartido; un desajuste rompería
   silenciosamente el acceso cruzado. Se eligió 10001 —y no 1000, el valor por omisión de
   `adduser`— para que la comprobación `USER 10001:10001` no se auto-cumpla por accidente, y porque
   no colisiona con los UID de sistema de `alpine:3` (el más alto por debajo de 1000 es 405,
   `guest`; 65534 es `nobody`).
3. **`/var/lib/hexcell` creado en la imagen con propietario `10001:10001` y modo `0700`**, como
   punto de montaje del volumen de datos y raíz de las rutas por omisión de los binarios (bases
   SQLite, almacenes del sidecar y socket IPC).
4. **El endurecimiento en tiempo de ejecución que esta tarea nombra queda impuesto por la
   composición, no por la imagen** (HEX-070): `read_only: true`, `cap_drop: [ALL]`,
   `security_opt: [no-new-privileges:true]` y `tmpfs: [/tmp]` en ambos servicios. Las cuatro
   banderas viajan juntas y la plantilla las declara en cada bloque de servicio, no en anclas
   reutilizadas, para que el guardia mecánico pueda inspeccionar el servicio resuelto directamente.

### b) Composición de dos contenedores y modelo de propiedad del volumen (HEX-070)

5. **La célula es exactamente dos servicios —`nucleo` y `sidecar`— bajo un mismo proyecto de
   compose**, que comparten UNA red local de célula y UN volumen nombrado. Compose crea la red y el
   volumen antes de levantar ningún contenedor, así que no hay `depends_on` que declarar.
6. **El volumen de datos se monta SIEMPRE como volumen nombrado de Docker, nunca como bind
   mount.** La forma bind (larga o corta, comentada o activa) queda prohibida por la invariante de
   HEX-070: no hereda la propiedad `10001:10001`/`0700` de la imagen y exige pre-propietarizar el
   host desde fuera de la célula. El nombre del volumen es per-célula
   (`${HEXCELL_VOLUMEN_CELULA}`), nunca un literal.
7. **La propiedad del volumen se hereda de la imagen por construcción**: ambas imágenes entregan
   `/var/lib/hexcell` con dueño `10001:10001` y modo `0700`, y un volumen nombrado nuevo copia esa
   propiedad al montarse. Esa herencia, no una preparación externa, es lo que hace que una célula
   arranque en frío sin `Permission denied`. Como todas las células corren como el mismo `10001`,
   la separación entre una célula y otra la da el aislamiento por red y volumen per-célula, no el
   UID.

### c) Límites de recursos por contenedor (esta tarea, HEX-078) — PROVISIONAL

8. **Reparto de memoria: 48 MB al núcleo + 32 MB al sidecar = 80 MB exactos**, el techo por célula
   de NFR-01 sobre canal propio. La suma no puede desviarse del techo ni por encima ni por abajo.
   **PROVISIONAL pendiente de la tarea 16** (medición real de RSS de la célula compuesta bajo los
   límites de cgroup), según el orden de ejecución del plan: "6 antes de 16, con ajuste posterior".
9. **Reparto de CPU: 0.5 al núcleo y 0.25 al sidecar**, también **PROVISIONAL** pendiente de la
   tarea 16. No se encontró un motivo razonado para cambiarlos respecto de los valores previos.
10. **Límite de descriptores de archivo (`ulimits.nofile`): 1024 en ambos servicios**, blando ==
    duro en la forma corta de compose. No existía ningún límite de descriptores antes de esta
    tarea. El número es una línea base conservadora acorde al valor por omisión de los runtimes de
    contenedor; los descriptores se gastan en los manejadores de archivos SQLite (`sessions.db`,
    `knowledge_live.db` y los almacenes del sidecar), el socket IPC y las conexiones de red
    salientes. **PROVISIONAL** pendiente de la tarea 16, igual que memoria y CPU.
11. **Los tres límites permanecen parametrizados por célula** (`${HEXCELL_<SERVICIO>_LIMITE_...}`)
    en `deploy/cell.compose.yml`; ningún límite se convierte en literal dentro de la plantilla.
    `deploy/celula.env.ejemplo` es el único referente de los valores decididos.
12. **El guardia mecánico `deploy/verificar_limites.sh` inspecciona el YAML RESUELTO por
    `docker compose --env-file deploy/celula.env.ejemplo config`**, nunca la plantilla cruda, y
    compara mem_limit (en bytes), cpus y ulimits.nofile contra los valores EXACTOS que declara el
    referente. Es de igualdad exacta, no de presencia: detecta también un valor que se desvía en
    silencio del referente, no solo un campo ausente. Está probado por mutación (seis casos, uno
    por límite y servicio) y corre en CI.

## Consecuencias

### Positivas

* Una célula arranca en frío sin preparación externa del host: el volumen nombrado hereda la
  propiedad `10001:10001`/`0700` de la imagen.
* El endurecimiento completo —construcción e imagen por HEX-069, ejecución por HEX-070— queda
  anclado por cuatro guardias mecánicos probados por mutación
  (`deploy/verificar_endurecimiento.sh`, `deploy/verificar_senales.sh`,
  `deploy/verificar_aislamiento_estatica.sh` y `deploy/verificar_limites.sh`), todos en CI. Un
  guardia que nunca se vio fallar no es todavía un guardia.
* Los límites de recursos existen por primera vez a nivel de contenedor —antes no había ni
  memoria, ni CPU, ni descriptores— y son coherentes con NFR-01 sin convertirse en literales de la
  plantilla.
* La comparación de memoria en bytes hace el guardia inmune al formato del env-file: una escritura
  "48m" o "50331648" en el referente produce el mismo resultado.

### Negativas

* **Los valores de memoria, CPU y nofile NO están validados bajo carga.** Son provisionales hasta
  que la tarea 16 mida el consumo real de la célula compuesta y los ajuste; hasta entonces, el
  techo real de células por servidor sigue siendo desconocido (nota de NFR-01 en el PRD). Esta
  tarea no mide nada en vivo y ningún comando de verificación lo exige.
* `alpine:3` no permite retirar el shell por completo: la base mantiene utilidades de BusyBox que
  un atacante con acceso de ejecución podría aprovechar. La mitigación es la combinación con
  `read_only` + `cap_drop` + `no-new-privileges`, que cierra los tres frentes clásicos del ataque
  por contenedor (modificación de rootfs, capabilities, escaladas por setuid).

## Alternativas consideradas y descartadas

La transcripción de esta tarea no tomó decisiones propias, pero el diseño de su guardia evaluó dos
técnicas y descartó una de cada par:

* **Forma explícita `soft`/`hard` de `ulimits.nofile` frente a la forma corta.** Se descartó la
  forma larga con dos valores: la forma corta (un solo escalar fija blando == duro) resuelve a un
  entero plano en `docker compose config` y expresa exactamente la intención —un tope único de
  descriptores, sin una banda elástica que nadie usaría en una célula contenida—. Ver D-52 en
  `docs/bitacora-de-descartes.md`.
* **Comprobación de "campo presente" frente a igualdad exacta contra el referente.** Se descartó
  la presencia: es estrictamente más débil y no detecta un valor que se desvía en silencio del
  referente. Ver D-52.

Las alternativas que estas decisiones reemplazaron —imágenes base no mínimas, usuario por omisión
de `adduser`, bind mount del volumen— están documentadas en los comentarios de los propios
Dockerfiles y de `deploy/cell.compose.yml`, y las condiciones de reapertura del bind mount se
registran en `docs/bitacora-de-descartes.md` y en los comentarios de la plantilla.

## Referencias

* `kitty-specs/hex-069/00-spec.yaml` y `kitty-specs/hex-069/` (imágenes base, UID/GID 10001:10001,
  endurecimiento en tiempo de construcción).
* `kitty-specs/hex-070/00-spec.yaml` y `kitty-specs/hex-070/` (composición de dos contenedores,
  volumen nombrado exclusivo, banderas de ejecución).
* `Dockerfile` y `sidecar/Dockerfile`: comentarios de endurecimiento y retirada del shell.
* `deploy/cell.compose.yml` y `deploy/celula.env.ejemplo`: la plantilla y su referente de valores.
* `deploy/verificar_limites.sh`, `deploy/verificar_endurecimiento.sh`,
  `deploy/verificar_senales.sh`, `deploy/verificar_aislamiento_estatica.sh`: familia de guardias
  mecánicos probados por mutación que anclan estas decisiones.
* `docs/PRD.md`, NFR-01 (techo de 80 MB por célula sobre canal propio) y FR-02 (imágenes mínimas,
  aislamiento por célula).
* `docs/plan/fase-a-6-empaquetado-cli.md`, tareas 4, 5, 6, 8 y 16 (medición pendiente).
* `docs/bitacora-de-descartes.md`, D-52 (técnicas descartadas del guardia de límites).