# Protocolo IPC entre el núcleo y el sidecar

* **Versión de este protocolo:** 1.3, fijada el 2026-08-09.
* **Etapa que lo redacta:** A-3 (tarea 1 de `docs/plan/fase-a-3-adaptador-whatsmeow.md`).
* **Etapa que lo implementa:** A-3, repartida entre varias tareas. Este documento **declara** la
  semántica completa; el código que la cumple llega después y por partes: el outbox durable
  (tarea 3), la reconexión y la taxonomía de desconexión (tareas 6 y 7, cerradas por esta versión
  para el lado sidecar), el mapeo de identidad (tarea 9) y el cliente Rust del protocolo dentro de
  `WhatsmeowAdapter` (tarea 10). No existe todavía ningún socket abierto ni ningún extremo Rust:
  el estado se produce como `estado_sesion` codificable y se entrega a un sumidero inyectado.
* **Procesos que hablan este protocolo:** el binario `hexcell` (núcleo Rust) y el binario
  `hexcell-sidecar` (Go, whatsmeow), los dos contenedores de una misma célula sobre canal propio.
  El sidecar es un **coste permanente** de ese canal (`adr-0014`): este protocolo no es un
  andamio de transición hacia ninguna otra cosa.
* **Dónde se registrará la decisión:** `docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md`, todavía por
  escribir, es el ADR que fija el porqué del proceso separado, la elección del mecanismo IPC y el
  diseño de persistencia de sesión. Este documento es la **especificación**; aquel será el
  **registro de la decisión**, y se escribe cuando la etapa tenga delante también la persistencia
  de sesión (tarea 5) y la disciplina de comportamiento (tarea 14), porque su alcance las incluye.
  La sección 6 del contrato `docs/contrato-ipc-respaldo-del-sqlstore.md` difiere a ese mismo ADR
  la elección de transporte y de serialización; lo que aquí se fija es exactamente esa elección,
  y el ADR la recogerá sin cambiarla.

* **Correspondencia versión de documento → versión de cable:**

| Versión del documento | Versión de cable (`version` en el saludo) |
| :--- | :--- |
| 1.0 | `1` |
| 1.1 | `2` |
| 1.2 | `3` |
| 1.3 | `4` |

---

## Por qué esta especificación se escribe antes que el código

El protocolo tiene **dos extremos escritos en lenguajes distintos**, y el extremo Rust todavía no
existe. Si el formato se fijara de hecho, por lo que el sidecar Go acabe emitiendo, el núcleo
heredaría un formato elegido por la comodidad de la biblioteca de serialización de Go
—anidamiento, listas, valores nulos, tipos mezclados— que el lado Rust tendría que consumir sin
ninguna de esas comodidades.

Ese desequilibrio es concreto: **el workspace Rust solo declara `serde` en `hexcell-canal-whatsmeow`**, y
`adr-0019` rechazó explícitamente arrastrar un serializador por presupuesto de memoria (NFR-01,
≤ 80 MB por célula sobre canal propio). Escribir JSON a mano es barato; **analizarlo** a mano es
estrictamente más caro. Por eso el formato de la sección 1 no se elige por lo que es cómodo de
emitir, sino por lo que es **tratable de analizar sin dependencias** en el lado que aún no está
escrito.

---

## 1. Formato de mensaje

**Un objeto JSON plano por línea, codificado en UTF-8 y terminado en `\n` (0x0A).** No hay
cabecera binaria, ni prefijo de longitud, ni tramas multilínea: el delimitador de mensaje es el
salto de línea, y un mensaje es exactamente una línea.

Las cinco reglas del formato, todas restrictivas a propósito:

1. **Profundidad 1.** El valor de un campo nunca es otro objeto ni una lista. No hay estructuras
   anidadas ni arreglos de objetos en ninguna dirección.
2. **Solo cadenas y enteros.** Los valores son cadenas JSON o enteros con signo de 64 bits. No hay
   booleanos, ni `null`, ni números en coma flotante. Un booleano se expresa como una cadena de un
   conjunto cerrado; una marca temporal, como un entero.
