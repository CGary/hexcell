# Fase A · Etapa 6 — Empaquetado de la célula y CLI de operación

**Duración relativa:** Media.

---

## Objetivo

Hasta aquí existe un núcleo que funciona y un sidecar que habla con WhatsApp, ambos en la máquina del
desarrollador. Esta etapa los convierte en la unidad de despliegue real del producto —**la célula**—
y en algo gobernable desde una línea de comandos.

Una célula sobre canal propio son **dos contenedores**: el núcleo Rust y el sidecar Go, compartiendo
una red local y un volumen. Esa dualidad es la novedad frente al diseño original, y tiene un coste
medible: el sidecar añade unos 15-30 MB de RAM, razón por la cual NFR-01 fija para la Fase A un techo
de 80 MB por célula. Conviene decirlo sin ambigüedad, porque el plan anterior daba a entender lo
contrario: **ese coste es permanente**. El sidecar no desaparece —el canal propio es el canal por
defecto y el canal oficial se incorporará como canal adicional que convive con él—, de modo que los
80 MB son el presupuesto de una célula sobre canal propio, no una holgura transitoria a devolver. Los
50 MB solo aplicarían a una célula que corriera únicamente sobre canal oficial, y el modelo de
densidad del servidor debe dimensionarse sobre 80.

Hay dos requisitos del PRD que solo se pueden verificar de verdad en este punto. El primero es
NFR-01: una medición en el escritorio del desarrollador no significa nada, porque el objetivo de
negocio es alojar decenas de células en un servidor con 8 GB de memoria. El segundo es NFR-05, el
aislamiento estricto de almacenamiento, que exige que una célula **no pueda** acceder al volumen de
otra. Nótese la diferencia entre "no accede" y "no puede acceder": la primera es una convención y la
segunda es una propiedad del sistema. El producto vende privacidad a microempresas que comparten
hardware, así que solo la segunda es aceptable, y demostrarla requiere un intento explícito de
violarla que debe fallar.

La CLI que se construye aquí es deliberadamente parcial. Los comandos de ciclo de vida
—`cell pause`, `cell unpause`, `cell terminate`, `cell rebind`, `cell list`, `cell status`— operan
**solo sobre Docker**. No hay blackholing de Caddy porque no hay Caddy: la desconexión del websocket
saliente ya corta el tráfico entrante, y no queda ninguna petición sin contestar. `cell create`
completo, con subdominios y registro en Meta, pertenece a la etapa B-2.

Uno de esos comandos, `cell rebind`, existe por una razón que no es de comodidad sino de
recuperación. El baneo del número está declarado como **evento esperado** (`adr-0015`), y la salida
de un baneo permanente consiste en volver a emparejar la misma célula con un número distinto. Esa
operación era hasta ahora un procedimiento a mano sobre volúmenes y contenedores, hecho por alguien
con prisa y con un cliente esperando, que es la peor combinación posible para tocar a mano el
almacén donde vive la memoria conversacional. `cell rebind` la convierte en un comando con
confirmación, con orden fijo y con registro.

---

## Alcance

### Qué entra

* `Dockerfile` multi-etapa del **núcleo Rust**, que compila el binario y lo entrega sobre una imagen
  base mínima (Alpine o Scratch), sin cadena de herramientas ni dependencias innecesarias.
* `Dockerfile` multi-etapa del **sidecar Go**, sobre una imagen mínima equivalente, con el binario
  enlazado estáticamente cuando sea posible.
* Compilación con enlazado adecuado a las imágenes base elegidas y perfiles de *release* orientados a
  tamaño.
* Ejecución de ambos procesos como usuario sin privilegios, con sistema de archivos raíz de solo
  lectura salvo el volumen de datos, y sin capacidades de kernel superfluas.
* **Composición de la célula:** los dos contenedores con una red local propia, no accesible desde
  otras células, y un volumen compartido entre ellos que contiene las bases SQLite y las credenciales
  de sesión del sidecar.
* Diseño definitivo del volumen de datos por célula: un volumen dedicado, montado en una única ruta,
  con permisos que impiden el acceso cruzado entre células.
* Límites de recursos por contenedor: memoria, CPU y número de descriptores de archivo, repartidos
  entre núcleo y sidecar dentro del presupuesto de 80 MB por célula.
* Plantilla de composición parametrizada por célula, con las variables de entorno, el volumen, la red
  y los límites ya resueltos.
* Comprobación de salud de la célula apoyada en `GET /health/ready` del núcleo, que incluye el estado
  del enlace con el sidecar.
* Manejo correcto de señales dentro de ambos contenedores, para que el `SIGTERM` de Docker llegue al
  proceso y active el apagado ordenado de la etapa A-2 y el cierre limpio de sesión de la etapa A-3.
* **CLI de operación** en `hexcell-admin`, apoyada exclusivamente en el socket Unix de Docker:
  * `cell pause` — detener el sidecar (cerrando el websocket) y después emitir `SIGTERM` al núcleo con
    30 segundos de gracia. El drenaje del núcleo es **drenaje sin envío** [causa documentada]: las
    tareas en vuelo terminan y persisten su estado, pero **ninguna respuesta pendiente sale** por el
    canal durante una pausa, una migración o una eliminación. Una respuesta que se escapa al reanudar,
    horas después del mensaje que la originó, es justamente el patrón que el TTL de la etapa A-3
    existe para impedir.
  * `cell unpause` — arrancar ambos contenedores; el sidecar reanuda la sesión whatsmeow desde sus
    credenciales en cuanto vive, y la CLI sondea `GET /health/ready` cada 100 ms hasta la primera
    confirmación positiva, que exige pools SQLite operativos **y** sesión de canal activa reportada
    por el sidecar vía IPC (etapas A-2 y A-3).
  * `cell terminate` — cierre de sesión del canal, drenaje por `SIGTERM` de ambos contenedores y
    destrucción física de los volúmenes.
  * `cell rebind` — **re-emparejar una célula existente con un número distinto**, conservando la
    célula y su historia. Es la salida técnica de un baneo permanente, y su regla de conservación es
    exacta: se conservan `sessions.db`, `knowledge_live.db` y el **almacén de identidad del adaptador**
    —donde viven la identidad de conversación y la lista de exclusión (STOP), es decir, la memoria
    del bot por contacto—, y se **descarta el `sqlstore` del sidecar**, que pertenece a un
    dispositivo que ya no existe en el servidor de WhatsApp (`adr-0010`, `adr-0015`). Es una
    operación **destructiva sobre la identidad de canal** de la célula: exige **confirmación
    explícita**, igual que `cell terminate`. Deja además la célula en **pausa de envío hasta que el
    emparejamiento queda confirmado**, para que no intente responder sin sesión, y **registra la
    sustitución de forma auditable**: número anterior, fecha absoluta y motivo. Es un comando de la
    **Fase A**: nace de la operación del canal propio y no depende de nada de la Fase B.
  * `cell list` y `cell status` — estado consolidado de cada célula, incluida la salud del canal.
* Registro persistente del estado de cada célula en el plano de control, para que la CLI no dependa
  exclusivamente de inferir el estado a partir de Docker.
* **Alertas push por bot de Telegram** ante **ocho** condiciones: **baneo temporal detectado**, sesión
  desvinculada, sidecar sin reconectar durante más de 5 minutos, bucle de reinicios, saldo LLM
  agotado o modo degradado, tasa de descartes GCRA anómala, descarte de un envío no solicitado
  (violación del invariante de solo-responder de la etapa A-3) y **caída anómala del ratio de acuses
  de entrega segmentado por contacto**.
* **Métricas por célula** que alimentan esas alertas y el diagnóstico posterior: ratio de acuses de
  entrega **por contacto** y latencia hasta el acuse, **reconexiones por hora** y **ventana de
  silencio entrante** (cero mensajes recibidos en X horas hábiles cuando históricamente hay tráfico).
  Las emite el sidecar (etapa A-3); aquí se recogen, se comparan contra umbral y se entregan.
