# ADR-0015 — Política de convivencia con el riesgo de baneo del canal propio

* **Estado:** Vigente desde el 2026-07-28.
* **Depende de:** `adr-0014-canal-propio-permanente.md`, que convierte el canal propio en canal de
  producción permanente y hace obligatoria esta política.
* **Etapa:** A-3, con alcance transversal a las etapas A-2, A-6 y A-7.
* **Requisitos tocados:** FR-02, FR-11, FR-12, NFR-05.

---

## Contexto

`adr-0014` deja el canal propio como canal de producción permanente, con clientes de pago encima, y
sustituye la compuerta del tercer cliente por compuertas de riesgo. Eso obliga a escribir **qué
postura tiene el proyecto frente al baneo**, con rango de decisión y no de lista de tareas.

La premisa que ordena todo lo demás está registrada en `adr-0014`: **el riesgo de baneo es en buena
medida estructural.** Meta identifica la biblioteca por su huella de protocolo, los baneos alcanzan a
cuentas de bajo volumen y solo-respuesta, y una parte sustancial se decide de forma automática y sin
aviso. De ahí se sigue la única conclusión de diseño que importa:

> **El baneo se documenta como evento esperado, no como fallo.** Las medidas que reducen la
> probabilidad actúan sobre el término secundario del problema; **las que reducen el daño son las que
> tienen más valor por unidad de coste.**

Este ADR fija las **cuatro capas de defensa** y su jerarquía. Las tareas concretas, sus criterios de
aceptación y su reparto por etapas viven en `docs/plan/`; aquí solo se decide qué se hace, qué no se
hace y por qué.

## Decisión

Se adopta un modelo de **cuatro capas**, ordenadas de menor a mayor valor esperado: reducir la
probabilidad, detectar pronto, contener el daño y recuperar. **La Capa 3 es la de mayor valor por
coste de todo el plan** y ninguna medida de la Capa 1 puede usarse como argumento para relajarla.

Cada medida de la Capa 1 se marca como **[causa documentada]** —hay una razón pública o un mecanismo
verificable que la respalda— o **[precautorio]** —es plausible pero no está demostrada—. La distinción
es obligatoria y no decorativa: impide que una corazonada barata se convierta con el tiempo en una
defensa creída.

### Capa 1 — Reducir la probabilidad

1. **Invariante solo-respuesta impuesto por tipos, no solo por test** [causa documentada]. Un
   `Outbound` solo debe poder construirse a partir del identificador de un evento entrante válido. Un
   test se puede saltar; un constructor privado, no. Refuerza el invariante ya existente en lugar de
   sustituirlo.
2. **TTL absoluto en la cola de salida y reintentos idempotentes** [causa documentada]. Este es el
   vector real de violación del invariante: un reintento, o un reencolado tras reinicio, entrega una
   respuesta horas más tarde y **para el receptor parece una iniciación de conversación**. Se decide
   descarte duro al superar el TTL medido desde la marca temporal del evento entrante, reintentos
   acotados y **ninguna cola de mensajes muertos que reencole al arrancar**.
3. **Drenaje sin envío** al pausar, migrar o eliminar una célula [causa documentada].
4. **Latencia mínima de respuesta y horario de atención configurable** [causa documentada].
   Responder en menos de un segundo a las cuatro de la madrugada es la señal no humana más barata de
   emitir por accidente.
5. **Emitir el indicador de "escribiendo" antes de responder** [precautorio, con el matiz de más
   abajo].
6. **Variar la plantilla del mensaje de presentación del bot** [causa documentada]. Un texto idéntico
   repetido a cientos de destinatarios es una señal bastante más plausible que la del indicador de
   escritura.
7. **Lista de exclusión (STOP) persistente por célula y contacto** [causa documentada]: efecto
   inmediato, sin caducidad, con precedencia sobre cualquier otra regla y una única confirmación de
   baja.
8. **Identificación como bot y salida a humano ofrecida en el primer turno** [causa documentada]. Los
   reportes de usuarios son una de las tres familias de señales oficiales de Meta.
9. **Un mensaje por turno; nunca grupos, listas de difusión ni estados** [causa documentada].
10. **Cortacircuitos conversacional** [causa documentada]: ante repetición o frustración detectada, el
    bot cede a un humano **y calla, pero emitiendo un único mensaje de traspaso**. Callar en seco
    aumenta los bloqueos, que son una señal peor que el propio silencio.
11. **Higiene del número** [precautorio]: SIM física con antigüedad y uso previo, **a nombre del
    cliente**; nunca número virtual, VoIP ni SIM recién activada; perfil de negocio completo.
12. **El teléfono primario del dueño debe seguir en uso humano real** [precautorio]. Un primario
    inerte cuyo único tráfico sale del dispositivo enlazado es un patrón anómalo.
