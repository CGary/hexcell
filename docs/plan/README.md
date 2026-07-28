# Plan de Implementación por Fases — HexCell Orchestrator

> Documento índice. Última actualización: 2026-07-28.
> Fuentes normativas: [PRD.md](../PRD.md) (requisitos FR/NFR y criterios de QA), [README.md](../../README.md) (arquitectura y CLI), [STATUS.md](../STATUS.md) (avance).

---

## 1. Visión general

HexCell es un orquestador multi-célula que despliega bots de WhatsApp para microempresas sobre
hardware local modesto. El plan que sigue traduce en una secuencia de etapas ejecutables lo que ya
está documentado en el proyecto, sin añadir requisitos de cosecha propia. Se apoya en dos fuentes con
rango distinto:

* El **PRD es la fuente normativa**. Cada etapa declara explícitamente qué requisitos funcionales
  (FR) y no funcionales (NFR) cubre, y entre todas se cubren FR-01 a FR-12 y NFR-01 a NFR-05.
* El **README.md aporta detalle operativo** que el PRD no recoge, principalmente el flujo de
  onboarding con Meta Embedded Signup y `override_callback_uri` y el comando de eliminación
  definitiva de una célula (ambos en la etapa B-2). Esos elementos van marcados con una nota de
  fuente en la etapa correspondiente. Ante cualquier contradicción entre ambos documentos, manda el
  PRD.

### La estructura de dos fases

El plan no es una secuencia lineal de ocho etapas hacia el canal oficial, pero **tampoco son dos
fases separadas por una compuerta de negocio**: desde el cambio de rumbo del 28 de julio de 2026 son
**dos canales que conviven**. Los nombres de archivo (`fase-a-N-*.md`, `fase-b-N-*.md`) y la
numeración de las etapas se conservan intactos; lo que cambió es su significado.

* **Fase A — Canal propio en producción.** Canal propio mediante la biblioteca **whatsmeow** sobre un
  websocket saliente: sin webhook, sin IP pública, sin Caddy, sin TLS entrante y sin handshake
  anti-Hairpin. Es el canal **por defecto y permanente**, con clientes de pago reales encima.
  `piloto-01` y `piloto-02` son las dos primeras células, no el alcance total. Docker desde el primer
  día. Siete etapas.
* **Fase B — Canal oficial adicional.** Canal oficial mediante la **Meta Cloud API** con webhooks.
  Tres etapas. Sigue **CONGELADA**, pero ya no la dispara un número de clientes: se activa **cuando
  aparece un cliente que la justifique**, típicamente una empresa medianamente grande que pueda
  asumir el alta y el coste del canal oficial. Cuando llegue, **se suma** al canal propio: no lo
  sustituye, no lo cierra y no retira ningún sidecar. Sus etapas declaran alcance, criterios y
  dependencias, pero **no se estiman en detalle todavía**. Además, la mitad del alcance de la etapa
  B-2 depende de una decisión —la entrada pública— que se toma al principio de la etapa B-1.

**La compuerta del tercer cliente queda derogada**, junto con la regla de que no se comercializa
sobre canal no oficial. Lo que ahora disciplina el crecimiento son **compuertas de riesgo**, no de
validación: un **techo duro de cartera** mientras el canal propio sea el único y un **umbral de
incidentes que congela altas** cuando se supera una tasa de baneos. Ambas viven en la etapa
[A-7](fase-a-7-pilotos.md), y sus valores numéricos son decisiones de negocio pendientes.

Lo que hace posible que ambos canales convivan sin duplicar el producto es el **puerto de canal**
(`ChannelAdapter`, FR-12), declarado en la primera etapa del plan: un trait del núcleo Rust que
normaliza el evento entrante, el envío, la identidad de conversación y los acuses. Todo lo construido
sobre ese puerto —persistencia, control de admisión, presupuesto, conocimiento, aislamiento— sirve
igual a las células de uno y otro canal. Incorporar el canal oficial debe ser escribir un segundo
adaptador que **coexista** con el primero, no reescribir el producto, y la etapa B-1 tiene entre sus
criterios de aceptación demostrarlo.