3. **Conjunto de campos cerrado por tipo de mensaje.** Cada tipo declara exactamente sus campos.
   Un campo desconocido es un error de protocolo, no una extensión tolerada.
4. **Todos los campos, siempre presentes, en orden fijo.** La ausencia de valor se representa con
   la cadena vacía `""` o con el entero `0`, nunca omitiendo el campo. Un analizador escrito a
   mano no tiene que tratar campos opcionales ni orden variable, que son las dos fuentes habituales
   de complejidad accidental al analizar JSON sin biblioteca.
5. **Límite de línea: 131 072 bytes** (128 KiB), contando el salto de línea final. Una línea más
   larga es un error de protocolo y cierra la conexión. El límite existe para que el lector del
   otro extremo pueda dimensionar un búfer acotado en lugar de crecer sin techo ante una entrada
   malformada, que es la misma disciplina de contrapresión que `adr-0016` aplica al canal de
   eventos del núcleo.

Los dos primeros campos de **toda** línea son siempre los mismos y en este orden:

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | Versión de cable del protocolo. En esta especificación, `3`. |
| `tipo` | cadena | Uno de los nueve tipos cerrados de la sección 6. |

### Por qué JSON y no un formato binario, y qué se difiere a `adr-0011`

Un formato binario sería más pequeño y más rápido, y las dos cosas son irrelevantes al volumen de
una célula: unos pocos mensajes por segundo en el peor caso. Lo que sí importa es que el tráfico
del socket se pueda volcar a un archivo y entenderse a simple vista durante un diagnóstico, y que
un desajuste entre los dos binarios se detecte con un mensaje legible en lugar de con un
desplazamiento de bytes.

Este documento **no decide** si el lado Rust analizará estas líneas a mano —como ya hace
`crates/hexcell/src/registro.rs` para emitirlas— o si la tarea 10 justificará por fin una
dependencia de serialización. Fija la restricción que hace **viable** la primera opción y deja la
elección al ADR. Cualquier anidamiento admitido aquí crearía esa dependencia en silencio.

---

## 2. Transporte: socket de dominio Unix sobre el volumen compartido

**Un socket de dominio Unix (`AF_UNIX`) de tipo `SOCK_STREAM`**, cuyo archivo vive en el volumen
compartido de la célula. No es TCP sobre `localhost`, ni HTTP, ni una tubería nombrada.

* **`SOCK_STREAM` y no `SOCK_DGRAM`**, porque el flujo de bytes con entrega ordenada es lo que
  hace correcto el delimitado por salto de línea. Un socket de datagramas obligaría a que cada
  mensaje cupiera en un datagrama y perdería el orden entre reintentos.
* **Y no TCP sobre `localhost`**, porque el socket de dominio Unix se autoriza con los permisos
  del sistema de archivos —un archivo con dueño y modo— en lugar de con un puerto que cualquier
  proceso del mismo espacio de red puede alcanzar.
* **Ruta por omisión:** `/var/lib/hexcell/ipc/sidecar.sock`, configurable en el sidecar con la
  variable de entorno `HEXCELL_SOCKET_IPC`. El núcleo debe recibir la misma ruta por su propia
  configuración; el protocolo no la descubre solo.
* **Permisos:** el archivo del socket se crea con modo `0600` y pertenece al usuario que comparten
  los dos contenedores de la célula. Ningún otro proceso del servidor puede abrirlo.

### Papeles: el sidecar escucha, el núcleo conecta

**El sidecar es el servidor** —crea el socket, hace `bind` y `listen`— y **el núcleo es el
cliente**, que conecta y reintenta mientras no lo consiga. El reparto no es arbitrario:

1. El estado durable del canal —el outbox de la tarea 3 y el `sqlstore` de la tarea 5— vive del
   lado del sidecar. El proceso que conserva el estado es el que debe estar disponible para que el
   otro lo busque, no al revés.
