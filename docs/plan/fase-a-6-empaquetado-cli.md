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
—`cell pause`, `cell unpause`, `cell terminate`, `cell list`, `cell status`— operan **solo sobre
Docker**. No hay blackholing de Caddy porque no hay Caddy: la desconexión del websocket saliente ya
corta el tráfico entrante, y no queda ninguna petición sin contestar. `cell create` completo, con
subdominios y registro en Meta, pertenece a la etapa B-2.

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
* `hexcell-admin` con los comandos `cell pause`, `cell unpause`, `cell terminate`, `cell list` y
  `cell status`.
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
6. **Fijar los límites de recursos** (0,5 días). Memoria, CPU y descriptores por contenedor, con el
   reparto entre núcleo y sidecar coherente con el techo de 80 MB por célula.
7. **Verificar la propagación de señales** (0,5 días). Comprobar que `docker stop` con margen de 30
   segundos produce el apagado ordenado del núcleo y el cierre limpio de sesión del sidecar, con
   salidas con código 0 y sin recurrir a `SIGKILL`.
8. **Parametrizar la plantilla de arranque por célula** (1 día). Todo lo que distingue a una célula de
   otra pasa a ser configuración: identificador, volumen, red, secretos y límites.
9. **Implementar el cliente del socket Unix de Docker** (1,5 días). Arranque, parada con margen,
   inspección, eliminación de contenedores y de volúmenes, con manejo explícito de errores.
10. **Construir el esqueleto de la CLI y el modelo de estado** (1 día). Analizador de argumentos,
    salida legible, códigos de retorno significativos, modo de simulación, y estados posibles de una
    célula con sus transiciones válidas.
11. **Implementar `cell pause` y `cell unpause`** (1,5 días). Orden explícito en la pausa —primero el
    sidecar, después el núcleo— y sondeo de disponibilidad cada 100 ms con límite temporal y mensaje
    de error claro si nunca llega a estar lista.
12. **Implementar `cell terminate`** (1 día). Cierre de sesión del canal desvinculando el dispositivo,
    drenaje de ambos contenedores, borrado físico de volúmenes incluidas las credenciales, y
    confirmación explícita requerida por tratarse de una operación destructiva.
13. **Implementar `cell list` y `cell status`** (0,5 días). Estado consolidado cruzando el plano de
    control con la realidad de Docker y con la salud del canal, señalando discrepancias.
14. **Dotar de idempotencia y recuperación a los comandos** (1 día). Reejecución segura tras un fallo
    parcial, con detección del punto en que quedó la secuencia.
15. **Medir memoria y tamaño de imágenes** (0,5 días). Consumo de la célula completa en reposo y bajo
    carga, y peso de ambas imágenes, registrados como valores de referencia.
16. **Escribir la prueba de aislamiento** (1 día). Levantar dos células y demostrar que ninguna puede
    leer ni escribir el volumen de la otra ni alcanzar su red, ni siquiera conociendo la ruta.
17. **Integrar la construcción de las imágenes en la CI** (1 día). Construcción reproducible,
    etiquetado por versión y por commit, y publicación en el registro elegido.
18. **Montar el canary de biblioteca y el despliegue escalonado** (1 día). Alta de una **célula
    centinela** propia, con número propio de HexCell y sin ningún cliente encima, que corre la
    versión candidata de whatsmeow durante **72 horas** antes de que la actualización toque a nadie
    más. Después, escalonado por lotes de la cartera, con parada si el lote anterior presenta baneos,
    desconexiones anómalas o `Client outdated (405)`. Queda escrito como prohibición operativa:
    **nunca actualizar todas las células el mismo día**. La centinela es además el sitio donde se
    ensayan medidas cuya eficacia no está probada —el experimento con Meta Verified, entre ellas—,
    porque es el único número cuyo baneo no le cuesta el negocio a nadie.
19. **Implementar alertas push, métricas por célula y el dead-man's switch** (1,5 días). Tres piezas
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
20. **Escribir el runbook de operación** (0,5 días). Qué comando usar en cada situación, qué efecto
    tiene y cómo verificar que salió bien.

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
| **Confundir la observabilidad con una defensa.** | Alto, y es un riesgo de criterio, no de código: se dimensiona el negocio como si vigilar redujera la probabilidad de baneo. | Queda escrito en la tarea 19 y se repite aquí: **la observabilidad acorta el tiempo de reacción, no evita el baneo**. El baneo permanente **suele llegar sin aviso previo**; el temporal es el único que a veces lo da, y por eso es la alerta de máxima prioridad. Las medidas que de verdad importan son las de contención de daño. |
| **Mirar el ratio de acuses en agregado** en lugar de por contacto. | Medio-alto: los bloqueos de usuarios —única señal indirecta disponible— se diluyen en la media y no se detecta ninguno hasta que llega el baneo. | La segmentación por contacto es alcance explícito de la tarea 19 y criterio de aceptación con una prueba de un solo contacto que deja de acusar. |
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
  canary. Es bloqueante para la tarea 18, y su baneo es un coste asumido de antemano: para eso está.
* **De la etapa A-3:** la taxonomía de desconexión, el contador de envíos rechazados y las métricas
  por célula —acuses por contacto, reconexiones por hora, silencio entrante— son señales que emite el
  sidecar; esta etapa las recoge, las compara contra umbral y las entrega. El pinneado por commit y
  la ventana de actualización también se fijan allí; aquí se ejecuta su escalonado.
* **Decisiones de producto pendientes:** el **modelo de monetización** define cuándo se suspende a un
  cliente por falta de pago. El mecanismo se entrega aquí; la política que lo activa, no.