### Las ideas que gobiernan el orden

**Nada se conecta a un canal real hasta que el componente que lo consume sabe protegerse a sí
mismo.** Es la misma idea que gobernaba el plan anterior, aplicada a un canal distinto: el control de
admisión (GCRA) y la contabilidad financiera se construyen antes de que haya un piloto real, porque
un consumo sin techo satura un servidor doméstico igual venga por websocket que por webhook.

**El conocimiento (RAG) es un subsistema con su propio ciclo de vida.** Necesita persistencia estable
debajo, así que se aborda después del núcleo de datos, pero antes del empaquetado en contenedor,
porque el diseño del volumen de disco y de los puntos de montaje depende de cómo se materialicen las
épocas de conocimiento en el sistema de archivos.

**Los umbrales se pre-registran y solo el cliente externo valida.** Los umbrales numéricos y los
criterios de fracaso se fijan por escrito antes de dar de alta a ningún piloto, y piloto-01 —el
negocio del propio dueño— queda degradado a banco de pruebas técnico: sus datos no cuentan como
evidencia de negocio, porque nadie es cliente de sí mismo. La métrica de disposición a pagar es un
**cobro simbólico pero real** ejecutado a piloto-02 desde el segundo mes, no una declaración de
intenciones.

**Los respaldos no esperan al final.** En el plan anterior vivían en la última etapa. Ahora están en
la etapa A-2, antes incluso de que exista el canal real, porque con pilotos operando sobre datos
conversacionales de clientes finales de un negocio ajeno, un disco que falla no es un incidente
técnico: es la pérdida de la confianza que la validación necesita. Y cubren **tres** bases, no dos:
la sesión del canal vive en el `sqlstore` del sidecar, y restaurar el historial con la sesión muerta
deja al bot mudo con todos sus datos intactos.

### Nomenclatura mínima

Definiciones de los términos que se repiten a lo largo del plan:

* **Célula** (*cell*): la unidad desplegable por cliente. Una microempresa corresponde a una célula.
  Sobre canal propio son dos contenedores (núcleo Rust y sidecar Go) que comparten red local y
  volumen; sobre canal oficial es un solo contenedor, más un subdominio y una cuenta de WhatsApp
  Business. Ambos tipos de célula conviven en el mismo servidor. Es el término
  que sustituye a la nomenclatura de arrendamiento que usaba la versión anterior de este plan. En la
  CLI y en el código el sustantivo es `cell`.
* **Puerto de canal** (`ChannelAdapter`): trait del núcleo Rust que aísla el dominio de cualquier
  transporte de WhatsApp. Normaliza el evento entrante canónico, el envío, la identidad de
  conversación mapeada a un identificador interno y los acuses. Es la frontera de **coexistencia**
  entre canales: sostiene dos adaptadores vivos a la vez, en células distintas (FR-12).
* **whatsmeow**: biblioteca Go que implementa el protocolo no oficial de WhatsApp Web mediante un
  websocket saliente. Es el adaptador del **canal propio**, permanente y por defecto.
* **Sidecar**: proceso auxiliar que acompaña al núcleo dentro de una célula. Alberga la sesión
  whatsmeow, porque no existe equivalente maduro en Rust. Añade unos 15-30 MB de RAM y es **coste
  permanente** de toda célula sobre canal propio; una célula sobre canal oficial no lo lleva.
* **Emparejamiento** (*pairing*): vinculación de la sesión whatsmeow a un número de WhatsApp mediante
  código QR o código de emparejamiento. Sus credenciales se persisten para no repetirlo en cada
  reinicio.
* **Compuertas de riesgo**: los dos frenos que sustituyen a la derogada compuerta del tercer cliente.
  Un **techo duro de cartera** —número máximo de células sobre canal propio mientras sea el único— y
  un **umbral de incidentes que congela altas** si se supera una tasa de baneos. Sus valores son
  decisiones de negocio pendientes; ambos se fijan en la etapa A-7.