* **Canary de biblioteca y despliegue escalonado.** Una **célula centinela propia**, con **número
  propio** de HexCell y ningún cliente encima, corre la versión candidata de whatsmeow durante
  **72 horas** antes de que la actualización se escalone al resto de la cartera. **Nunca se actualizan
  todas las células el mismo día.** El pinneado por commit y la ventana de actualización los fija la
  etapa A-3; el escalonado se ejecuta desde aquí, porque es aquí donde viven el empaquetado y el
  despliegue.
* **Dead-man's switch externo** (healthchecks.io, capa gratuita): ping cada 5 minutos desde un `cron`
  local, con notificación desde fuera del servidor cuando el ping deja de llegar.
* Idempotencia y recuperación: cada comando debe poder reejecutarse tras un fallo parcial y dejar el
  sistema en el estado pretendido.
* Medición formal del consumo de memoria de la célula completa en reposo y bajo carga.
* Publicación de ambas imágenes desde la CI, versionadas de forma reproducible.

### Qué NO entra

* Caddy, subdominios, certificados y blackholing: etapa B-2.
* `cell create` con alta de subdominio y registro en Meta: etapa B-2. El alta de las células piloto de
  la Fase A se hace en la etapa A-7 con un procedimiento más simple.
* Orquestadores de clúster. El PRD fija un servidor local único; introducir Kubernetes o similares
  contradice el objetivo de eficiencia.
* Cualquier interfaz gráfica de administración.
* El panel de métricas, la agregación por servidor y el resto de la observabilidad de operación:
  etapa B-3. Aquí solo se adelanta el mínimo de alertado que exige tener clientes reales.

### Requisitos del PRD cubiertos

* **FR-02** — aislamiento completo por célula en contenedores dedicados sobre imágenes mínimas.
* **FR-11** — operaciones CLI de suspensión y reactivación, en su variante de Fase A (sin Caddy).
* **NFR-01** — techo de 80 MB de RAM por célula en reposo para la Fase A, verificado por medición.
* **NFR-05** — aislamiento estricto de almacenamiento entre células, verificado por intento de
  violación.

---

## Entregables

* `Dockerfile` del núcleo y `Dockerfile` del sidecar, con sus `.dockerignore`.
* `deploy/cell.compose.yml` (o especificación equivalente) parametrizada por célula, con los dos
  contenedores, la red local y el volumen compartido.
* `hexcell-admin` con los comandos `cell pause`, `cell unpause`, `cell terminate`, `cell rebind`,
  `cell list` y `cell status`.
* Registro auditable de sustituciones de número por célula —número anterior, fecha absoluta y
  motivo—, alimentado por `cell rebind` y consultable desde `cell status`.
* Módulo cliente del socket Unix de Docker.
* Almacén de estado del plano de control con su esquema y migraciones.
* `docs/adr/adr-0007-imagen-y-aislamiento.md` documentando las imágenes base elegidas, la
  composición de dos contenedores, el modelo de permisos del volumen y los límites de recursos.
* Módulo de alertas con el cliente del bot de Telegram y las **ocho** condiciones que las disparan,
  con su orden de prioridad declarado.
* Recolección de las métricas por célula —acuses por contacto, reconexiones por hora y ventana de
  silencio entrante— con sus umbrales configurables y marcados como valores a calibrar.
* Procedimiento de **canary de biblioteca y despliegue escalonado**, con la célula centinela dada de
  alta y su número propio.
* Configuración del dead-man's switch y la entrada de `cron` que lo alimenta.
* `docs/runbook-operacion.md`: manual breve de operación con los comandos y sus efectos, incluida la
  respuesta ante cada alerta.
* Script de medición de memoria y de tamaño de imagen, ejecutable de forma repetible.
* Prueba automatizada de aislamiento: una célula intenta leer el volumen de otra y falla.
* Trabajo de CI que construye y publica ambas imágenes etiquetadas.

---

## Orden de ejecución (revisado 2026-09-10)

Las entradas conservan su numeración; esta sección es la autoridad sobre el orden.

`8 → 4 → 5 → 7 → 17 → 6 → 16 → 9 → 10 → 11 → 22 → 24 → 12 → 13 → 14 → 15 → 18 → 20 → 23 → 21 → 19`