2. El sidecar es el que **produce** eventos sin que nadie se los pida. Un productor que tuviera
   que conectar hacia un consumidor ausente necesitaría su propia lógica de reintento además del
   outbox; escuchando, conserva lo no confirmado hasta que alguien llegue a por ello.

**Una sola conexión activa a la vez.** Si llega una segunda conexión mientras hay una establecida,
el sidecar acepta la nueva y cierra la anterior: en la práctica eso solo ocurre cuando el núcleo
se reinició sin que su descriptor anterior se hubiera cerrado del todo, y quedarse con la conexión
más reciente es lo que resuelve ese caso sin intervención.

### Desenlace del socket obsoleto al arrancar

Un archivo de socket **sobrevive al proceso que lo creó**. Si el contenedor del sidecar muere sin
limpiar, en el siguiente arranque el `bind` fallaría con `EADDRINUSE` sobre un archivo que no
escucha nadie. Borrar el archivo a ciegas antes de cada `bind` sería peor: dos sidecars vivos por
error se robarían el socket en silencio. El procedimiento fijado es el siguiente:

1. El sidecar intenta **conectar** como cliente a la ruta configurada.
2. Si la conexión **tiene éxito**, hay otro sidecar vivo escuchando: este arranque es un error de
   operación. El proceso registra el hecho y **termina**; no borra nada.
3. Si la conexión falla con «conexión rechazada» —nadie escucha— o el archivo no existe, el socket
   es obsoleto: el sidecar **desenlaza** la ruta y procede con `bind` y `listen`.
4. Cualquier otro error al comprobar la ruta aborta el arranque con registro, sin borrar nada.

---

## 3. Saludo de versión

**El primer mensaje de cada conexión, en las dos direcciones, es un `saludo`.** El núcleo, recién
conectado, envía el suyo antes que cualquier otra cosa; el sidecar responde con el suyo antes de
entregar ningún evento.

Si la `version` recibida no coincide con la propia, el extremo que la recibe **cierra la conexión**
y registra el desajuste con las dos versiones. No hay negociación ni degradación parcial: un
desajuste de versión es un error de despliegue —una imagen que no se actualizó con la otra— y
tratarlo como tal, con la célula caída y un mensaje claro, es mucho más barato que descubrirlo
semanas después por un campo que se leía torcido.

Con la versión 1.2 del documento, la versión de cable pasa de `2` a `3`. Con la versión 1.3, pasa de `3` a `4`. La regla no cambia de
sustancia: sigue siendo igualdad estricta del entero, en las dos direcciones, sin negociación ni
degradación. Si un sidecar que habla la versión 4 recibe un saludo con versión 3, cierra la
conexión e informa; el caso inverso es simétrico. En la práctica, este desajuste indica que una
imagen del contenedor se actualizó y la otra no, y el remedio es actualizar, no negociar.

El saludo no lleva ninguna credencial: la autorización es el permiso del archivo del socket
(sección 2), no un dato del protocolo.

---

## 4. Semántica de confirmación de entrega

La garantía del canal es **entrega al menos una vez** (*at-least-once*), con **deduplicación en el
núcleo** por el identificador de deduplicación de FR-12. Entrega exactamente una vez no se promete
y no se puede prometer: el acuse de protocolo hacia WhatsApp lo emite la biblioteca de forma
automática al recibir el mensaje y no se puede diferir, de modo que existe una ventana real —de
milisegundos— entre ese acuse y la escritura durable, y un corte de corriente dentro de ella pierde
el evento sin que WhatsApp lo reenvíe. El outbox reduce esa ventana; no la elimina.

### Persistir primero

**La primera acción del sidecar tras recibir un evento del websocket —antes de traducirlo, antes
de entregarlo, antes de cualquier otra cosa— es persistirlo con `fsync` en el outbox durable.**
Solo después se emite por el socket. El orden es una propiedad del código, no una intención: por
eso el outbox (tarea 3) se implementa **antes** que la traducción de eventos (tarea 8).

### El acuse referencia el identificador durable, nunca un número de secuencia