* **WABA**: *WhatsApp Business Account*, la cuenta de Meta a la que se asocian los números de
  teléfono y las suscripciones de webhook de un cliente. Solo aplica en las células sobre canal
  oficial.
* **Webhook**: petición HTTP que Meta envía a nuestro servidor cuando ocurre un evento. Solo aplica en
  las células sobre canal oficial.
* **GCRA** (*Generic Cell Rate Algorithm*): algoritmo de control de tasa que decide, con una sola
  marca temporal por clave y sin cerrojos, si un evento se admite o se descarta. Opera sobre el flujo
  normalizado del puerto de canal, no sobre HTTP.
* **Fast-Reject**: patrón por el cual respondemos `HTTP 200 OK` inmediato a una petición que no vamos
  a procesar, para que Meta la dé por entregada y no la reintente. Solo aplica en la Fase B: en la
  Fase A no hay petición entrante que contestar.
* **Shadow DB**: base de datos en sombra donde se compila el nuevo conocimiento sin tocar la que
  está sirviendo tráfico en producción.
* **Época**: versión inmutable y numerada de la base de conocimiento (`knowledge_epoch_N.db`).
* **Blackholing**: sustitución temporal del proxy inverso por una respuesta estática, de modo que
  el tráfico se absorbe sin llegar a ningún backend. Solo aplica en la Fase B.
* **Drenaje controlado** (*Graceful Drain*): cierre ordenado de un pool de conexiones antiguo,
  esperando a que terminen las operaciones en vuelo antes de liberar los descriptores de archivo.

---

## 2. Tabla de etapas

### Fase A — Canal propio en producción (whatsmeow)

| Nº | Nombre | Objetivo (una línea) | FR / NFR cubiertos | Depende de |
| :-- | :--- | :--- | :--- | :--- |
| A-1 | [Fundaciones del repositorio](fase-a-1-fundaciones.md) | Dejar el repositorio, la licencia, el workspace Rust y la CI listos, y declarar el puerto de canal. | FR-12 (declaración), FR-01 (tipos) | — |
| A-2 | [Núcleo de la célula: mensajería y persistencia dual](fase-a-2-nucleo-persistencia.md) | Construir el motor de mensajería sobre el puerto de canal, con tests de contrato contra el caso restrictivo y respaldo de las tres bases. | FR-01, FR-05, FR-12, NFR-01 (parcial) | A-1 |
| A-3 | [Adaptador whatsmeow: sidecar Go y puerto de canal](fase-a-3-adaptador-whatsmeow.md) | Conectar la célula a WhatsApp de verdad, con outbox durable, sesión persistente y disciplina anti-ban. | FR-01 (Fase A), FR-12 (implementación) | A-2 |
| A-4 | [Control de admisión y presupuesto](fase-a-4-admision-presupuesto.md) | Impedir que ráfagas de mensajes o el coste del LLM desestabilicen el sistema. | FR-08, FR-09, FR-10 | A-2 (y A-3 para medir con tráfico real) |
| A-5 | [Motor de conocimiento: Shadow DB y épocas](fase-a-5-conocimiento-shadow-db.md) | Actualizar el conocimiento del bot sin detener la producción ni corromper el WAL. | FR-06, FR-07, NFR-03 | A-2 (y A-4 para el coste de embeddings) |
| A-6 | [Empaquetado de la célula y CLI de operación](fase-a-6-empaquetado-cli.md) | Convertir núcleo y sidecar en una célula contenedorizada gobernable desde la CLI, con alertas push y dead-man's switch. | FR-02, FR-11 (Fase A), NFR-01, NFR-05 | A-2, A-3, A-4, A-5 |
| A-7 | [Células piloto y compuertas de riesgo](fase-a-7-pilotos.md) | Operar piloto-01 (banco de pruebas técnico) y piloto-02 (validación de negocio, con cobro real), y fijar el techo duro de cartera y el umbral de incidentes que congela altas. | Cierre operativo de FR-01, FR-02, FR-12; calibración de FR-08 y FR-10 | A-6 |