* **8 primero:** la plantilla fija `HEXCELL_DIRECCION_SALUD` antes de que ningún contenedor hermano sondee la salud (el valor por omisión es loopback, `crates/hexcell/src/configuracion.rs:340-348`).
* **17 junto a 5:** la prueba de aislamiento es el criterio de aceptación de la composición.
* **6 antes de 16, con ajuste posterior:** 6 fija límites provisionales desde NFR-01, 16 mide bajo esos límites y 6 se ajusta con el dato.
* **14 después de 13:** `cell status` incluye el historial de sustituciones, que solo existe tras `rebind`.
* **12 y 13 tras la tarea 24:** no se implementa un `terminate` que borre volúmenes con la sesión viva.
* Actualización 2026-09-11: 8, 4, 5, 9 y 24 cerradas; 25 dividida en 25-a (cerrada) y 25-b (pendiente, antes de 20). La cadena restante: 7 → 17 → 6 → 16 → 10 → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 25-b → 20 → 23 → 21 → 19.
* Actualización 2026-09-13: 7 cerrada (HEX-075). La cadena restante: 17 → 6 → 16 → 10 → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 25-b → 20 → 23 → 21 → 19.
* Actualización 2026-09-13: 25-b cerrada (HEX-072-b), con lo que la tarea 25 queda cerrada por completo y la 20 deja de estar bloqueada. La cadena restante: 17 → 6 → 16 → 10 → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 20 → 23 → 21 → 19.
* Actualización 2026-09-13: 10 dividida en 10-a (HEX-074-a, cerrada) y 10-b (pendiente). La cadena restante: 17 → 6 → 16 → 10-b → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 20 → 23 → 21 → 19.
* Actualización 2026-09-13: 17 cerrada (HEX-076). La cadena restante: 6 → 16 → 10-b → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 20 → 23 → 21 → 19.
* Actualización 2026-09-13: 10-b cerrada (HEX-074-b) con el analizador y los subcomandos desplazados a 10-c (HEX-074-c). 20 dividida: 20-a (HEX-077-a), 20-c (HEX-077-c) y 20-d (HEX-077-d) cerradas; 20-b (HEX-077-b) pendiente. La cadena restante: 6 → 16 → 10-c → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 20-b → 23 → 21 → 19.
* Actualización 2026-09-13: 6 cerrada (HEX-078) con el reparto de memoria 48m/32m, CPU 0.5/0.25 y nofile 1024 —los tres PROVISIONALES hasta la medición de la tarea 16— y el guardia `deploy/verificar_limites.sh` (probado por mutación, en CI). La cadena restante: 16 → 10-c → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 20-b → 23 → 21 → 19.
* Actualización 2026-09-14: 10-c cerrada (HEX-074-c) y con ella la tarea 10 completa: analizador de argumentos a mano sobre `std::env::args`, los seis subcomandos `cell` con validación, modo de simulación sin efectos laterales y `src/main.rs` cableado —ya no imprime el talón de A-1—, sobre el contrato de códigos de salida de 10-b. Registrados `adr-0036` (gramática, tabla de desenlaces y modo de simulación) y D-53 (descarte de `clap`, `argh`, `pico-args` y `structopt`). La persistencia del estado del plano de control, la idempotencia, la reconciliación contra Docker y el registro de sustituciones de `cell status` siguen diferidos a las tareas 11 a 15. La cadena restante: 16 → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 20-b → 23 → 21 → 19.
* Actualización 2026-09-14: 20-b cerrada (HEX-077-b) y con ella la tarea 20 completa; la octava condición (bucle de reinicio) queda diferida en D-54 y `adr-0037`, no pendiente en la cadena. La cadena restante: 16 → 11 → 22 → 12 → 13 → 14 → 15 → 18 → 23 → 21 → 19.
* Actualización 2026-09-21: 22 cerrada (HEX-081) con el grupo `config render` de `hexcell-admin` —lista blanca cerrada de claves no secretas, superposición sobre valores por defecto en KEY=VALUE y validación de fallo cerrado en el render, nunca en el arranque— más los dos archivos de ejemplo bajo `deploy/` y la guarda `deploy/verificar_renderizado_configuracion.sh` (probada por mutación, en CI). Registrados `adr-0038` (segundo grupo `config render`) y D-56 (descarte de un analizador externo para la configuración). Los secretos siguen viajando solo por variable de entorno y los overlays con valores reales son datos de cliente: en este repositorio solo entran ejemplos con marcadores. La cadena restante: 16 → 11 → 12 → 13 → 14 → 15 → 18 → 23 → 21 → 19.
* Actualización 2026-09-21: 11 cerrada (HEX-080); ver su nota de cierre. Pendiente mecánico fuera de la cadena, sin tarea numerada: los tests del workspace no se lintean en CI (`cargo clippy --workspace` sin `--all-targets`) y con la bandera fallan 38 diagnósticos preexistentes; se limpian y se activa la bandera en `ci.yml` en el mismo commit. La cadena restante: 12 → 13 → 14 → 15 → 18 → 23 → 21 → 19.
* Actualización 2026-09-19: 16 cerrada (HEX-079) con el instrumento en vivo `deploy/medir_memoria_y_imagenes.sh`; la corrida manual real y el registro de los números quedan diferidos (AC-6), en la sección de valores de referencia de `docs/plantilla-celula.md`. La cadena restante: 11 → 22 → 12 → 13 → 14 → 15 → 18 → 23 → 21 → 19. Nota 2026-09-21: el instrumento da una cota inferior, no el peor caso (no ejercita GCRA, inferencia ni la ruta whatsmeow, D-55); ni su corrida manual ratifica el reparto provisional 48/32 MB de `adr-0007`, que sigue provisional hasta la prueba de carga sostenida pendiente en STATUS. Primera corrida manual el 2026-09-21 (commit `5d03a19`, sin dispositivo emparejado): en reposo 9,0 MiB de `anon` y 35,7 MiB de `memory.current` agregados; imágenes de 11,3 MiB (núcleo) y 29,0 MiB (sidecar); tabla en `docs/plantilla-celula.md`.
* Actualización 2026-09-22: 14 cerrada (HEX-083) con el almacén SQLite del plano de control (`adr-0039`), la validación de transiciones antes de Docker, la persistencia sólo tras éxito, y los comandos `cell status` y `cell list` reales. **Desviación consciente del orden declarado arriba:** la restricción «14 después de 13» se apoyaba en que el historial de sustituciones de `cell status` sólo existe tras `rebind`, y la tarea 13 sigue abierta, así que 14 se cerró ANTES de 13. La desviación es legítima porque esta tarea crea la tabla `sustituciones` y sólo la LEE: un historial vacío no es una carencia de `cell status` sino el estado real de una célula que nunca fue reemparejada, y así se imprime («sustituciones: (ninguna)»). La tarea 13 es la que escribe en esa tabla y, al cerrarse, llenará el historial sin tocar el lector. El gancho `Retirada`/`sesion_cerrada` queda inerte porque la tarea 12 no está fusionada. Registrados `adr-0039` (almacén del plano de control) y D-58 (descarte de un crate de migraciones). La cadena restante: 12 → 13 → 15 → 18 → 23 → 21 → 19.
* Actualización 2026-09-22: 23 cerrada (HEX-084) con el grupo `reporte tokens` de `hexcell-admin` —agrega la fórmula literal de la vista `consumo_por_conversacion` (migración 0004) sobre una copia `VACUUM INTO` de `sessions.db` abierta en solo lectura, con ventana opcional por `resuelta_ms` (`--desde` inclusivo, `--hasta` exclusivo) y modo `--simular`—. La alternativa de agregar los registros estructurados en vez de la copia queda registrada como no implementada en la nota de cierre de la tarea 23, no en la bitácora de descartes. La cadena restante: 12 → 13 → 15 → 18 → 21 → 19.
* Actualización 2026-09-22: 12 cerrada (HEX-082) con la CLI de `cell terminate` —secuencia destructiva de seis pasos con cierre de sesión mediante contenedor hermano—, dividida en dos hijos: HEX-082-a (ruta del núcleo `POST /admin/sesion/cierre`) y HEX-082-b (CLI y pruebas). Con la tarea 14 (HEX-083) ya fusionada, el gancho `Retirada`/`sesion_cerrada` deja de estar inerte: `cell terminate` persiste esa transición contra el almacén del plano de control (ver el párrafo de cierre de la tarea 12). La cadena restante, con 12, 14 y 23 ya cerradas: 13 → 15 → 18 → 21 → 19.
* Actualización 2026-09-22: 13 cerrada (HEX-085) con `cell rebind` real sobre Docker; ver su nota y su párrafo de cierre. La cadena restante: 15 → 18 → 21 → 19.

---

## Tareas

1. **Escribir el `Dockerfile` del núcleo** (1 día). Etapa de compilación con la cadena de
   herramientas y etapa final mínima con solo el binario y sus datos.
2. **Escribir el `Dockerfile` del sidecar** (0,5 días). Compilación Go y entrega sobre imagen mínima,
   con la versión de whatsmeow fijada de forma visible en la etiqueta de la imagen.
3. **Resolver el enlazado y minimizar los binarios** (1 día). Ajustar los objetivos de compilación a
   las imágenes base, activar las optimizaciones de tamaño y eliminar símbolos innecesarios.
4. **Endurecer ambos contenedores** (1 día). Usuario sin privilegios, raíz de solo lectura,
   eliminación de capacidades no necesarias y ausencia de shell si la imagen base lo permite.
5. **Componer la célula** (1 día). Red local propia por célula, volumen compartido entre núcleo y
   sidecar con los permisos correctos, y socket IPC dentro del volumen. Verificar que ninguna célula
   alcanza la red de otra.

   **Alcance añadido (decidido 2026-09-10):** esta tarea es además la **dueña de las banderas de
   ejecución** del endurecimiento —`read_only`, `cap_drop: [ALL]`, `no-new-privileges` y el `tmpfs`
   de la ruta de escritura temporal— y del **modelo de propiedad del volumen**. La tarea 4 endurece
   las dos imágenes en tiempo de construcción (usuario numérico `10001:10001`, `/var/lib/hexcell` en
   modo `0700`, sin shell) y deja esas banderas **nombradas en un bloque de comentario pero no
   impuestas**; la tarea 8 las excluye explícitamente de su alcance. Sin este anclaje el
   endurecimiento entraría en las imágenes y nunca se encendería en ejecución. Dato medido el
   2026-09-10 que condiciona la plantilla: un volumen **nombrado** vacío hereda dueño y modo del
   directorio de la imagen, mientras que un **bind mount** no lo hace y falla con `Permission
   denied`.
6. **Fijar los límites de recursos** (0,5 días). Memoria, CPU y descriptores por contenedor, con el
   reparto entre núcleo y sidecar coherente con el techo de 80 MB por célula.

   **Cerrada el 2026-09-13 con HEX-078**: reparto de memoria fijado en 48m al núcleo + 32m al
   sidecar = 80m exactos (NFR-01), CPU en 0.5/0.25 y `ulimits.nofile` en 1024 para ambos, todo
   parametrizado por célula en `deploy/cell.compose.yml` con los valores en
   `deploy/celula.env.ejemplo`. Los tres valores son PROVISIONALES y se confirman o corrigen con la
   medición de la tarea 16 ("6 antes de 16, con ajuste posterior"); guardia mecánico
   `deploy/verificar_limites.sh` (probado por mutación en sus seis casos, en CI) que ancla los
   límites sobre el YAML resuelto con igualdad exacta contra el referente. El procedimiento de
   respuesta ante un `OOMKilled` de estos límites queda diferido a la tarea 21.