El núcleo confirma cada evento con un mensaje `confirmacion` que lleva el **identificador de
deduplicación** del evento —el mismo `id_deduplicacion` que viajó en el `evento_entrante`—, y el
sidecar marca la entrada del outbox como procesada **solo** al recibirlo.

**Está prohibido usar un número de secuencia por conexión como referencia del acuse.** El motivo
es el criterio de aceptación de la etapa: cero eventos perdidos y cero procesados por duplicado
tras un reinicio desacompasado de los dos procesos, **en cualquiera de los dos órdenes**. Un
contador por conexión se reinicia con la conexión, así que tras una reconexión el acuse número 7
del núcleo y el evento número 7 del sidecar pueden ser cosas distintas, y el desajuste marca como
procesado un evento que nunca se entregó. El identificador de deduplicación, en cambio, es
**durable y global**: sobrevive al reinicio de los dos procesos, identifica el mismo evento en las
dos bases y no depende de cuántas conexiones hubo por el medio.

### Reentrega

Al establecerse una conexión, y tras el saludo, el sidecar **reentrega todo lo no confirmado** del
outbox antes de emitir eventos nuevos, en el orden en que lo persistió. La reentrega es inofensiva
porque el núcleo deduplica: un evento ya procesado se descarta por su identificador y se confirma
igualmente, para que el sidecar pueda por fin marcarlo y purgarlo.

El núcleo **no** confirma al recibir: confirma **cuando el evento está durablemente registrado de
su lado**. Confirmar antes convertiría la garantía en «al menos una vez hasta que el núcleo se
caiga», que es no tener garantía.

Las órdenes que van del núcleo al sidecar —hoy solo la del respaldo del `sqlstore`, sección 7— no
usan este mecanismo: no llevan outbox, y una orden perdida por una desconexión se vuelve a emitir
en la siguiente ronda. Perder una copia de una ronda no es un evento de cliente perdido.

---

## 5. Reconexión de cualquiera de los dos extremos

Los dos procesos se reinician por separado, en cualquier orden, y el protocolo debe sobrevivir a
los tres casos. Ninguno exige intervención manual.

### El núcleo se reinicia primero

El sidecar detecta el cierre de la conexión, **sigue recibiendo del websocket y sigue persistiendo
en el outbox**: no se detiene por no tener a quién entregar. Lo que no consigue entregar se acumula
como no confirmado. Cuando el núcleo vuelve, conecta, saluda y recibe la reentrega completa de la
sección 4. El sidecar no cierra su sesión de WhatsApp por una desconexión del núcleo; desvincularse
del canal porque el consumidor local se reinició sería destruir la sesión por un motivo ajeno a
ella.

### El sidecar se reinicia primero

El núcleo detecta el cierre y **reintenta conectar con retroceso exponencial y techo**, sin
abandonar. Mientras no haya conexión, el estado de sesión que el núcleo publica es el de la
sección 6 con valor `reconectando`, y la célula **no se declara lista**. Al volver el sidecar, este
desenlaza el socket obsoleto (sección 2), escucha de nuevo, y el siguiente reintento del núcleo
conecta. Todo lo que el sidecar no había confirmado sigue en el outbox y se reentrega.

### Los dos se reinician a la vez

Es el caso anterior con el reintento del núcleo empezando antes: no hay nada específico que hacer.
El invariante que sostiene los tres casos es el mismo: **el estado que importa está en disco, no en
la conexión**.

### Retroceso configurable del sidecar

La política propia del sidecar usa retroceso exponencial determinista con techo. Sus valores se
leen por la misma configuración que el socket y el `sqlstore`, nunca desde un camino ad hoc:
`HEXCELL_RETROCESO_INICIAL_MS`, `HEXCELL_RETROCESO_FACTOR`,
`HEXCELL_RETROCESO_MAXIMO_MS`, `HEXCELL_RETROCESO_BANEO_INICIAL_MS` y
`HEXCELL_RETROCESO_BANEO_MAXIMO_MS`. Los valores por omisión existen para arrancar el proceso,
pero quedan **pendientes de calibración** bajo tráfico real.