### Fase B — Canal oficial adicional (Meta Cloud API) · **CONGELADA hasta que un cliente la justifique**

| Nº | Nombre | Objetivo (una línea) | FR / NFR cubiertos | Depende de |
| :-- | :--- | :--- | :--- | :--- |
| B-1 | [Canal oficial: adaptador Cloud API y entrada pública](fase-b-1-canal-oficial.md) | Escribir el segundo adaptador del puerto y decidir la entrada pública, incorporando el canal oficial de forma **aditiva**: las células sobre canal propio siguen operando sin cambios. | FR-01 (canal oficial), FR-08 (Fast-Reject), FR-12 | A-2 (puerto de canal) y la aparición de un cliente que justifique el canal oficial |
| B-2 | [Plano de control y onboarding comercial](fase-b-2-plano-de-control-onboarding.md) | Gobernar rutas, certificados y altas de clientes de pago sin exponer errores 502 a Meta. | FR-03, FR-04, FR-11 (Fase B), NFR-02, NFR-04 | B-1 (y su ADR de entrada pública) |
| B-3 | [Endurecimiento, QA y operación comercial](fase-b-3-endurecimiento-qa.md) | Demostrar con pruebas medibles que se cumplen los criterios de aceptación y los NFR. | NFR-01 a NFR-05, verificación cruzada de FR-02, FR-07, FR-08, FR-12 | B-2 |

Las etapas de la Fase B **no llevan estimación**. Su alcance, sus criterios de aceptación y sus
dependencias están escritos para poder responder con conocimiento de causa el día que un cliente
pida el canal oficial, no para planificar trabajo que quizá no se haga. Además, la etapa B-2 depende de una decisión abierta
—Cloudflare Tunnel frente a VPS con WireGuard— que altera la mitad de su contenido.

---

## 3. Justificación de la secuencia

El orden no es arbitrario; cada salto responde a una dependencia técnica concreta.

**A-1 → A-2.** No se puede escribir código de producción sin un workspace, una licencia y una CI que
impida que la primera semana de trabajo se convierta en deuda. Y sobre todo: el puerto de canal se
declara antes que cualquier otra cosa, porque es la frontera que hace que el resto del trabajo
sobreviva al cambio de fase. Declararlo después sería declararlo tarde.

**A-2 → A-3.** El núcleo se construye íntegramente contra un adaptador simulado. No es un rodeo: es
la única forma de garantizar que no se acopla al transporte, porque durante toda la etapa A-2 el
transporte no existe. Cuando llega el sidecar whatsmeow, sustituye al simulado sin tocar el núcleo.

**A-2 → A-4.** El control de admisión y la contabilidad financiera operan *sobre* un flujo de eventos
y *sobre* un estado persistente. Necesitan que exista el pipeline y las tablas de saldo antes de
poder interponerse en ellos. La etapa A-3 conviene tenerla también, para calibrar con tráfico real en
lugar de solo simulado.

**A-4 → A-5.** El motor de conocimiento consume APIs externas de embeddings, que cuestan dinero.
Tener antes la contabilidad de dos fases permite que la ingesta por lotes se someta al mismo
presupuesto que el resto de llamadas externas, en lugar de convertirse en un agujero de gasto sin
instrumentar.

**A-2, A-3, A-4, A-5 → A-6.** La composición de la célula y su diseño de volúmenes solo pueden
fijarse cuando se sabe qué archivos existen en disco (dos bases activas, la de staging, las épocas
históricas y las credenciales de sesión del sidecar) y qué recursos consumen los dos procesos en
reposo. Empaquetar antes obliga a rehacer los `Dockerfile` en cada iteración.

**A-6 → A-7.** Dar de alta a un piloto real exige poder pausarlo, reactivarlo, respaldarlo y darlo de
baja. Poner un negocio ajeno en producción sin capacidad de operarlo es el peor orden posible.