7. **Verificar la propagación de señales** (0,5 días). Comprobar que `docker stop` con margen de 30
   segundos produce el apagado ordenado del núcleo y el cierre limpio de sesión del sidecar, con
   salidas con código 0 y sin recurrir a `SIGKILL`.

   **Cerrada el 2026-09-13 con HEX-075**: `STOPSIGNAL SIGTERM` anclado en ambos Dockerfiles y
   `stop_grace_period: "30s"` en ambos servicios de `deploy/cell.compose.yml`; guardia mecánico
   `deploy/verificar_senales.sh` (probado por mutación, en CI) más el script manual en vivo
   `deploy/verificar_apagado_ordenado.sh`, corrido contra contenedores reales desde un volumen
   vacío: ambos contenedores salieron con código 0 dentro del margen, sin `SIGKILL`, con el
   checkpoint del WAL confirmado. No se encontró ningún defecto en el manejo de señales a nivel de
   proceso (`crates/hexcell/src/apagado.rs`, `sidecar/main.go`), que ya era correcto.
8. **Parametrizar la plantilla de arranque por célula** (1 día). Todo lo que distingue a una célula de
   otra pasa a ser configuración: identificador, volumen, red, secretos y límites.
9. **Implementar el cliente del socket Unix de Docker** (1,5 días). Arranque, parada con margen,
   inspección, eliminación de contenedores y de volúmenes, con manejo explícito de errores.
10. **Construir el esqueleto de la CLI y el modelo de estado** (1 día). Analizador de argumentos,
    salida legible, códigos de retorno significativos, modo de simulación, y estados posibles de una
    célula con sus transiciones válidas.
    **Dividida el 2026-09-13.** 10-a (HEX-074-a, cerrada): agregado de estado de célula del plano de
    control (`CicloDeVidaDeCelula`, `crates/hexcell-admin/src/estado_de_celula.rs`), solo en memoria y
    sin dependencias nuevas. 10-b (HEX-074-b, cerrada el 2026-09-13; alcance real abajo): analizador de argumentos, subcomandos, códigos
    de retorno, modo de simulación y cableado de `src/main.rs`; la persistencia del plano de control
    queda diferida a la tarea de A-6 que la decida.
    **Actualización 2026-09-13.** 10-b (HEX-074-b) cerró con menos alcance del anotado: entregó
    `codigo_de_salida.rs`, `salida.rs` y el cliente Docker (`src/docker/`) sobre `lib.rs`; el
    analizador de argumentos, los subcomandos y el cableado de `src/main.rs` pasan a 10-c
    (HEX-074-c).
    **Cerrada por completo el 2026-09-14** con 10-c (HEX-074-c): analizador a mano sobre
    `std::env::args`, los seis subcomandos `cell` con validación de argumentos, modo de simulación sin
    efectos laterales y `src/main.rs` cableado —ya no imprime el talón de A-1—, con `adr-0036` y D-53
    registrados.
11. **Implementar `cell pause` y `cell unpause`** (1,5 días). Orden explícito en la pausa —primero el
    sidecar, después el núcleo— y sondeo de disponibilidad cada 100 ms con límite temporal y mensaje
    de error claro si nunca llega a estar lista.

    **Criterio de aceptación (revisado 2026-09-10):** Sondeo de `/health/ready` desde un
    contenedor hermano dentro de la red de la célula, nunca desde el anfitrión, con
    `HEXCELL_DIRECCION_SALUD` fijado en la plantilla de la tarea 8. Se acepta cuando `cell unpause`
    devuelve 0 solo tras un 200 y devuelve un código distinto de 0 con mensaje explícito al agotar el
    límite de tiempo.

    **Cerrada el 2026-09-21 con HEX-080.** `cell pause` detiene ambos contenedores con `stop` sin plazo
    explícito (rige el `stop_grace_period` de la plantilla), sidecar primero y completamente detenido
    antes de señalar al núcleo: el estado Docker final es `exited`, no `paused`, y la invariante «nada
    sale durante la pausa» la garantiza ese orden. `cell unpause` arranca ambos y sondea `/health/ready`
    cada 100 ms desde un contenedor hermano con límite de 60 s. La cláusula «el estado pasa a
    `Suspendida`» del criterio original se **traslada a la tarea 14**, que es la que crea el almacén del
    plano de control; sin almacén el estado es incognoscible desde un proceso que termina. La CLI no
    abre conexión IPC con el sidecar (D-57). Deja un seguimiento para la tarea 15: el cliente Docker
    tiene dos operaciones de parada sobre el mismo endpoint (`detener_contenedor` con `t=30`, ya sin
    llamadores en producción, y la variante sin plazo).
12. **Implementar `cell terminate`** (1 día). Cierre de sesión del canal desvinculando el dispositivo,
    drenaje de ambos contenedores, borrado físico de volúmenes incluidas las credenciales, y
    confirmación explícita requerida por tratarse de una operación destructiva.

    **Criterio de aceptación (revisado 2026-09-10):** Se acepta cuando `cell terminate` desvincula el
    dispositivo mediante el tipo IPC de cierre de sesión (tarea 24) y el estado transita a
    `desvinculada_sesion_cerrada`. Desbloqueada el 2026-09-11 por HEX-071 (tarea 24): el tipo IPC de
    cierre de sesión existe.

    **Cerrada el 2026-09-22 con HEX-082.** `cell terminate --id <id> --confirmar` ejecuta la secuencia
    destructiva de seis pasos: inspeccionar núcleo y sidecar, POST `/admin/sesion/cierre` mediante un
    contenedor hermano en la red de la célula, detener sidecar y núcleo (sin plazo explícito, rige el
    `stop_grace_period`), eliminar ambos contenedores y el volumen de datos. El nombre del volumen se
    lee de `docker inspect` (Mounts[].Name), nunca se deriva del `--id`. El estado objetivo es
    `Retirada` con motivo `sesion_cerrada`, y se persiste desde la fusión de HEX-082 porque HEX-083
    (tarea 14) entró antes en main y ya trae el almacén del plano de control. La superficie de administración cambia con la ratificación R1: la plantilla de
    célula expone el listener de administración en la red interna de la célula (`HEXCELL_DIRECCION_ADMIN`),
    de modo que el contenedor hermano puede alcanzar la ruta de cierre de sesión.