No se confunden dos planos: una desconexión del socket local se reintenta con normalidad; una
desconexión del canal por baneo temporal entra en `pausada`, usa el retroceso largo y no ejecuta
reactivación automática.

---

## 6. Conjunto cerrado de tipos de mensaje

Once tipos. Los seis de la versión 1.0 se conservan intactos; los tres tipos de emparejamiento
llegan con la versión 1.1. La versión 1.2 no añade tipos: solo cierra el vocabulario de
`estado_sesion`. La versión 1.3 añade dos tipos para la dirección saliente: `mensaje_saliente` y `acuse_envio`.
Ampliar el conjunto de tipos es cambiar la versión del protocolo.

| `tipo` | Dirección | Propósito |
| :--- | :--- | :--- |
| `saludo` | ambas | Primer mensaje de toda conexión (sección 3). |
| `evento_entrante` | sidecar → núcleo | Un mensaje recibido del canal, ya normalizado. |
| `confirmacion` | núcleo → sidecar | Acuse durable de un `evento_entrante` (sección 4). |
| `estado_sesion` | sidecar → núcleo | Estado de la sesión de WhatsApp y su causa. |
| `orden_respaldo_sqlstore` | núcleo → sidecar | Orden de copia del `sqlstore` (sección 7). |
| `acuse_respaldo_sqlstore` | sidecar → núcleo | Desenlace de esa copia (sección 7). |
| `orden_emparejar` | núcleo → sidecar | Orden de iniciar un emparejamiento por QR o por código de vinculación. |
| `codigo_emparejamiento` | sidecar → núcleo | Código QR o código de vinculación de ocho caracteres. |
| `acuse_emparejamiento` | sidecar → núcleo | Resultado terminal del emparejamiento. |
| `mensaje_saliente` | núcleo → sidecar | Mensaje que el núcleo envía hacia el canal. |
| `acuse_envio` | sidecar → núcleo | Notificación de progreso o fallo de un mensaje saliente. |

### `saludo`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `saludo`. |
| `emisor` | cadena | `nucleo` o `sidecar`. |
| `id_celula` | cadena | Identificador opaco de la célula, para correlacionar registros. |

### `evento_entrante`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `evento_entrante`. |
| `id_deduplicacion` | cadena | Identificador durable del evento (FR-12). Es lo que el acuse referencia. |
| `id_conversacion` | cadena | Identificador **interno** del hilo, opaco para el núcleo. |
| `id_remitente` | cadena | Identificador **interno** de quien escribió, opaco para el núcleo. |
| `contenido` | cadena | Texto del mensaje, ya normalizado. |
| `marca_temporal_ms` | entero | Momento del evento según el transporte, en milisegundos desde la época Unix. |

**Ningún identificador de transporte cruza esta frontera.** No hay campo para el JID de whatsmeow,
ni para el identificador de dispositivo, ni para el número de teléfono, y no lo habrá: el mapeo del
JID al identificador interno vive **dentro del adaptador**, en su almacén de identidad propio
(tarea 9, `adr-0010`), y el núcleo trata el identificador interno como opaco. El conjunto de campos
cerrado de la regla 3 de la sección 1 es lo que hace esa garantía verificable por la forma del
mensaje y no solo por la disciplina de quien lo escriba.

`marca_temporal_ms` es la marca del **evento entrante**, no la del encolado: es la que mide el TTL
absoluto de la cola de salida (tarea 12), y medirlo desde otro instante es exactamente el fallo
contra el que ese TTL existe.

### `confirmacion`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `confirmacion`. |
| `id_deduplicacion` | cadena | El mismo que llegó en el `evento_entrante`. Nunca un número de secuencia. |

### `estado_sesion`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `estado_sesion`. |
| `estado` | cadena | `activa`, `reconectando`, `desvinculada` o `pausada`. |
| `causa` | cadena | Variante cruda de la taxonomía de desconexión; `""` si no aplica. |
| `codigo` | entero | Código de la rama de desconexión cuando lo hay; `0` si no aplica. |
| `expira_en_ms` | entero | Expiración declarada de un baneo temporal, en milisegundos desde la época Unix; `0` si no aplica. |

