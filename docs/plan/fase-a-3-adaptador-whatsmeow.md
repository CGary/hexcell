# Fase A · Etapa 3 — Adaptador whatsmeow: sidecar Go y puerto de canal

**Duración relativa:** Larga.

---

## Objetivo

El núcleo de la etapa A-2 sabe conversar, pero habla con un adaptador simulado. Esta etapa lo conecta
por primera vez a WhatsApp de verdad, y lo hace por la vía no oficial: la biblioteca **whatsmeow**,
que implementa el protocolo de WhatsApp Web sobre un **websocket saliente**. No hay webhook, no hay
IP pública, no hay certificado que emitir ni puerto que abrir en el router. El servidor local se
conecta hacia fuera.

whatsmeow es una biblioteca Go y no existe equivalente maduro en Rust, de modo que el adaptador vive
en un **proceso separado** —un sidecar— que acompaña al núcleo dentro de la misma célula. Los dos
contenedores comparten red local y volumen, y se comunican por IPC sobre un socket local. Del lado
Rust, ese IPC se envuelve en una implementación del trait `ChannelAdapter`, de modo que el núcleo
sigue sin enterarse de que existe whatsmeow.

Hay dos problemas que esta etapa tiene que resolver bien o la célula no sobrevive a su primera
semana. El primero es la **persistencia de sesión**: whatsmeow se empareja escaneando un QR o
introduciendo un código, y si las credenciales no se persisten, cada reinicio del contenedor exige que
alguien vuelva a coger el teléfono del cliente. Es inaceptable en operación.

El segundo es el **riesgo de baneo**, y conviene enunciarlo sin adornos porque ordena todo lo demás:
es en buena medida **estructural**. Meta detecta la biblioteca por su huella de protocolo, y ninguna
medida de comportamiento lo elimina. Los issues `tulir/whatsmeow` **#810** y **#807** (mayo de 2025,
concentrados en Brasil) y **#989** (noviembre de 2025, suspensiones de 24 h con código de
*enforcement* `BULK_MESSAGING` pese a enviar pocos mensajes con pausas de 5 s) documentan baneos y
avisos de *"unauthorized tools"* sobre cuentas de **bajo volumen y solo-respuesta**; ninguno
identificó un patrón accionable y los tres se cerraron como *not planned*. La consecuencia de diseño
es doble. Primera: las medidas de comportamiento de esta etapa actúan sobre el término secundario de
la probabilidad —se implementan igualmente, porque son baratas, y se marcan una por una como
**[causa documentada]** o **[precautorio]** para que nadie las confunda con garantías—. Segunda: el
baneo se documenta como **evento esperado, no como fallo**, y lo que esta etapa sí puede garantizar
es que ninguna violación del invariante de solo-respuesta salga de nuestro propio código.

---

## Alcance

### Qué entra

* Sidecar Go con la sesión whatsmeow: conexión, mantenimiento del websocket y traducción de los
  eventos del protocolo al formato canónico del puerto de canal.
* **Emparejamiento** por código QR y por *pairing code* (código de ocho caracteres introducido en el
  teléfono), con una interfaz de operación que no obligue a exponer el terminal al cliente.
* **Persistencia de sesión** en el `sqlstore` de whatsmeow sobre el volumen de la célula, de modo que
  un reinicio del contenedor reanude la sesión sin re-escanear el QR.
* **Reconexión automática con retroceso exponencial** ante caídas del websocket, con límite superior
  de espera y registro de cada intento.