13. **Implementar `cell rebind`** (1 día). Re-emparejamiento de una célula existente con un número
    distinto, que es la salida técnica de un baneo permanente y no un alta nueva. Secuencia fija:
    confirmación explícita del operador —es una operación destructiva sobre la identidad de canal,
    con la misma exigencia que `cell terminate`—; **pausa de envío** de la célula, que se mantiene
    hasta que el emparejamiento queda confirmado, para que no intente responder sin sesión;
    **descarte del `sqlstore`** del sidecar, que corresponde a un dispositivo muerto y no se
    restaura nunca desde respaldo en este escenario; **conservación intacta de `sessions.db`, de
    `knowledge_live.db` y del almacén de identidad del adaptador**, donde viven la identidad de
    conversación y la lista de exclusión (STOP), y destino declarado explícitamente (conservar o
    regenerar) de `identidad.db` y `outbox.db` del sidecar; emparejamiento con el número nuevo por
    `orden_emparejar` en modo `qr` o `codigo_de_vinculacion`; y **anotación auditable de la
    sustitución** con el número anterior, la fecha absoluta y el motivo. El comando pertenece a la
    **Fase A** y no toca Caddy ni nada de la Fase B.

    **Criterio de aceptación (revisado 2026-09-10):** Emparejamiento por `orden_emparejar` en modo `qr` o
    `codigo_de_vinculacion` (no existe `PairPhone()` en el adaptador). Pausa de envío ejercida por la
    orden IPC de la tarea 24. Conservación verificada por checksum de `sessions.db`,
    `knowledge_live.db` y `adapter_identity.db`; el destino de `identidad.db` y `outbox.db` del
    sidecar queda declarado explícitamente en la tarea (conservar o regenerar).

    **Nota 2026-09-22.** La anotación auditable de la sustitución NO guarda el número anterior ni
    ningún identificador de transporte: `adr-0039` fija que el almacén del plano de control sólo
    guarda el id de célula, y `sessions.db` nunca guarda identificadores crudos; la fila de
    `sustituciones` lleva id de célula, motivo (`--motivo`) y fecha absoluta en milisegundos. El
    cierre de sesión previo al descarte es **a mejor esfuerzo**: un `fallido` se escribe en stderr y
    la secuencia continúa, porque tras un baneo el cierre suele fallar. Destino declarado de las
    bases del sidecar: se descarta sólo `sqlstore.db` (con su `-wal` y `-shm`) y se **conservan**
    `identidad.db` (grafo de contactos con la lista STOP, separado a propósito del dispositivo) y
    `outbox.db` (envíos debidos). Una célula que quedó en `Reemparejando` por un fallo o por un
    código expirado se **reanuda** con el mismo `cell rebind`, que salta directamente al
    emparejamiento. Limitación conocida: la reanudación tras un `reanudar` fallido, o tras un fallo
    entre la persistencia de `Reemparejando` y el arranque del sidecar (pasos 5 a 7), reentra en el
    paso de emparejamiento y puede recibir `ya_emparejada`, que termina en `Fallo`; no hay todavía
    ruta de recuperación definida (decisión pendiente en STATUS).

    **Cerrada el 2026-09-22 con HEX-085.** `cell rebind --id <id> --motivo <texto> --confirmar
    [--metodo qr|codigo_de_vinculacion]` ejecuta la secuencia fija sobre Docker —pausa de envío,
    cierre de sesión a mejor esfuerzo, persistencia de `Reemparejando`, descarte del `sqlstore`
    mediante contenedor hermano, reinicio del sidecar con la pausa reaplicada, emparejamiento,
    sondeo de la sesión hasta `activa`, reanudación del envío y persistencia de `EnEjecucion` con
    motivo `emparejamiento_confirmado` más la fila de `sustituciones`—, dividida en dos hijos:
    HEX-085-a (rutas de sesión del núcleo y emparejamiento real en el adaptador whatsmeow) y
    HEX-085-b (CLI y pruebas). Prueba de humo en Docker real con una célula de canal simulado
    arrancada desde volumen vacío: checksums de `sessions.db`, `knowledge_live.db` y
    `adapter_identity.db` idénticos antes y después.
14. **Implementar `cell list` y `cell status`** (0,5 días). Se acepta cuando `cell status` cruza el
    almacén de plano de control, `docker inspect` y `/health/ready`, marca cada discrepancia con un
    código estable e incluye el historial de sustituciones. No reporta ratio de acuses ni ventana de
    silencio: eso es la tarea 20.

    **Nota 2026-09-21:** hereda de la tarea 11 la cláusula «tras `cell pause` el estado de la célula
    pasa a `Suspendida` y tras `cell unpause` vuelve a `EnEjecucion`», porque esta es la primera tarea
    que necesita el almacén del plano de control y por tanto la que lo crea. La tabla de transiciones
    ya existe en `crates/hexcell-admin/src/estado_de_celula.rs` (HEX-074-c); lo que falta es persistirla.

    **Cerrada el 2026-09-22 con HEX-083.** Almacén SQLite del plano de control con migración
    versionada por `PRAGMA user_version` (`adr-0039`), ruta configurable por variable de entorno
    `HEXCELL_ADMIN_ALMACEN`, validación de transiciones antes de Docker y persistencia sólo tras
    éxito. `cell status` cruza las tres fuentes (almacén, Docker, sonda de salud) y reporta los
    cinco códigos de discrepancia (DISC-01 a DISC-05). `cell list` imprime la unión de células del
    almacén y de Docker. El gancho de transición `Retirada` con motivo `sesion_cerrada` queda
    declarado pero inerte porque la tarea 12 no está fusionada en main. La tabla `sustituciones` se
    crea pero sólo se lee; la tarea 13 (cell rebind) es la que escribe en ella. Registrados
    `adr-0039` (almacén del plano de control) y D-58 (descarte de un crate de migraciones).

    **Seguimiento 2026-09-22 (HEX-083):** la revisión dejó tres asimetrías de prueba sin cerrar,
    trasladadas a la tarea 15: la respuesta 500 de Docker sólo se prueba para el contenedor del
    núcleo, DISC-05 no tiene el caso del par a medias (un contenedor presente y el otro ausente) y
    DISC-03 no tiene el caso de sonda inalcanzable. El gancho `terminate → Retirada` se cablea en la
    fusión de HEX-082 (tarea 12), no en una tarea nueva.

    Actualización 2026-09-22: gancho terminate → Retirada cableado en HEX-082.
15. **Dotar de idempotencia y recuperación a los comandos** (1 día). Reejecución segura tras un fallo
    parcial, con detección del punto en que quedó la secuencia.

    **Nota 2026-09-21:** al tocar el cliente Docker, fundir `detener_contenedor` (con `t=30`, sin
    llamadores en producción desde HEX-080) y `detener_contenedor_sin_plazo` en una sola operación con
    plazo opcional, y trasladar la prueba correspondiente de HEX-074-b.

    **Nota 2026-09-22:** hereda de la tarea 14 (HEX-083) tres pruebas de ruta de fallo que faltan en
    `cell status`: respuesta 500 de Docker también para el sidecar, DISC-05 con el par de contenedores
    a medias y DISC-03 con la sonda inalcanzable. Son rutas de recuperación, que es lo que esta tarea
    endurece.

    **Seguimiento 2026-09-22:** variante de `cell terminate` para dispositivos ya baneados, donde no
    hay sesión que cerrar (el sidecar no puede contactar con el servidor de WhatsApp). La ruta de
    cierre devolvería 502 o la sonda saldría con código distinto de cero, abortando la secuencia sin
    destruir nada. Se estudiará una variante forzada del cierre que omita ese paso cuando el
    dispositivo ya está baneado, sin añadir una nueva bandera al contrato existente sino como
    extensión del comportamiento ante un fallo específico del cierre; el mecanismo concreto queda
    pendiente de la tarea 15.
16. **Medir memoria y tamaño de imágenes** (0,5 días). Consumo de la célula completa en reposo y bajo
    carga, y peso de ambas imágenes, registrados como valores de referencia.
    No se reutiliza `rss_linea_base` (mide solo el núcleo con adaptador simulado).

    **Criterio de aceptación (revisado 2026-09-10):** Medir sobre la célula compuesta de la tarea 5
    con adaptador whatsmeow y bajo los límites de cgroup de la tarea 6: RSS agregado de ambos
    contenedores leído de cgroup v2, no de `/proc` del anfitrión, en reposo y bajo la carga de
    `crates/hexcell/tests/carga.rs`, más el peso de ambas imágenes. `rss_linea_base` no se reutiliza
    (mide solo el núcleo con adaptador simulado). No se añade compuerta de tamaño en CI (decisión de
    HEX-067).

    **Cerrada el 2026-09-19 con HEX-079**: instrumento de medición en vivo
    `deploy/medir_memoria_y_imagenes.sh`, que levanta la célula compuesta de la tarea 5 con el
    adaptador whatsmeow bajo los límites de la tarea 6, lee la memoria agregada de ambos contenedores
    desde cgroup v2 en reposo y bajo un generador declarado, y mide ambas imágenes con `docker image
    inspect`. `carga.rs` no se reutiliza ni se modifica como generador externo (limitación declarada
    en el script y descarte registrado como D-55); la cifra bajo carga es una cota inferior. La
    sección «Valores de referencia de memoria y tamaño de imágenes» de `docs/plantilla-celula.md`
    queda lista y vacía para la corrida manual posterior (AC-6). `rss_linea_base` no se toca.