Dos precisiones que este documento **no** puede saltarse:

* **El puerto Rust no reserva hoy ningún campo de estado de sesión.** El trait `ChannelAdapter`
  (`crates/hexcell-core/src/canal.rs`) declara exactamente dos métodos, `send` y `estado_ventana`,
  y el sub-trait `CicloDeVidaSesion` otros dos, `iniciar_emparejamiento` y `cerrar_sesion`.
  Incorporar este estado al puerto y a `GET /health/ready` es trabajo de la tarea 10 de esta misma
  etapa, no algo ya hecho en A-2. Se deja escrito para que nadie lo dé por existente.
* **El vocabulario de `causa` queda cerrado en la versión 1.2**, con cada variante instrumentada
  por separado. La señal cruda **viaja junto a** su proyección a `estado`, nunca en su lugar:
  colapsarlas destruiría la única señal de aviso previo que suele existir.

Estados declarados:

| Valor | Significado |
| :--- | :--- |
| `activa` | Sesión de WhatsApp operativa. |
| `reconectando` | Desconexión transitoria con reintentos en curso. |
| `desvinculada` | Sesión inválida por `LoggedOut`; requiere recuperación humana. |
| `pausada` | Baneo temporal detectado; no hay reactivación automática. |

<!-- inicio-causas-estado-sesion -->
| `causa` | Proyección a `estado` | `codigo` | `expira_en_ms` |
| :--- | :--- | :--- | :--- |
| `baneo_temporal` | `pausada` | Código `TempBanReason` de whatsmeow (101..106). | Expiración absoluta Unix epoch ms; `0` si whatsmeow no declara expiración. |
| `cliente_obsoleto` | `reconectando` | `0`. | `0`. |
| `desconexion_de_transporte` | `reconectando` | `0`. | `0`. |
| `desvinculada_dispositivo_removido` | `desvinculada` | Código `ConnectFailureReason` recibido en `LoggedOut`. | `0`. |
| `desvinculada_sesion_cerrada` | `desvinculada` | Código `ConnectFailureReason` recibido en `LoggedOut`. | `0`. |
| `error_de_flujo` | `reconectando` | Código numérico del `StreamError` si es interpretable; `0` si no aplica. | `0`. |
| `fallo_de_conexion` | `reconectando` | Código `ConnectFailureReason` de whatsmeow (400..503). | `0`. |
| `sesion_reemplazada` | `reconectando` | `0`. | `0`. |
<!-- fin-causas-estado-sesion -->

Dos trampas de la API quedan documentadas porque cambian el comportamiento:

* `device_removed` no existe como razón pública de `LoggedOut`. La firma observable es
  `LoggedOut{OnConnect:false}`; `LoggedOut{OnConnect:true}` puede traer la misma razón numérica y
  se clasifica como `desvinculada_sesion_cerrada`.
* `TemporaryBan.Expire` es una duración relativa. El sidecar la convierte a milisegundos absolutos
  con `ahora_ms + Expire.Milliseconds()`. Si `Expire == 0`, `expira_en_ms` queda en `0`.

La rama `baneo_temporal` entra en `pausada`, usa el retroceso largo configurado y no ejecuta ningún
camino de reactivación automática. Volver al servicio exige reiniciar el proceso o contenedor por
decisión humana; no existe mensaje IPC de reanudación.

### `orden_emparejar`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `orden_emparejar`. |
| `metodo` | cadena | `qr` o `codigo_de_vinculacion`. |

El número de teléfono de la célula **no viaja en este mensaje**. Si el método es
`codigo_de_vinculacion`, el sidecar lo lee de su configuración (`HEXCELL_TELEFONO_CELULA`), donde
lo fijó el procedimiento de alta de la célula. Poner el número en un campo IPC lo expondría a un
núcleo comprometido y violaría la guardia de `mensajes_test.go` que prohíbe campos con nombres de
identificador de transporte.