13. **Rampa de volumen** durante las primeras semanas de cada célula [precautorio].
14. **whatsmeow pinneado por commit, con ventana de actualización definida** [precautorio]. Correr
    atrasado tiene doble riesgo: se deja de conectar por `Client outdated (405)` y se declara una
    versión de cliente atípica.

#### Corrección documentada sobre el indicador de "escribiendo"

El whitepaper oficial *"Stopping Abuse: How WhatsApp Fights Bulk Messaging and Automated Behavior"*
(WhatsApp, 2019-02-06), sección *While Messaging*, dice literalmente:

> *"If an account continually sends messages without triggering the typing indicator, it can be a
> signal of abuse, and we will ban the account."*

La frase aparece en un párrafo propio sobre mecanismos que apuntan **directamente a la
automatización**, separado del párrafo de volumen (el que habla de "100 mensajes en 15 segundos").

**Se decide redactarlo siempre con este matiz exacto:** emitir el indicador de "escribiendo" es
**higiene documentada de coste cero, no una defensa**. El documento tiene siete años, es anterior a la
arquitectura multi-dispositivo de 2021, no existe versión actualizada, no hay evidencia pública de su
eficacia, y su propio razonamiento —que los emisores masivos "puede que no tengan capacidad técnica
de falsificarlo"— se debilita cuando falsificarlo cuesta una línea de código. Se emite porque es
gratis, no porque proteja.

**Todo lo que se vende alrededor —jitter, "calentamiento" de cuenta con protocolo de pasos y plazos—
es folclore de proveedores de envío masivo y no entra en esta documentación como medida.**

### Capa 2 — Detectar pronto

**Esta capa acorta la reacción; no evita el baneo.** Debe decirse con todas las letras cada vez que se
describa, porque el error clásico es tomar un panel por una defensa.

* **Instrumentar cada variante de desconexión por separado**: `LoggedOut` con su razón, baneo temporal
  con su expiración, `StreamReplaced`, fallo de conexión con su código. **Colapsarlas en un único
  estado "desconectado" destruye la señal** y es el error más caro de esta capa.
* **Ratio de acuses de entrega segmentado por contacto** —detección indirecta de bloqueos— y latencia
  hasta el acuse.
* Reconexiones por hora, y ventana de silencio entrante: cero mensajes recibidos en X horas hábiles
  cuando históricamente hay tráfico.
* **Lo que no es observable:** cuántos usuarios han reportado el número. **Esa señal no existe y
  ningún panel debe fingir que la tiene.**
* **El baneo temporal es alerta de máxima prioridad**: suele ser el único aviso previo que hay.

### Capa 3 — Contener el daño

Es la capa de mayor valor por coste y la que sustituye a la compuerta derogada en `adr-0014`.

* **Techo duro de cartera** mientras el canal propio sea el único canal en producción. **El número
  concreto es decisión de negocio pendiente** y debe fijarse por escrito antes del alta del primer
  cliente de pago.
* **Umbral de incidentes que congela altas:** superada una tasa de baneos, no se da de alta ninguna
  célula nueva hasta analizar la causa. **Su valor numérico es igualmente decisión de negocio
  pendiente.**
* **El cliente es siempre el titular del número y de la SIM; HexCell nunca lo es.** Es quien puede
  apelar, y así el baneo no cruza hacia la identidad del proveedor.
* **Contrato que declara el canal como propio y no oficial**, con el riesgo de baneo explícito, sin
  garantía de disponibilidad y con un modo degradado pactado por escrito.
* **Aislamiento estricto por célula** —contenedor, volumen, socket y `sqlstore` propios—, sin
  credenciales ni procesos compartidos salvo el orquestador (FR-02, NFR-05).
* **Canary de biblioteca:** una célula centinela propia, con número propio, corre la versión candidata
  de whatsmeow durante 72 horas antes de escalonar la actualización al resto. **Nunca se actualiza
  toda la cartera el mismo día.**
* **No se usan proxies, VPN ni rotación de IP.** Las direcciones de centro de datos son señal antispam
  directa; la salida residencial del servidor local es el perfil benigno.

### Capa 4 — Recuperar

* **Clasificador de incidente escrito**, con una rama por caso: desconexión transitoria, baneo
  temporal con expiración, baneo permanente y desvinculación hecha por el propio dueño.
* **Ante un baneo temporal, no reconectar en bucle:** retroceso exponencial largo, célula en pausa y
  esperar la expiración. **Persistir con el cliente no oficial durante un baneo temporal escala el
  baneo a permanente** (`faq.whatsapp.com/1848531392146538`); migrar a la app oficial restaura el
  acceso al expirar.
* **Apelación desde la app oficial en el teléfono del titular**, dentro de las primeras horas y con
  guion redactado de antemano. Solo el dueño del número puede presentarla.
* **Plantilla de comunicación al cliente en menos de una hora**, con qué se pierde, qué no y cuál es
  el modo degradado. Escrita antes de la crisis, no durante.