17. **Escribir la prueba de aislamiento** (1 día). Levantar dos células y demostrar que ninguna puede
    leer ni escribir el volumen de la otra ni alcanzar su red, ni siquiera conociendo la ruta.

    **Cerrada el 2026-09-13 con HEX-076**: guardia mecánico
    `deploy/verificar_aislamiento_estatica.sh` (probado por mutación, en CI) que ancla, sobre el
    YAML resuelto de `deploy/cell.compose.yml`, que cada célula declara su propia red y su propio
    volumen nombrado con los nombres exactos del referente y que ningún servicio publica un puerto
    al host; más el script manual en vivo `deploy/verificar_aislamiento.sh`, que levanta dos células
    reales (A y B) desde volúmenes vacíos y ejerce el cruce de volumen (lectura y escritura), el
    alcance de red al núcleo y al sidecar ajenos (por nombre de contenedor y por IP cruda) y el
    socket IPC ajeno. No se encontró ningún hallazgo que corrigiera `deploy/cell.compose.yml`: la
    plantilla ya declaraba red y volumen per-célula y ningún `ports:` desde HEX-070. El
    script en vivo se corrió de punta a punta contra contenedores reales el 2026-09-13: las once
    aserciones pasan. Esa primera corrida encontró un defecto en la propia prueba —AC-8 comparaba
    solo el número de dispositivo de ambos sockets, que dos volúmenes nombrados comparten siempre,
    de modo que la aserción era una guarda invertida imposible de pasar—; se corrigió al par
    dispositivo:inodo y quedó registrado como D-49.
18. **Integrar la construcción de las imágenes en la CI** (1 día). Construcción reproducible,
    etiquetado por versión y por commit, y publicación en el registro elegido.
    * Guarda en CI que falla si la imagen corre como root o sin rootfs de solo lectura (criterio ya
      enunciado en esta etapa, hoy sin comprobación mecánica).
19. **Montar el canary de biblioteca y el despliegue escalonado** (1 día). Alta de una **célula
    centinela** propia, con número propio de HexCell y sin ningún cliente encima, que corre la
    versión candidata de whatsmeow durante **72 horas** antes de que la actualización toque a nadie
    más. Después, escalonado por lotes de la cartera, con parada si el lote anterior presenta baneos,
    desconexiones anómalas o `Client outdated (405)`. Queda escrito como prohibición operativa:
    **nunca actualizar todas las células el mismo día**. La centinela es además el sitio donde se
    ensayan medidas cuya eficacia no está probada —el experimento con Meta Verified, entre ellas—,
    porque es el único número cuyo baneo no le cuesta el negocio a nadie.
20. **Implementar alertas push, métricas por célula y el dead-man's switch** (1,5 días). Tres piezas
    complementarias:
    * **Alertas activas** por bot de Telegram, con una simple llamada HTTP saliente desde el
      servidor, ante **ocho** condiciones. La primera va aparte por prioridad: **baneo temporal
      detectado**, con su fecha de expiración, que es **alerta de máxima prioridad** por ser el
      **único aviso previo que suele existir**; cualquier otra alerta puede esperar a la mañana
      siguiente, esta no. Las siete restantes: sesión de canal desvinculada, sidecar sin reconectar
      durante más de 5 minutos, bucle de reinicios de cualquiera de los dos contenedores, saldo LLM
      agotado o entrada en modo degradado, tasa de descartes GCRA anómala, descarte de un envío no
      solicitado (violación del invariante de solo-responder), y **caída anómala del ratio de acuses
      de entrega segmentado por contacto**. Esta última es la **detección indirecta de bloqueos de
      usuarios**: el bloqueo no se notifica, pero cuando un contacto bloquea el número **cesan sus
      acuses de entrega**; por eso el ratio se segmenta por contacto y **nunca se mira en agregado**,
      donde el efecto se diluye hasta desaparecer. Las señales del canal, del invariante y de los
      acuses las emite el sidecar (etapa A-3); las del saldo y los descartes GCRA, el núcleo (etapa
      A-4). Esta tarea las **entrega**.
    * **Alertas activas** — **Siete de las ocho condiciones entregadas con HEX-077-b**
      (2026-09-14, `adr-0037`). La condición de bucle de reinicios de contenedores queda para una
      tarea futura (D-54) porque no existe ningún productor de señal para ella en el repositorio.
    * **Métricas por célula**: reconexiones por hora y ventana de silencio entrante —cero mensajes
      recibidos en X horas hábiles cuando históricamente hay tráfico—, además de la latencia hasta el
      acuse. Los umbrales quedan como parámetros a calibrar con datos reales, no como constantes
      elegidas de antemano.
    * **Dead-man's switch externo** con healthchecks.io en su capa gratuita: un `cron` local hace
      ping cada 5 minutos y **la ausencia de ping** dispara la notificación desde fuera del servidor.
      Es la única clase de alerta que sobrevive al fallo que más importa: **un servidor muerto no
      puede avisar de que ha muerto**, así que la vigilancia tiene que vivir en otro sitio.

    > **Lo que NO es observable.** Cuántos usuarios han reportado el número. **Esa señal no existe**,
    > por ninguna vía, y ningún panel ni ninguna alerta de este plan debe fingir que la tiene. Los
    > reportes son una de las tres familias de señales con las que Meta decide, y llegan a nuestro
    > lado únicamente como consecuencia consumada: un baneo.

    > **Lo que esto NO hace.** La observabilidad **acorta el tiempo de reacción; no evita el baneo**.
    > Ninguna alerta de esta lista reduce la probabilidad de que Meta desactive un número: el riesgo
    > es en buena medida estructural. Y el **baneo permanente suele llegar sin aviso previo** —el
    > baneo temporal es el único que a veces lo da—, de modo que el valor de esta tarea es enterarse
    > en minutos en lugar de en días, no evitar nada.

    > **Descongelación deliberada.** La observabilidad completa pertenece a la etapa B-3. Este mínimo
    > se adelanta a conciencia porque hay **usuarios reales desde la primera célula**: sin él, la
    > forma de enterarse de que el bot lleva dos días mudo es que el cliente lo mencione. Se adelanta
    > lo imprescindible, no el panel de métricas.

    **Criterio de aceptación (revisado 2026-09-10):** Cada una de
    las ocho condiciones se provoca en prueba con un sumidero de notificación falso y produce
    exactamente una notificación con su código. Las métricas se entregan por registro estructurado o
    copias `VACUUM INTO`, nunca por endpoint HTTP ni consulta en vivo de `hexcell-admin` (adr-0024).
    Umbrales como parámetros sin valor normativo. Depende de la tarea 25-b. Trazabilidad: FR-14
    (decisión de 2026-09-10).
    **Dividida el 2026-09-13.** 20-a (HEX-077-a, cerrada): puerto de notificación en `hexcell-core`
    con sumidero falso y sumidero Telegram en `crates/hexcell` (`HEXCELL_TELEGRAM_BOT_TOKEN`,
    `HEXCELL_TELEGRAM_CHAT_ID`, `HEXCELL_TELEGRAM_URL_BASE`), sin cablear en `main.rs` ni en la
    composición. 20-c (HEX-077-c, cerrada): latencia hasta el acuse como cuarta clave del productor de
    métricas del sidecar (`adr-0035`). 20-d (HEX-077-d, cerrada): dead-man's switch. 20-b (HEX-077-b,
    cerrada el 2026-09-14): siete de las ocho condiciones de alerta cableadas sobre señales existentes
    (`crates/hexcell/src/alertas.rs`), sumidero Telegram construido en `main.rs` y variables
    `HEXCELL_TELEGRAM_*` pasadas por la composición; la octava —bucle de reinicio— queda diferida en
    D-54 y `adr-0037`. Con 20-b cierra la tarea 20 completa.
21. **Escribir el runbook de operación** (0,5 días). Qué comando usar en cada situación, qué efecto
    tiene y cómo verificar que salió bien. Incluye `cell rebind` con su remisión explícita al
    runbook de baneo de la etapa A-7, que es donde se decide **si procede** sustituir el número;
    aquí solo se documenta **cómo** se ejecuta.

    **Alcance añadido (decidido 2026-09-13, HEX-078):** el procedimiento de respuesta ante un
    `OOMKilled` —qué hacer cuando un contenedor muere por exceder su límite de memoria, fijados los
    límites en la tarea 6— pasa a ser alcance explícito de esta tarea: se documenta en el runbook
    que aquí se escribe, diferido desde HEX-078 (AC-6 del 00-spec.yaml de esa tarea).
    `docs/runbook-operacion.md` NO se crea en HEX-078.