### `codigo_emparejamiento`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `codigo_emparejamiento`. |
| `metodo` | cadena | `qr` o `codigo_de_vinculacion`. Indica de qué tipo es `valor`. |
| `valor` | cadena | Dato opaco: la cadena a codificar como QR, o el código de ocho caracteres. |
| `expira_en_ms` | entero | Milisegundos desde la época Unix en que este código deja de ser válido. `0` si la expiración es desconocida (caso del código de vinculación, cuya caducidad whatsmeow no expone). |

Cada emisión de `codigo_emparejamiento` con `metodo=qr` **sustituye al anterior**: el consumidor
muestra solo el último y descarta los previos. Con `metodo=codigo_de_vinculacion` se emite
exactamente uno.

### `acuse_emparejamiento`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `acuse_emparejamiento`. |
| `resultado` | cadena | `completado`, `expirado` o `fallido`. |
| `motivo` | cadena | Descripción legible si `resultado` es `fallido`; `""` en caso contrario. **Nunca lleva la cadena QR, el código de vinculación ni ningún otro dato de credencial.** |

### `mensaje_saliente`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `mensaje_saliente`. |
| `id_mensaje` | cadena | Identificador global del mensaje originado en el núcleo. |
| `id_conversacion` | cadena | Identificador interno de la conversación destino. |
| `contenido` | cadena | Texto del mensaje a enviar. |
| `marca_temporal_origen_ms` | entero | Milisegundos desde la época Unix en que el núcleo originó el mensaje. |

### `acuse_envio`

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `acuse_envio`. |
| `id_mensaje` | cadena | El mismo identificador global del `mensaje_saliente`. |
| `estado` | cadena | Estado de la entrega: `enviado`, `entregado`, `leido` o `fallido`. |
| `id_correlacion` | cadena | Identificador asignado por el canal subyacente (ej. whatsmeow) al enviar; `""` si el estado es `fallido` temprano. |
| `motivo` | cadena | Descripción legible del error si el estado es `fallido`; `""` en caso contrario. |
| `marca_temporal_ms` | entero | Momento del suceso en milisegundos desde la época Unix. |

---

## 7. La operación de respaldo del `sqlstore`

`docs/contrato-ipc-respaldo-del-sqlstore.md`, versión 1.0 del 2026-07-30, fija el **mensaje**, el
**responsable**, la **frecuencia** y el **destino** de la copia del `sqlstore`, y difiere a
`adr-0011` el mecanismo de transporte. Este protocolo es ese mecanismo, y **encaja con aquel
contrato sin modificarlo**: los campos de las dos tablas siguientes son exactamente los suyos.

### `orden_respaldo_sqlstore` (núcleo → sidecar)

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `orden_respaldo_sqlstore`. |
| `orden` | cadena | Cadena fija `respaldar_sqlstore`. |
| `destino` | cadena | Directorio de destino ya resuelto por quien dispara la orden. |
| `identificador_de_ronda` | cadena | Agrupa esta orden con las de las otras tres bases de la misma ronda. |

### `acuse_respaldo_sqlstore` (sidecar → núcleo)

| Campo | Tipo | Descripción |
| :--- | :--- | :--- |
| `version` | entero | `4`. |
| `tipo` | cadena | `acuse_respaldo_sqlstore`. |
| `identificador_de_ronda` | cadena | El mismo recibido en la orden. |
| `resultado` | cadena | `completado` o `fallido`. |
| `ruta_de_la_copia` | cadena | Ruta de la copia; `""` si `resultado` es `fallido`. |
| `bytes` | entero | Tamaño de la copia; `0` si `resultado` es `fallido`. |
| `motivo` | cadena | Descripción legible del fallo; `""` si `resultado` es `completado`. **Nunca lleva ninguna credencial del protocolo ni ningún contenido de mensaje.** |

El contrato de A-2 describe `ruta_de_la_copia`, `bytes` y `motivo` como campos «presentes solo
si…». La regla 4 de la sección 1 —todos los campos siempre presentes— **no contradice** esa
condicionalidad: la expresa con el valor vacío en lugar de con la omisión del campo, para que el
analizador escrito a mano del otro extremo no tenga que tratar campos opcionales. La condición
semántica es la misma; cambia solo cómo se codifica la ausencia.