* **Política de reconexión diferenciada ante baneo temporal** [causa documentada]. Ante la variante de
  baneo temporal de la taxonomía, **está prohibido reconectar en bucle**: retroceso exponencial largo,
  célula en pausa y espera a la expiración. No es una recomendación de operación sino comportamiento
  del código, porque el hecho está verificado: **persistir con el cliente no oficial durante un baneo
  temporal escala el baneo a permanente**
  ([faq.whatsapp.com/1848531392146538](https://faq.whatsapp.com/1848531392146538)). La reactivación
  exige **decisión humana**; ninguna célula baneada se reactiva sola.
* **Taxonomía de desconexión** expuesta por el IPC hacia el núcleo, con **cada variante instrumentada
  por separado**: `LoggedOut` con su razón —`device_removed` entre ellas—, **baneo temporal con su
  fecha de expiración**, `StreamReplaced` y fallo de conexión con su código. Colapsarlas en un único
  estado "desconectado" **destruye la señal**: un `StreamReplaced` es una anécdota operativa, un
  baneo temporal es el único aviso previo que suele existir y un `LoggedOut` con `device_removed`
  cambia el procedimiento de restauración de la etapa A-2. La taxonomía es la señal cruda; el estado
  de sesión que el IPC ya expone (activa / reconectando / desvinculada) es su **proyección** para
  `GET /health/ready`, y se deriva de ella en lugar de sustituirla. Sobre la variante de baneo
  temporal, la célula queda **en pausa**, no reconectando.
* **Outbox durable en el sidecar (entrega *at-least-once*).** La **primera acción** del sidecar tras
  recibir un evento del websocket —antes de traducirlo, antes de entregarlo al núcleo, antes de
  cualquier otra cosa— es persistirlo con `fsync` en un outbox durable: una tabla SQLite sobre el
  volumen compartido. La entrega al núcleo se marca procesada **solo** con la confirmación explícita
  del núcleo. Al arrancar cualquiera de los dos procesos, todo lo no confirmado se reentrega. El
  núcleo deduplica por el identificador de deduplicación de FR-12, de modo que la reentrega sea
  inofensiva.

  > **Nota honesta sobre el límite de esta garantía.** El acuse de protocolo hacia WhatsApp lo emite
  > la biblioteca de forma automática al recibir el mensaje y **no se puede diferir** hasta que
  > nuestro outbox haya hecho `fsync`. Existe por tanto una ventana real —de milisegundos— en la que
  > un corte de corriente entre el acuse de protocolo y el `fsync` pierde el evento sin que WhatsApp
  > lo reenvíe. El outbox no elimina esa ventana: la reduce de "todo lo que hubiera en memoria" a
  > "el evento en vuelo". Se documenta porque prometer entrega exactamente-una-vez sobre este canal
  > sería mentir.
* **Protocolo IPC local** entre sidecar y núcleo: definición del formato de mensaje, del socket, de la
  semántica de reconexión y de la política de reintento y confirmación, apoyada en el outbox durable,
  de modo que ni un evento entrante se pierda si uno de los dos procesos se reinicia antes que el
  otro. El protocolo expone además el **estado de la sesión whatsmeow** (activa / reconectando /
  desvinculada); el núcleo incorpora ese estado a su `GET /health/ready` —contrato declarado en la
  etapa A-2, donde el trait `ChannelAdapter` ya reserva el campo y el simulado lo reportaba siempre
  activo—, de modo que la célula no se declara lista mientras la sesión del canal no esté activa.
* **Emisión de eventos de alerta y de métricas** hacia el sistema de notificaciones push que construye
  la etapa A-6: sesión desvinculada, **baneo temporal detectado con su expiración** —la señal de
  máxima prioridad de todo el plan, por ser el único aviso previo que suele existir—, sidecar sin
  reconectar durante más de 5 minutos, bucle de reinicios, descarte de un envío no solicitado por el
  componente de envío, y **descarte por TTL vencido en la cola de salida**. El sidecar produce además
  las métricas por célula que A-6 consume: **ratio de acuses de entrega segmentado por contacto**
  —nunca en agregado: es la única detección indirecta de bloqueos de usuarios, porque el bloqueo no se
  notifica pero sí cesan los acuses de ese contacto—, latencia hasta el acuse, reconexiones por hora y
  ventana de silencio entrante. El sidecar produce las señales; la etapa A-6 las entrega.
* Implementación del trait `ChannelAdapter` en Rust sobre ese IPC, incluido el **sub-trait de ciclo de
  vida de sesión** (emparejamiento y persistencia de credenciales), que en la Fase B quedará sin
  implementar porque la Cloud API no lo necesita.
* Mapeo del **JID** de whatsmeow al identificador interno de conversación, dentro del adaptador. El
  JID no cruza la frontera del puerto.
* Traducción de los acuses del protocolo a los acuses normalizados `sent`/`delivered`/`read`/`failed`.
* **Invariante de solo-responder impuesto por el sistema de tipos** [causa documentada]. El bot nunca
  inicia una conversación, y eso deja de ser una política verificada a posteriori para pasar a ser una
  propiedad del código: un `Outbound` **solo es construible a partir de un identificador de evento
  entrante válido**, mediante un constructor privado que exige ese testigo. Un test se puede saltar o
  borrar; un constructor privado, no —violarlo **no compila**—. El test de intento de envío no
  solicitado y el contador expuesto de rechazos **se conservan** como segunda línea de defensa contra
  el hueco que el tipo no cubra, nunca como única.
* **Cola de salida con TTL absoluto y reintentos idempotentes** [causa documentada]. Es el **vector
  real** de violación del invariante: el tipo garantiza que todo envío nace de un evento entrante,
  pero un reintento o un reencolado tras reinicio entrega esa respuesta **horas tarde**, y una
  respuesta que llega horas tarde es indistinguible de una **iniciación de conversación**. Por tanto:
  descarte duro si se supera el TTL medido **desde la marca temporal del evento entrante**, nunca
  desde el momento del encolado; reintentos acotados en número; y **ninguna cola de mensajes muertos
  que reencole al arrancar**. El TTL es un parámetro a calibrar y se documenta como tal; ningún valor
  se fija aquí por defecto.
* **Latencia mínima de respuesta y horario de atención configurable** [causa documentada]. Responder
  en menos de un segundo a las cuatro de la madrugada es la señal no humana más barata de emitir por
  accidente. Ambos son parámetros por célula, a calibrar con el cliente; esta etapa entrega el
  mecanismo y su punto de configuración, no un número.
* **Emisión del indicador de "escribiendo" antes de responder** [precautorio]. Se implementa como
  **higiene documentada de coste cero, no como defensa**. El único respaldo público es el whitepaper
  *"Stopping Abuse: How WhatsApp Fights Bulk Messaging and Automated Behavior"* (WhatsApp, 6 de
  febrero de 2019), sección *While Messaging*: *"If an account continually sends messages without
  triggering the typing indicator, it can be a signal of abuse, and we will ban the account."* Sus
  limitaciones deben quedar escritas junto a la medida: el documento tiene siete años, es **anterior a
  la arquitectura multi-dispositivo** (2021), no existe versión actualizada, no hay evidencia pública
  de su eficacia, y su propio razonamiento —que los emisores masivos "puede que no tengan capacidad
  técnica de falsificarlo"— se debilita cuando falsificarlo cuesta **una línea de código**. Se emite
  porque no cuesta nada, no porque proteja.
* **Variación de la plantilla del mensaje de presentación del bot** [causa documentada]. Un texto
  idéntico repetido a cientos de destinatarios es una señal bastante más plausible que la del
  indicador de escritura. Se entrega un conjunto de variantes por célula y una selección que no
  repita literalmente el mismo texto a destinatarios distintos.
* **Un mensaje por turno** [causa documentada]. Una respuesta entrante produce **un solo mensaje
  saliente**, nunca una ráfaga troceada. Y **nunca grupos, listas de difusión ni estados**: el
  adaptador no expone esas primitivas, de modo que no se puedan usar por descuido.
* **Identificación como bot y salida a humano ofrecida en el primer turno** [causa documentada]. El
  primer mensaje de cada conversación nueva declara que se está hablando con un asistente automático
  y ofrece la vía para hablar con una persona. Los reportes de usuarios son una de las tres familias
  de señales oficiales de Meta, y un usuario que sabe qué tiene delante reporta menos.
* **Cortacircuitos conversacional** [causa documentada]. Ante repetición detectada o frustración del
  interlocutor, el bot **cede a un humano y calla**, pero emitiendo **un único mensaje de traspaso**
  antes de hacerlo. Callar en seco aumenta los bloqueos, y un bloqueo es una señal que sí llega a
  Meta.
* **Lista de exclusión (STOP) persistente por célula y contacto** [causa documentada]. Efecto
  inmediato, **sin caducidad**, **precedencia sobre todo lo demás** —sobre la cola de salida, sobre el
  cortacircuitos y sobre cualquier respuesta pendiente— y **una única confirmación de baja**, que es
  el último mensaje que ese contacto recibe. Persiste en el volumen de la célula y sobrevive a
  reinicios, restauraciones y re-emparejamientos.
* **Rampa de volumen** configurable en las primeras semanas de vida de cada célula [precautorio]. Se
  entrega el mecanismo y sus parámetros. Explícitamente **fuera de alcance**: los protocolos de
  "calentamiento" con pasos y plazos y el *jitter* aleatorio como supuesta imitación de un humano.
  Son folclore de proveedores de envío masivo, no hay evidencia que los respalde y no entran en esta
  documentación como medida.
* **Dependencia de whatsmeow fijada por commit**, no por etiqueta ni por rango [precautorio], con una
  **ventana de actualización definida** por escrito. Correr atrasado tiene doble riesgo: se deja de
  conectar por `Client outdated (405)` (issues #415 y #1031, el patrón de rotura recurrente) y se
  declara una versión de cliente atípica, que es señal por sí misma. Procedimiento documentado de
  actualización ante una rotura de protocolo, con el *bump* de commit como operación de un solo paso.
  El escalonado de esa actualización por la cartera y la célula centinela que la ensaya 72 h antes
  pertenecen a la etapa A-6.

### Qué NO entra

* Cualquier funcionalidad de envío masivo, difusión, estados, grupos o contacto en frío. Es
  incompatible con el invariante de solo-responder y con la naturaleza del producto.
* Cualquier **mensaje proactivo "útil"**: recordatorios, seguimientos, encuestas de satisfacción o
  "¿sigues ahí?". Queda escrito aquí para que nadie lo reintroduzca como mejora de producto: es
  exactamente lo que el invariante impuesto por tipos impide construir.
* El adaptador de Cloud API: etapa B-1.
* El alta de las células piloto reales: etapa A-7. Aquí se prueba con un número de laboratorio propio,
  distinto de los números de los pilotos.
* Control de admisión y presupuesto: etapa A-4.

### Requisitos del PRD cubiertos

* **FR-01** — implementación de la variante de Fase A: recepción de mensajes por la sesión whatsmeow
  del sidecar, entregados al núcleo por el puerto de canal.
* **FR-12** — primera implementación completa del puerto, incluido el sub-trait de ciclo de vida de
  sesión.

---

## Entregables

* Binario del sidecar Go, con la dependencia de whatsmeow **fijada por commit** y la ventana de
  actualización declarada por escrito.
* Implementación `WhatsmeowAdapter` del trait `ChannelAdapter` en el workspace Rust.
* **Tipos del invariante de solo-responder**: el testigo de evento entrante y el constructor privado
  de `Outbound`, con la prueba de que el código que intenta esquivarlos no compila.
* **Cola de salida con TTL absoluto**, con su parámetro de TTL documentado y su contador de descartes
  expuesto.
* **Lista de exclusión (STOP)** persistente por célula y contacto, con su esquema y su punto de
  precedencia en el camino de envío.
* **Taxonomía de desconexión** documentada como parte de la especificación del IPC, con la
  correspondencia explícita entre cada variante y el estado de sesión que se proyecta a
  `GET /health/ready`.
* Especificación escrita del protocolo IPC, versionada en el repositorio, incluida la semántica de
  confirmación y de reentrega.
* Esquema y implementación del **outbox durable** del sidecar, con su política de retención y purga.
* Runbook de **re-emparejamiento por `PairPhone()`** (ver tarea 16), como procedimiento de
  recuperación de primera clase.
* `docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md` con el porqué del proceso separado, la elección del
  mecanismo IPC y el diseño de persistencia de sesión, con la numeración que fija el
  [índice de ADR](../adr/README.md). Es distinto de `adr-0009`, que registra la **elección de la
  biblioteca**; este registra la **arquitectura de sidecar** que esa elección impone.
* `docs/runbook-canal-fase-a.md`: emparejamiento de una célula, diagnóstico de desconexión,
  re-emparejamiento y procedimiento de actualización ante rotura de protocolo.
* Disciplina de comportamiento del canal implementada y con **cada medida marcada como
  [causa documentada] o [precautorio]** y sus parámetros a calibrar identificados como tales.
* Pruebas: del adaptador contra un sidecar simulado, y del sidecar contra un número de laboratorio.

---

## Tareas

1. **Especificar el protocolo IPC** (1 día). Formato de mensaje, transporte (socket de dominio Unix
   sobre el volumen compartido), semántica de confirmación de entrega y comportamiento ante
   reconexión de cualquiera de los dos extremos. Se escribe antes de implementar nada.
2. **Construir el esqueleto del sidecar y la conexión whatsmeow** (1,5 días). Arranque, conexión del
   websocket, recepción de eventos crudos y registro estructurado.
3. **Implementar el outbox durable** (1,5 días). Tabla SQLite sobre el volumen compartido, escritura
   con `fsync` como primera acción tras recibir del websocket, marcado de procesado solo contra
   confirmación del núcleo, reentrega de lo no confirmado al arrancar, y política de retención y
   purga de lo ya confirmado. Se implementa **antes** que la traducción de eventos: si el outbox
   llega después, el orden "persistir primero" se convierte en una intención en lugar de una
   propiedad del código.
4. **Implementar el emparejamiento por QR y por código** (1,5 días). Generación y presentación del QR,
   solicitud del *pairing code*, y una superficie de operación que permita completar el alta sin
   acceso al terminal del servidor.
5. **Implementar la persistencia de sesión en `sqlstore`** (1 día). Almacenamiento de credenciales
   sobre el volumen de la célula, con los permisos del modelo de aislamiento, y reanudación
   automática al arrancar.
6. **Implementar la reconexión con retroceso exponencial y la parada ante baneo temporal** (1,5 días).
   Reintentos con espera creciente y techo, distinción entre error transitorio y sesión inválida, y
   registro de cada transición. Y una rama aparte: ante la variante de **baneo temporal** de la
   taxonomía, retroceso exponencial largo, célula en pausa y espera a la expiración, sin
   reintentos agresivos y sin reactivación automática. La rama se implementa en el código, no se
   deja al criterio de quien opere.
7. **Implementar la taxonomía de desconexión y la detección de desvinculación** (1,5 días). Cada
   variante instrumentada por separado —`LoggedOut` con su razón, baneo temporal con su expiración,
   `StreamReplaced`, fallo de conexión con su código—, expuesta por el IPC hacia el núcleo, con su
   proyección al estado de sesión (activa / reconectando / desvinculada) que alimenta
   `GET /health/ready` y con un estado observable desde la CLI. La proyección **no sustituye** a la
   señal cruda: ambas viajan por el IPC.
8. **Traducir eventos y acuses al formato canónico** (1,5 días). Mensaje entrante a evento canónico
   con su identificador de deduplicación; acuses del protocolo a `sent`/`delivered`/`read`/`failed`.
9. **Implementar el mapeo JID → identificador interno** (1 día). Dentro del adaptador, con la garantía
   verificable de que el JID no cruza la frontera del puerto.
10. **Implementar `WhatsmeowAdapter` en Rust** (1,5 días). Cliente del IPC envuelto en el trait,
   incluido el sub-trait de ciclo de vida de sesión, con manejo de la caída del sidecar.
11. **Imponer el invariante de solo-responder en el sistema de tipos** (1 día). Testigo de evento
    entrante, constructor privado de `Outbound` que lo exige, y revisión de que ningún camino del
    sidecar ni del adaptador pueda fabricar un envío sin él. Se acompaña de una prueba de
    **compilación fallida**: el código que intenta construir un envío sin testigo debe ser rechazado
    por el compilador, y esa prueba forma parte de la batería. El contador expuesto de rechazos y el
    test de intento deliberado se conservan como segunda línea.
12. **Implementar la cola de salida con TTL absoluto y reintentos idempotentes** (1 día). TTL medido
    desde la marca temporal del evento entrante —no desde el encolado—, descarte duro al superarlo
    con registro y contador propios, reintentos acotados en número e idempotentes, y **ausencia
    deliberada de cola de mensajes muertos**: nada se reencola al arrancar. El TTL queda como
    parámetro documentado a calibrar, no como constante escondida en el código.
13. **Implementar la lista de exclusión (STOP)** (0,5 días). Tabla persistente por célula y contacto
    sobre el volumen, consulta en el punto más temprano del camino de envío —por delante de la cola de
    salida y del cortacircuitos—, efecto inmediato, sin caducidad y con una única confirmación de
    baja.
14. **Implementar la disciplina de comportamiento del canal** (1,5 días). Latencia mínima de respuesta
    y horario de atención configurables por célula; emisión del indicador de "escribiendo" antes de
    responder; variación de la plantilla del mensaje de presentación; un solo mensaje saliente por
    turno, sin primitivas de grupo, difusión ni estados en el adaptador; identificación como bot y
    salida a humano en el primer turno; cortacircuitos conversacional que cede a un humano emitiendo
    un único mensaje de traspaso; y rampa de volumen configurable para las primeras semanas de la
    célula. Los parámetros son configurables, pero desactivar la disciplina no es una opción de
    configuración. Cada medida se documenta con su marca de **[causa documentada]** o
    **[precautorio]** y, en el caso del indicador de escritura, con sus limitaciones al lado.
15. **Probar contra un número de laboratorio** (1,5 días). Emparejamiento, conversación real, reinicio
    de contenedores con reanudación sin re-escaneo, corte de red con reconexión, y desvinculación
    forzada desde el teléfono.
16. **Escribir y ensayar el runbook de re-emparejamiento por `PairPhone()`** (1 día). El
    re-emparejamiento no es un último recurso improvisado sino un **procedimiento de recuperación de
    primera clase**: `PairPhone()` genera un código de ocho caracteres que el piloto teclea en su
    propio teléfono, de modo que **no hace falta tener su teléfono en la mano** ni desplazarse. Es la
    segunda capa de defensa cuando el respaldo del `sqlstore` no basta o llega desfasado. El
    procedimiento se **ensaya una vez con piloto-01 antes de dar de alta a piloto-02**: un
    procedimiento de recuperación nunca ejecutado no es un procedimiento, es una suposición.
17. **Redactar el runbook del canal, el pinneado por commit y la ventana de actualización** (1 día).
    Fijación de la dependencia whatsmeow **por commit**, con la ventana de actualización declarada por
    escrito, y el paso a paso ante una rotura de protocolo: comprobar el proyecto de la biblioteca,
    subir el commit, reconstruir la imagen del sidecar y redesplegar. Queda escrito que el patrón de
    rotura recurrente es `Client outdated (405)` y que **no se puede comprometer ningún tiempo de
    recuperación** que dependa de un mantenedor voluntario. El escalonado de la actualización por la
    cartera, con la célula centinela, se ejecuta desde la etapa A-6.

---

## Criterios de aceptación

* Una célula recién creada se empareja con un número de WhatsApp mediante QR o código, y a partir de
  ese momento recibe y responde mensajes reales.
* **Reiniciar ambos contenedores de la célula reanuda la sesión sin re-escanear el QR.**
* **Todo evento recibido del protocolo está escrito en el outbox con `fsync` antes de cualquier otra
  acción**, verificado por inspección del orden de operaciones y por una prueba que interrumpe el
  proceso inmediatamente después de la recepción.
* **Tras un reinicio desacompasado de ambos procesos, en cualquiera de los dos órdenes y en cualquier
  punto del ciclo de entrega: cero eventos perdidos y cero eventos procesados por duplicado.** Es el
  criterio que sustituye al ambiguo "ningún mensaje acusado se pierde", que no decía qué medir ni
  cómo comprobarlo.
* El re-emparejamiento por `PairPhone()` se ha ejecutado con éxito al menos una vez sobre una célula
  real, con el código tecleado por el usuario en su propio teléfono y sin acceso físico al mismo.
* Un corte de red de varios minutos se recupera automáticamente por reconexión con retroceso, sin
  intervención manual y sin pérdida de eventos ya confirmados.
* Una desvinculación forzada desde el teléfono se detecta, se señaliza al núcleo y queda visible como
  estado consultable; no se disfraza de desconexión transitoria ni se reintenta indefinidamente.
* **Cada variante de desconexión llega al núcleo distinguible de las demás**: `LoggedOut` con su
  razón —`device_removed` incluida—, baneo temporal con su fecha de expiración, `StreamReplaced` y
  fallo de conexión con su código. Una prueba provoca o inyecta cada variante y verifica que ninguna
  se colapsa con otra en un genérico "desconectado", y que el estado de sesión que consume
  `GET /health/ready` se deriva de ellas sin borrarlas.
* **Ante un baneo temporal, el sidecar no reconecta en bucle.** Una prueba que inyecta la variante de
  baneo temporal verifica que la célula pasa a pausa, que el intervalo de reintento crece con
  retroceso largo hasta la expiración declarada y que **no hay reactivación automática** sin decisión
  humana. Es criterio de aceptación bloqueante y no una recomendación de operación: persistir escala
  el baneo temporal a permanente.
* El identificador JID de whatsmeow **no aparece** en ninguna estructura del núcleo ni en
  `sessions.db`; solo vive dentro del adaptador.
* Los acuses del protocolo se reflejan en el núcleo exclusivamente como
  `sent`/`delivered`/`read`/`failed`.
* El bot **no emite ningún mensaje que no sea respuesta a un mensaje entrante**, y eso se verifica
  **en el compilador antes que en ninguna prueba**: existe un caso de prueba que intenta construir un
  envío sin un identificador de evento entrante válido y **cuyo criterio de éxito es que no compile**.
  El invariante deja de ser una política y pasa a ser una propiedad del tipo.
* Como segunda línea —nunca como única—, una prueba intenta deliberadamente un envío no solicitado
  por los caminos que el tipo no cubre y verifica que el componente de envío lo bloquea y que el
  intento queda registrado (contador expuesto incrementado y entrada en el registro), en producción y
  no solo en laboratorio.
* **Una respuesta cuya edad supera el TTL absoluto desde la marca temporal del evento entrante se
  descarta, no se entrega tarde.** Una prueba retiene una respuesta más allá del TTL —por reintento o
  por reinicio del proceso con la cola poblada— y verifica que el envío **no sale**, que el descarte
  queda contabilizado y que nada la reencola al arrancar. Entregar esa respuesta con horas de retraso
  es lo que convierte una respuesta legítima en algo indistinguible de una iniciación de
  conversación.
* Un contacto dado de baja por la lista de exclusión (STOP) **deja de recibir cualquier mensaje**,
  incluidas las respuestas ya encoladas y el mensaje de traspaso del cortacircuitos, tras una única
  confirmación de baja. La exclusión sobrevive a un reinicio del contenedor, a una restauración desde
  respaldo y a un re-emparejamiento.
* El primer mensaje de una conversación nueva **identifica al remitente como asistente automático y
  ofrece la salida a un humano**, verificado sobre una conversación real del número de laboratorio.
* Activado el cortacircuitos conversacional, el bot cede a un humano tras emitir **exactamente un**
  mensaje de traspaso, y no vuelve a escribir en ese hilo sin un mensaje entrante nuevo. El caso de
  "callar en seco" sin mensaje de traspaso cuenta como fallo de la prueba.
* Un turno entrante produce **un único mensaje saliente**. El adaptador no expone primitiva alguna de
  grupo, lista de difusión ni estado, verificado por inspección de su superficie pública.
* Antes de cada respuesta se emite el indicador de "escribiendo", verificado sobre el número de
  laboratorio. Queda registrado junto a la medida que es **higiene de coste cero y no una defensa**.
* Los retardos de respuesta observados respetan la **latencia mínima** configurada y el **horario de
  atención** de la célula: un mensaje recibido fuera de horario no se responde hasta la apertura, y
  ninguna respuesta sale por debajo del umbral mínimo.
* El mensaje de presentación **no es idéntico** entre conversaciones distintas de la misma célula.
* La dependencia whatsmeow está fijada **por commit** y ese commit es visible en la imagen publicada.
  Subirlo y reconstruir la imagen del sidecar es una operación que no requiere tocar el núcleo Rust
  ni el protocolo IPC.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| **Baneo del número** por parte de WhatsApp. | Alto en la célula afectada, y **no eliminable**. | El riesgo es en buena medida **estructural**: Meta detecta la biblioteca por su huella de protocolo. Los issues `tulir/whatsmeow` **#810**, **#807** y **#989** documentan baneos sobre cuentas de bajo volumen y solo-respuesta, sin patrón accionable, cerrados como *not planned*. Las medidas de comportamiento de esta etapa reducen la probabilidad, no la anulan; el valor real está en **reducir el daño** (una célula, un número del cliente, aislamiento estricto) y en detectar pronto (etapa A-6). El baneo se trata como **evento esperado, no como fallo**. |
| **Baneo temporal que se convierte en permanente** por reconexión agresiva. | Muy alto: convierte una parada de 24 h en la pérdida definitiva del número de un cliente de pago. | Hecho verificado ([faq.whatsapp.com/1848531392146538](https://faq.whatsapp.com/1848531392146538)): persistir con el cliente no oficial durante un baneo temporal escala el baneo. La rama de baneo temporal de la taxonomía pone la célula en pausa con retroceso largo, sin reactivación automática, y es **criterio de aceptación bloqueante**, no una recomendación. |
| **Un reintento o un reencolado entrega una respuesta horas tarde** y parece iniciación de conversación. | Alto: es el vector real de violación del invariante, y el que el sistema de tipos **no** cubre. | TTL absoluto medido desde la marca temporal del evento entrante, descarte duro al superarlo, reintentos acotados e idempotentes y ausencia deliberada de cola de mensajes muertos. Con prueba que verifica el descarte, no la entrega tardía. |
| **Rotura del protocolo** por un cambio de WhatsApp, con **bus factor 1** en la biblioteca. | Alto: el canal queda inoperativo hasta que el arreglo se publique, y prácticamente todos los ~1.620 commits de whatsmeow son de un único mantenedor. | Dependencia fijada **por commit** y aislada en el sidecar, de modo que el arreglo sea un *bump* de una línea, con ventana de actualización definida y célula centinela que la ensaya 72 h antes de escalonarla (etapa A-6). El patrón recurrente es `Client outdated (405)` (issues #415 y #1031) y el arreglo es siempre actualizar. **No se compromete ningún tiempo de recuperación** que dependa de un tercero voluntario. Precedente: [la rotura de abril de 2026](https://github.com/lharries/whatsapp-mcp/issues/216) se resolvió en días, frente al [incidente equivalente en Baileys](https://github.com/WhiskeySockets/Baileys/issues/2488); con los clientes se pacta expresamente la posibilidad de semanas de silencio (etapa A-7). |
| **Correr atrasado** en la versión de la biblioteca. | Medio-alto, y por partida doble: se deja de conectar por `Client outdated (405)` y se declara una versión de cliente atípica, que es señal por sí misma. | Pinneado por commit **con ventana de actualización declarada**, no pinneado indefinido. Actualizar es la mitigación, no el riesgo; lo que se controla es el ritmo. |
| **Colapsar las variantes de desconexión** en un único estado "desconectado". | Alto: destruye la señal. El baneo temporal deja de distinguirse de un `StreamReplaced`, y con él se pierde el único aviso previo que suele existir. | Taxonomía instrumentada variante a variante en el IPC, con criterio de aceptación que las prueba por separado. El estado de sesión de `/health/ready` es una proyección de la taxonomía, nunca su sustituto. |
| Pérdida de las credenciales de sesión y re-emparejamiento forzoso. | Medio: sin una vía de recuperación acordada, obliga a coordinar con el piloto-02 en el peor momento. | **Dos capas.** Capa 1: el `sqlstore` entra en el respaldo de la etapa A-2 como tercera base, copiado por el propio sidecar vía `VACUUM INTO` sobre orden IPC y con frecuencia alta —esta etapa **expone la operación IPC** que lo hace posible, no la da por hecha—. Capa 2: re-emparejamiento por `PairPhone()`, con código de ocho caracteres que el piloto teclea en su propio teléfono, ensayado antes del alta de piloto-02. |
| Un fallo de corriente entre el acuse de protocolo y el `fsync` del outbox. | Bajo, pero real e imposible de eliminar: el acuse hacia WhatsApp es automático y no se puede diferir. | Se documenta explícitamente en el alcance en lugar de prometer entrega exactamente-una-vez. El outbox reduce la ventana de pérdida a milisegundos, de "todo lo que hubiera en memoria" a "el evento en vuelo". |
| El JID se filtra al núcleo por comodidad de depuración. | Alto: rompe la frontera entre el núcleo y el transporte y contamina datos históricos. | Criterio de aceptación explícito y prueba automatizada. |
| Un fallo del IPC pierde eventos entrantes silenciosamente. | Alto: mensajes de clientes finales que nunca se responden, sin rastro. | Outbox durable con `fsync` como primera acción y confirmación explícita del núcleo; semántica de reentrega especificada por escrito antes de implementar; y prueba de reinicio desacompasado de ambos procesos en ambos órdenes. |
| La disciplina de comportamiento se relaja bajo la presión de "responder más rápido". | Muy alto: pérdida del número del cliente. | Los parámetros son configurables, pero desactivar la disciplina no es una opción de configuración; queda registrado en `adr-0011` como decisión, no como ajuste. |
| Violación de los Términos de Servicio de WhatsApp. | Asumido conscientemente. | Riesgo **permanente y estructural**, no transitorio: el canal propio es el canal por defecto y el canal oficial se incorporará como canal **adicional que convive** con él, de modo que **no lo elimina**. Se gestiona, no se cierra: cliente titular del número y de la SIM, contrato que declara el canal como propio y no oficial sin garantía de disponibilidad, aislamiento estricto por célula y medidas de contención de daño. La evidencia de que ninguna conducta lo anula está en los issues **#810**, **#807** y **#989**. |

---

## Dependencias

* **De otras etapas:** etapa A-2 completa. El adaptador sustituye al simulado en un núcleo que ya
  funciona, la deduplicación por identificador de FR-12 que hace inofensiva la reentrega del outbox ya
  existe, y el procedimiento de respaldo al que esta etapa aporta la operación IPC de copia del
  `sqlstore` ya está construido.
* **Hacia otras etapas:** la etapa A-2 consume la variante `LoggedOut` con `device_removed` de la
  taxonomía, porque de ella depende su regla de restauración del `sqlstore`, y verifica contra esta
  etapa que la continuidad del hilo sobrevive al re-emparejamiento. La etapa A-6 consume las señales
  de alerta y las métricas por célula que aquí se emiten, y ejecuta el escalonado de actualización de
  la biblioteca con su célula centinela.
* **Externas:** un número de WhatsApp de laboratorio, distinto de los números de los clientes, y un
  teléfono para el emparejamiento y las pruebas de desvinculación.
* **Decisiones de producto pendientes:** ninguna bloquea el desarrollo de esta etapa, pero tres
  parámetros quedan sin valor por defecto y deben calibrarse con datos reales antes del primer
  cliente de pago, registrados como decisión pendiente en `docs/STATUS.md`: el **TTL absoluto** de la
  cola de salida, la **latencia mínima de respuesta** y el **horario de atención** por célula. Fijar
  aquí un número inventado sería peor que declararlos abiertos.