22. **Configuración por célula como archivos** (1 día). Implementar la gestión de configuración basada en archivos (valores por defecto compartidos y superposiciones o overlays por célula) con validación de fallo cerrado al arrancar (concretando la tarea 8 sin editarla), gestionada de forma centralizada por `hexcell-admin` y versionable en git.

    **Cerrada el 2026-09-21 con HEX-081.**

    **Criterio de aceptación (revisado 2026-09-10):** Los archivos contienen solo parámetros no
    secretos; todo secreto sigue viajando por variable de entorno (HEX-064/HEX-065). `hexcell-admin`
    renderiza los archivos al entorno de la plantilla de la tarea 8: el binario de la célula no gana
    un segundo lector de configuración. Un overlay con clave desconocida o valor inválido aborta el
    arranque. Trazabilidad: detalle operativo documentado en README, sección «Configuración por
    célula como archivos» (decisión de 2026-09-10); sin FR propia por decisión de producto.
23. **Comando de reporte de consumo de tokens por cliente** (0,5 días). Implementar un comando en `hexcell-admin` para generar el reporte de consumo de tokens por cliente apoyado en la persistencia consultable de A-4, contemplando la alternativa documentada de agregar los logs estructurados o leer las copias de respaldo (VACUUM INTO) para evitar leer de la base caliente bajo contención (FR-10).

    **Criterio de aceptación (revisado 2026-09-10):** El comando lee solo una copia
    `VACUUM INTO` o los registros estructurados, nunca `sessions.db` en caliente (STATUS.md,
    adr-0024). Agrega `consumo_por_conversacion` a total por célula y periodo; se acepta cuando el
    total coincide con la suma de conciliaciones sembradas. Trazabilidad: FR-14 (decisión de
    2026-09-10).

    **Cerrada el 2026-09-22 con HEX-084.**

    **Nota de cierre:** el comando `reporte tokens` de `hexcell-admin` agrega la fórmula literal
    de la vista `consumo_por_conversacion` de la migración 0004 (monto reservado menos la
    conciliación, sumado solo sobre reservas conciliadas) sobre una copia `VACUUM INTO` de
    `sessions.db` producida por la ruta de respaldo de la etapa A-2 y abierta en solo lectura,
    con ventana opcional por `resuelta_ms` (`--desde` inclusivo, `--hasta` exclusivo, fechas
    `AAAA-MM-DD` UTC validadas a mano sin crate nuevo, por el criterio de D-53) y modo
    `--simular`. La alternativa de agregar los registros estructurados en vez de la copia se
    consideró y **no se implementó**: queda registrada aquí como alternativa no implementada, no
    en la bitácora de descartes.
24. **Extensión del protocolo IPC: tipo de cierre de sesión y orden de pausa de envío**. `cerrar_sesion` es un stub que devuelve `SinConexion` (`crates/hexcell-canal-whatsmeow/src/adaptador.rs:744-747`, `TODO(A-3)`) y el protocolo no tiene tipo de logout ni orden de pausa (solo existe el estado `pausada`). Pendiente de aceptación de A-3 ejecutado en A-6. Traza a FR-12. Criterio: nuevo tipo de mensaje documentado en `docs/protocolo-ipc-nucleo-sidecar.md` con subida de versión de cable, implementado en `sidecar/internal/ipc/mensajes.go` y `crates/hexcell-canal-whatsmeow/src/mensajes.rs`, con prueba de contrato que desvincula y otra que pausa y reanuda el envío.

    **Cerrada el 2026-09-11 con HEX-071**: versión de cable 6, cuatro tipos nuevos (cierre de sesión y su acuse, orden de pausa de envío y su acuse), `cerrar_sesion` implementado (`crates/hexcell-canal-whatsmeow/src/adaptador.rs:947`). El enunciado anterior describe el estado previo.
25. **Productor de métricas del sidecar prometido en A-3**. `docs/plan/fase-a-3-adaptador-whatsmeow.md:105-109` promete ratio de acuses por contacto, reconexiones por hora y ventana de silencio; no existe productor en `sidecar/`. Trazabilidad: la promesa de A-3 no cita FR; registrada como pendiente en STATUS.md. Criterio: el sidecar emite las tres series por el canal aprobado en adr-0024 (registro estructurado), con prueba que las provoca en simulación.

    **Dividida el 2026-09-11.** 25-a (HEX-072-a, cerrada): clasificación de acuses de entrega y lectura de whatsmeow en un sumidero interno del sidecar (`sidecar/internal/canal/acuses.go`); por D-45 los acuses no viajan por IPC. 25-b (HEX-072-b, cerrada el 2026-09-13): el productor periódico de las tres series (ratio de acuses por contacto, reconexiones por hora, ventana de silencio) en una sola línea `key=value` del registro estructurado (`sidecar/internal/metricas/metricas.go`), homóloga a `metricas_instantanea` y normada en adr-0033, que extiende adr-0024. El acuse no lleva identificador de contacto, así que la segmentación por contacto se resuelve con un join correlación → conversación acotado a 256 contactos y 1024 correlaciones, con desalojo determinista (actividad más antigua primero, id ascendente como desempate) y contador `contactos_omitidos` para que el truncamiento sea observable; por D-46 se descartó la lista de LRU real. La clave por contacto es `id_conversacion`, nunca un JID (adr-0019). La latencia hasta el acuse, cuarta serie de la promesa de A-3, queda explícitamente diferida.

---

## Criterios de aceptación

* Una célula arranca con sus dos contenedores, el núcleo responde `GET /health/ready` con `200 OK` y
  la célula procesa un mensaje real de extremo a extremo.
* El consumo de memoria residente de la célula completa en reposo —núcleo más sidecar— es **inferior
  a 80 MB**, medido con ambas bases abiertas y la sesión de canal activa (NFR-01, Fase A).
* `cell pause` cierra el websocket antes de detener el núcleo, y durante toda la pausa no queda
  ninguna petición entrante sin atender, porque no hay ninguna.
* **Ni `cell pause`, ni `cell terminate`, ni una migración de célula emiten un solo mensaje saliente
  durante el drenaje**, y ninguna respuesta pendiente se entrega al reanudar: una prueba deja
  respuestas encoladas, pausa la célula, la reanuda y verifica que no salió nada.
* `cell unpause` no da la célula por lista hasta que `GET /health/ready` ha respondido `200 OK` al
  menos una vez, y esa confirmación exige pools SQLite operativos **y** sesión de canal activa; el
  sidecar reanuda la sesión sin re-emparejamiento **antes** de que la readiness pueda confirmarla,
  nunca después.
* Si el sidecar no logra reconectar la sesión whatsmeow dentro del margen de sondeo, `cell unpause`
  **no** declara la célula operativa: agota el tiempo de espera y la CLI reporta con claridad que la
  célula levantó contenedores pero el canal sigue mudo, distinguiendo ese caso del de pools SQLite
  caídos.
* `docker stop` con margen de 30 segundos produce salidas con código 0 en ambos contenedores y
  checkpoint del WAL completado, sin recurrir a `SIGKILL`.
* Una célula no puede listar, leer ni escribir el volumen de datos de otra, ni alcanzar su red
  interna; el intento falla y queda registrado (NFR-05).
* Ninguno de los dos procesos se ejecuta como `root` y el sistema de archivos raíz es de solo lectura
  salvo la ruta de datos.
* `cell terminate` deja el sistema sin rastro de la célula: sin contenedores, sin volúmenes y con el
  dispositivo desvinculado del número.
* **`cell rebind` exige confirmación explícita** y, sin ella, no toca nada: una invocación no
  confirmada deja la célula exactamente como estaba, con su sesión y sus datos intactos.