* **Re-emparejamiento con `PairPhone()` ensayado y cronometrado en el alta de cada cliente.** Exige al
  dueño con el teléfono delante: si no se ha practicado, el tiempo de recuperación lo fija su agenda,
  no el código.
* **Regla de restauración del `sqlstore`:** ante `LoggedOut` con `device_removed`, whatsmeow **borra
  la sesión él mismo** y restaurar el respaldo es inútil, porque el dispositivo ya no existe en el
  servidor. **No toda desconexión implica `device_removed`.** Regla exacta: *no restaurar el
  `sqlstore` solo si hubo `LoggedOut` con `device_removed`*; el respaldo sigue siendo plenamente
  válido ante corrupción o fallo de disco.
* **Verificar que la continuidad del hilo sobrevive al re-emparejamiento:** tras obtener un nuevo
  identificador de dispositivo, el mismo contacto debe mapear al mismo hilo en `sessions.db`. Es lo
  que el puerto de canal (FR-12) debía garantizar, y **hay que probarlo, no asumirlo**.
* **Simulacro completo antes del primer cliente de pago:** baneo simulado, restauración,
  re-emparejamiento y bot respondiendo, con cronómetro. Criterio de éxito: **el bot reconecta y
  responde**. Nunca "el archivo existe".

### Experimento registrado, no medida

**Meta Verified.** Varios usuarios del issue #810 de `tulir/whatsmeow` reportaron que activarlo en la
cuenta de WhatsApp Business detuvo los avisos de *"unauthorized tools"*. Es correlación anecdótica de
2025, sin confirmación de Meta. Se registra como **experimento a ensayar en `piloto-01`** y **nunca se
documenta como medida probada** ni se contabiliza como defensa.

## Lo que NO hay que hacer

Queda escrito para que nadie lo reintroduzca más adelante como idea nueva:

* Proxies, VPN o rotación de IP.
* Parchear whatsmeow para camuflar su huella: no funciona —la detección es multiseñal— y saca al
  proyecto del flujo de actualizaciones, que sí importa.
* Números virtuales o SIM recién comprada.
* Cualquier mensaje proactivo "útil": recordatorios, seguimientos, encuestas, "¿sigues ahí?".
* Reconexión agresiva tras un baneo temporal.
* Un número maestro compartido entre clientes o a nombre de HexCell.
* Reactivar automáticamente una célula baneada sin decisión humana de por medio.
* Prometer disponibilidad sobre el canal propio.
* Creer que la Capa 2 evita baneos: **solo acorta el tiempo de reacción**.

## Consecuencias

### Positivas

* El proyecto deja de tratar el baneo como accidente y lo trata como escenario previsto, con
  clasificación, procedimiento y comunicación al cliente escritos antes de la crisis.
* La marca **[causa documentada] / [precautorio]** impide que el folclore de proveedores de envío
  masivo se cuele en el diseño con apariencia de rigor.
* El techo de cartera y el umbral de incidentes reponen el freno de crecimiento que se perdió al
  derogar la compuerta, ahora ligado al riesgo real y no a un número arbitrario de clientes.

### Negativas

* **Ninguna de estas capas elimina el riesgo estructural.** En conjunto reducen la probabilidad de
  forma no cuantificable y acotan el daño de forma sí verificable; prometer más sería falso.
* El coste operativo por alta sube: ensayo cronometrado de `PairPhone()`, higiene de número,
  contrato específico y simulacro previo al primer cliente de pago.
* El canary de biblioteca y la actualización escalonada obligan a mantener una célula centinela con
  número propio, que consume memoria y atención sin generar ingresos.
* El techo de cartera limita deliberadamente los ingresos mientras el canal propio sea el único, que
  es justamente su función y también su coste.

## Referencias

* `adr-0014-canal-propio-permanente.md`: decisión que hace obligatoria esta política; contiene la
  evidencia de baneos (issues #810, #807 y #989 de `tulir/whatsmeow`) y el riesgo de mantenimiento
  (bus factor 1, `Client outdated (405)`, issues #415 y #1031).
* `adr-0009-whatsmeow-adaptador-fase-a.md` y `adr-0011-whatsmeow-sidecar-e-ipc.md`: la política
  anti-ban no desactivable por configuración del sidecar implementa las medidas de Capa 1 que le
  corresponden.
* `adr-0010-puerto-de-canal.md` y FR-12: continuidad del hilo tras el re-emparejamiento.
* `docs/PRD.md`: FR-02 y NFR-05 (aislamiento por célula), FR-11 (suspensión sin errores hacia el
  canal), criterio de QA "Prueba de Recuperación de Sesión (Fase A)".
* `docs/STATUS.md`: los valores numéricos del techo de cartera y del umbral de congelación de altas
  se registran como decisión de negocio pendiente, anterior al alta del primer cliente de pago.
* `docs/plan/`: reparto de tareas por etapas (A-2 respaldos, A-3 sidecar y capas 1 y 4, A-6 alertas y
  observabilidad de la Capa 2, A-7 simulacro y umbrales).