**A-7 → B-1.** Aquí no hay dependencia técnica sino de demanda. La Fase B no se inicia porque la Fase
A esté técnicamente terminada ni porque se alcance un número de clientes: se inicia el día que
aparece un cliente que justifique el canal oficial y pueda asumir su alta y su coste. Mientras no
aparezca, el canal propio sigue siendo el modo de producción, y lo que gobierna cuántas células se
dan de alta son las compuertas de riesgo fijadas en A-7, no el paso a la Fase B.

**B-1 → B-2.** La decisión de entrada pública, que es la primera tarea de B-1, determina si el
handshake anti-Hairpin y el On-Demand TLS de Caddy existen o desaparecen. Planificar B-2 antes de esa
decisión es planificar la mitad de un trabajo que quizá no haga falta.

**B-2 → B-3.** Los criterios de QA del PRD son pruebas de sistema completo. Requieren células
comerciales reales desplegadas para ser significativas.

---

## 4. Cómo leer el plan

Cada archivo de etapa sigue la misma estructura, pensada para que un desarrollador pueda tomarla y
empezar sin leer las demás:

* **Objetivo** — por qué existe la etapa, en prosa.
* **Alcance** — qué entra, qué queda explícitamente fuera y qué FR/NFR del PRD cubre.
* **Entregables** — artefactos concretos que quedan en el repositorio al terminar.
* **Tareas** — lista ordenada; en la Fase A cada tarea está dimensionada entre medio día y dos días de
  trabajo. En la Fase B las tareas se enumeran sin estimación.
* **Criterios de aceptación** — comprobaciones verificables, ligadas a los criterios de QA del PRD
  cuando aplica.
* **Riesgos y mitigaciones**.
* **Dependencias** — de otras etapas y de decisiones externas.

### Sobre las decisiones de producto pendientes

STATUS.md registra varios asuntos sin resolver: el modelo de monetización, los flujos de usuario
finales, el manejo de excepciones comerciales y el proceso exacto de alta de una microempresa. Este
plan **no los resuelve**, porque no son decisiones de ingeniería. Aparecen en las etapas que los
necesitan bajo el epígrafe de dependencias externas o de riesgos, con una indicación clara de qué
parte del trabajo queda bloqueada mientras no exista una respuesta:

* **Modelo de monetización** — bloquea la calibración de los saldos y de la política de degradación
  en la etapa A-4, y el criterio de suspensión por falta de pago en la etapa B-2. La etapa A-7 le
  aporta su primera entrada empírica: la disposición a pagar de los pilotos y el coste real por
  conversación.
* **Proceso exacto de onboarding y flujos de usuario** — bloqueaban por completo el alta en el plan
  anterior. Ahora la etapa A-7 los aborda de la única forma honesta que existe: descubriéndolos con
  dos negocios reales en lugar de suponerlos. El alta comercial automatizada sigue bloqueada en la
  etapa B-2.
* **Manejo de excepciones comerciales** — condiciona el comportamiento del modo degradado en la
  etapa A-4 y el alcance de la lógica de negocio del bot, que este plan trata como fuera de alcance
  hasta que exista definición.
* **Entrada pública de la Fase B** — decisión de arquitectura pendiente, no de producto. Es la primera
  tarea de la etapa B-1 y condiciona la mitad de la B-2.
* **Techo duro de cartera y umbral de incidentes que congela altas** — decisiones de negocio
  pendientes desde el cambio de rumbo del 28 de julio de 2026. Sustituyen a la derogada compuerta del
  tercer cliente y se fijan por escrito en la etapa A-7, antes de dar de alta a ningún cliente de
  pago. Este plan no propone cifras.

### Estimación de duración

Las etapas de la Fase A no llevan fechas absolutas, porque el equipo aún no está dimensionado. Cada
una declara una **duración relativa** en una escala de tres niveles (Corta, Media, Larga) derivada de
la suma de sus tareas. La escala sirve para planificar capacidad, no para comprometer entregas.

Las etapas de la Fase B **no se estiman**. Están congeladas hasta que aparezca un cliente que
justifique el canal oficial.