* **`cell rebind` conserva la memoria del bot y descarta solo lo que corresponde al dispositivo
  muerto.** Una prueba con una célula que ya tiene historial verifica que, tras sustituir el número,
  `sessions.db`, `knowledge_live.db` y el almacén de identidad del adaptador siguen intactos —el mismo
  contacto cae en el mismo hilo y la lista de exclusión (STOP) sigue vigente— y que el `sqlstore`
  del sidecar se ha descartado en lugar de restaurarse. Que los archivos existan no basta: el
  criterio se cumple cuando **el bot responde por el número nuevo y reconoce al contacto de antes**.
* **Entre la invocación de `cell rebind` y la confirmación del emparejamiento, la célula no emite un
  solo mensaje.** La pausa de envío es parte del comando, no una recomendación al operador, y una
  prueba con respuestas encoladas verifica que ninguna sale durante ese intervalo.
* **Cada sustitución de número queda registrada** con el número anterior, la fecha absoluta y el
  motivo, y el registro es consultable desde `cell status` sin abrir ningún archivo a mano.
* Interrumpir cualquier comando a mitad y reejecutarlo lleva el sistema al estado pretendido sin
  intervención manual.
* Cada una de las **ocho** condiciones de alerta, provocada deliberadamente, produce un mensaje de
  Telegram en menos de un minuto.
* La alerta de **baneo temporal detectado** llega marcada como de máxima prioridad y distinguible de
  las demás a simple vista, e incluye la fecha de expiración que reporta la taxonomía de la etapa
  A-3.
* La **caída del ratio de acuses de un contacto concreto** dispara la alerta aunque el ratio agregado
  de la célula siga dentro de lo normal. Una prueba con un contacto que deja de acusar y el resto
  acusando con normalidad debe alertar: si solo se mira el agregado, no alerta, y ese es exactamente
  el fallo que este criterio existe para impedir.
* Ninguna alerta, panel ni informe presenta un recuento de reportes de usuarios: **esa señal no
  existe** y no se estima ni se aproxima.
* Las métricas de **reconexiones por hora** y de **ventana de silencio entrante** están disponibles
  por célula y son consultables desde `cell status`.
* Una actualización de whatsmeow **no llega a ninguna célula de cliente** sin haber corrido 72 horas
  en la célula centinela, y el despliegue posterior es escalonado: una prueba del procedimiento
  verifica que no existe ninguna vía —ni la CI, ni la CLI— que actualice toda la cartera en un solo
  paso.
* **Apagar el servidor entero produce una notificación** procedente del dead-man's switch externo,
  sin que el servidor haya podido emitir nada.
* Las imágenes se construyen de forma reproducible desde la CI y sus tamaños quedan registrados.
* Con varias células simultáneas, el consumo agregado es compatible con la capacidad del servidor
  objetivo de 8 GB.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| El sidecar dispara el consumo por encima del presupuesto de fase. | Alto: incumplimiento de NFR-01 y del modelo de densidad. | Medir pronto y por separado núcleo y sidecar; si se supera, ajustar tamaño de pools, caché de vectores y límites de concurrencia antes de continuar. |
| Problemas de enlazado con la biblioteca C de las imágenes base mínimas. | Medio: retrasos de integración y binarios que no arrancan. | Decidir imágenes base y objetivos de compilación al principio de la etapa y validarlos con binarios mínimos antes de empaquetar los reales. |
| Permisos de volumen mal configurados que dejan datos accesibles entre células. | Muy alto: fallo de privacidad frente al cliente final, agravado porque el volumen contiene además las credenciales de sesión del canal. | Prueba automatizada de aislamiento como criterio bloqueante de la etapa. |
| Las redes locales de las células no están realmente separadas. | Alto: una célula podría hablar con el sidecar de otra por el socket IPC. | Red dedicada por célula y prueba explícita de alcance cruzado. |
| Alguno de los procesos no recibe `SIGTERM` por quedar bajo un intérprete de shell. | Alto: apagados abruptos, riesgo de corrupción del WAL y de las credenciales de sesión. | Ejecutar cada binario como proceso principal directo y verificar la señal en la tarea 7. |
| El diseño de enlaces simbólicos de épocas se comporta distinto sobre el volumen montado. | Medio: la conmutación atómica falla solo en producción. | Repetir la prueba de estrés de la etapa A-5 dentro de la célula contenedorizada antes de cerrar esta etapa. |
| Detener el núcleo antes que el sidecar. | Medio: mensajes recibidos por el canal que no tienen a quién entregarse. | El orden está fijado en el ADR y verificado por la prueba de ciclo de vida. El outbox durable de la etapa A-3 hace que, aun ocurriendo, los eventos se reentreguen en lugar de perderse. |
| El bot lleva días mudo y nadie se entera hasta que el cliente lo menciona. | Muy alto: se quema la confianza de un cliente de pago y con ella la referencia comercial. | Alertas push ante desvinculación y falta de reconexión, más la ventana de silencio entrante, con las señales emitidas por el sidecar. |
| Toda la vigilancia vive dentro del servidor vigilado. | Alto: la caída total del servidor —el fallo más grave— es justo la que no genera ninguna alerta. | Dead-man's switch externo: la ausencia de ping notifica desde fuera. |
| Las alertas se disparan tanto que se ignoran. | Medio: una alerta que nadie lee equivale a no tenerla. | **Ocho** condiciones concretas y accionables, no un volcado de métricas, con el baneo temporal jerarquizado por encima del resto; los umbrales se recalibran con los datos reales de la etapa A-7. |
| **Confundir la observabilidad con una defensa.** | Alto, y es un riesgo de criterio, no de código: se dimensiona el negocio como si vigilar redujera la probabilidad de baneo. | Queda escrito en la tarea 20 y se repite aquí: **la observabilidad acorta el tiempo de reacción, no evita el baneo**. El baneo permanente **suele llegar sin aviso previo**; el temporal es el único que a veces lo da, y por eso es la alerta de máxima prioridad. Las medidas que de verdad importan son las de contención de daño. |
| **Mirar el ratio de acuses en agregado** en lugar de por contacto. | Medio-alto: los bloqueos de usuarios —única señal indirecta disponible— se diluyen en la media y no se detecta ninguno hasta que llega el baneo. | La segmentación por contacto es alcance explícito de la tarea 20 y criterio de aceptación con una prueba de un solo contacto que deja de acusar. |
| **Actualizar whatsmeow en toda la cartera el mismo día.** | Muy alto: una versión candidata defectuosa —o que llame la atención de la detección de Meta— se lleva por delante a todos los clientes a la vez, y con ellos la única fuente de ingresos. | Célula centinela propia con número propio durante 72 horas y escalonado por lotes con parada ante incidencias, con criterio de aceptación que verifica que no existe una vía de actualización masiva en un solo paso. |

---

## Dependencias

* **De otras etapas:** etapas A-2, A-3, A-4 y A-5 completas. En particular, la disposición definitiva
  del directorio de datos que fija la etapa A-5, la persistencia de sesión de la etapa A-3 y la línea
  base de memoria de la etapa A-2.
* **Externas:** un registro de imágenes donde publicar; acceso a un entorno con Docker equivalente
  al servidor de destino para las mediciones; un bot de Telegram con su token y el chat de destino;
  una cuenta gratuita de healthchecks.io; y un **número de WhatsApp propio de HexCell, distinto del
  de laboratorio de la etapa A-3 y de los de cualquier cliente**, dedicado a la célula centinela del
  canary. Es bloqueante para la tarea 19, y su baneo es un coste asumido de antemano: para eso está.
* **De la etapa A-3:** la taxonomía de desconexión, el contador de envíos rechazados y las métricas
  por célula —acuses por contacto, reconexiones por hora, silencio entrante— son señales que emite el
  sidecar; esta etapa las recoge, las compara contra umbral y las entrega. El pinneado por commit y
  la ventana de actualización también se fijan allí; aquí se ejecuta su escalonado.
* **Decisiones de producto pendientes:** el **modelo de monetización** define cuándo se suspende a un
  cliente por falta de pago. El mecanismo se entrega aquí; la política que lo activa, no.