Quién ejecuta la copia no cambia por existir este protocolo: **siempre el proceso del sidecar**,
con `VACUUM INTO` sobre sus propias conexiones. El núcleo nunca abre el archivo del `sqlstore`, ni
siquiera de solo lectura.

---

## 8. Errores de protocolo

Un error de protocolo es cualquiera de estos: versión que no coincide, `tipo` desconocido, línea
que no es un objeto JSON válido, campo ausente, campo desconocido, valor que no es cadena ni
entero, valor anidado, o línea que supera el límite de la sección 1.

Ante cualquiera de ellos, el extremo que lo detecta **cierra la conexión y registra el hecho**; no
intenta reencuadrar el flujo ni saltarse la línea. Una vez que el delimitado por líneas es dudoso,
seguir leyendo es adivinar. Cerrar y reconectar recupera un punto de sincronización conocido, y la
sección 4 garantiza que nada se pierde: lo no confirmado sigue en el outbox.

El registro de un error de protocolo lleva el tipo de error y, como mucho, el nombre del campo
ofensor; **nunca la línea recibida**, que podría contener el texto de un mensaje (`adr-0019`).

---

## 9. Qué queda deliberadamente fuera de este documento

* **El esquema del outbox durable** y su retención y purga: tarea 3. Aquí se fija la semántica que
  debe cumplir, no sus tablas.
* **La calibración real de los valores por omisión del retroceso de reconexión.** La versión 1.2
  declara las variables y la forma del algoritmo; los números son pendientes de calibración bajo
  tráfico real.
* **La traducción de los eventos de WhatsApp y el mapeo de identidad (tareas 8 y 9).** HEX-014 cubre la mitad entrante de la tarea 8 (el mensaje hacia `evento_entrante`) y el mapeo completo de identidad (tarea 9). `evento_entrante` se persiste en el outbox antes de su entrega al sumidero, siguiendo la convención de persistir primero.
* **El almacén de identidad.** Mapea los contactos anclados en el JID de número de teléfono hacia identificadores internos opacos, guardando el LID como un alias, en su propio archivo SQLite en `/var/lib/hexcell/identidad.db`, separado del `sqlstore`.
* **Cómo analiza estas líneas el lado Rust** —a mano o con una dependencia nueva—: tarea 10, con la
  decisión registrada en `adr-0011`.
* **La dirección saliente y los acuses.** Se implementaron en la versión 1.3 de este documento (`mensaje_saliente` y `acuse_envio` en la sección 6) mediante la tarea 12 de la etapa A-3.
* **El emparejamiento por QR y por código de vinculación**, que la versión 1.0 omitía, queda
  cubierto desde la versión 1.1 por los tres tipos `orden_emparejar`, `codigo_emparejamiento` y
  `acuse_emparejamiento`.

---

## Referencias

* `docs/plan/fase-a-3-adaptador-whatsmeow.md`: tareas 1 a 3, 6 a 10 y 18, y sus criterios.
* `docs/contrato-ipc-respaldo-del-sqlstore.md`: contrato de la copia del `sqlstore` (sección 7).
* `docs/adr/adr-0010-puerto-de-canal.md`: el puerto como frontera y el JID que no la cruza.
* `docs/adr/adr-0014-canal-propio-permanente.md`: el sidecar como coste permanente.
* `docs/adr/adr-0016-convencion-de-entrega-de-eventos.md`: la convención de entrega al `Motor`.
* `docs/adr/adr-0019-registro-estructurado.md`: registro sin serializador y el conjunto de campos
  como mecanismo de privacidad.
* `crates/hexcell-core/src/canal.rs`: `EventoEntrante`, `ChannelAdapter` y `CicloDeVidaSesion`, tal
  y como están declarados hoy.
* `docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md`: ADR que registrará esta decisión, por escribir.
