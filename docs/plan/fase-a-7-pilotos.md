# Fase A · Etapa 7 — Células piloto y compuerta de salida

**Duración relativa:** Media (semanas de operación con poco trabajo de ingeniería).

---

## Objetivo

Todas las piezas existen: la célula procesa mensajes reales por WhatsApp, se protege de las ráfagas,
controla su gasto, actualiza su conocimiento sin detenerse, está aislada, se respalda y se puede
gobernar desde la CLI. Falta lo único que justifica todo lo anterior: **averiguar si esto es un
negocio**.

Esta etapa no construye software. Pone en marcha **dos células piloto** y las opera durante el tiempo
necesario para responder tres preguntas que ninguna prueba automatizada puede contestar:

1. ¿El bot **responde bien**? No "responde": responde de forma que el dueño del negocio no se
   avergüence de que sus clientes lo lean.
2. ¿Los **clientes finales lo usan**? Un bot que funciona perfectamente y con el que nadie habla no
   valida nada.
3. ¿El piloto **paga**? En presente y en indicativo. La versión anterior de este plan preguntaba si
   el piloto *pagaría*, y esa pregunta no vale nada: responder "sí" a un amigo que te enseña su
   proyecto no cuesta dinero, cuesta cortesía. **El acto de pagar es la métrica; la declaración de
   intención no es evidencia de nada.**

Las dos células tienen **funciones distintas y asimétricas**, y confundirlas es la forma más rápida
de auto-engañarse:

* **piloto-01** — negocio de prueba del propio dueño del proyecto. Es un **banco de pruebas
  técnico**, y solo eso. Sirve para descubrir fallos de operación, ensayar el re-emparejamiento y
  medir estabilidad, consumo y coste. **Sus datos NUNCA cuentan para la validación de negocio.** El
  dueño no puede ser su propio cliente: no se cobra a sí mismo, no se queja a sí mismo y no abandona
  el servicio si es malo. Cualquier métrica de negocio extraída de piloto-01 mide entusiasmo propio,
  no demanda.
* **piloto-02** — negocio de un conocido, y **la única fuente de validación de negocio de toda la
  Fase A**. Todo el peso de la decisión de compuerta descansa sobre esta única célula, lo cual es
  poco pero es honesto; fingir que son dos sería peor.

La compuerta, además, se **pre-registra**: los umbrales numéricos se fijan por escrito **antes** de
dar de alta a nadie, junto con los **criterios de fracaso** que cierran o rediseñan el proyecto. Una
compuerta cuyos umbrales se deciden después de ver los datos no es una compuerta, es una
racionalización.

Ambas usan un **número de WhatsApp nuevo y dedicado**. Esta condición no es negociable y merece
enunciarse con claridad: el canal es no oficial y WhatsApp puede desactivar el número sin aviso ni
apelación. Si ese número fuera el número principal del negocio, un ban destruiría la línea comercial
del cliente. Con un número dedicado, un ban cuesta un número desechable y una conversación
incómoda, no el negocio de nadie.

Al final de esta etapa está la **compuerta**: la decisión de si el negocio está validado y si, por
tanto, el tercer cliente dispara la Fase B. Con los umbrales ya escritos, esa decisión consiste en
comparar, no en interpretar.

---

## Alcance

### Qué entra

* Alta de **piloto-01**: aprovisionamiento de la célula, adquisición del número nuevo dedicado,
  emparejamiento de la sesión whatsmeow por QR o código, carga del conocimiento inicial del negocio y
  verificación de extremo a extremo con mensajes reales.
* Alta de **piloto-02**, con la misma secuencia, una vez piloto-01 acumule operación estable.
* Procedimiento de alta documentado y repetible, apoyado en los comandos de la etapa A-6. No hay
  `cell create` de una sola invocación: el alta de la Fase A es un procedimiento operado, no
  automatizado, porque con dos células automatizarlo no se paga.
* **Expectativas pactadas por escrito con cada piloto**, antes de empezar:
  * El canal es no oficial. WhatsApp puede romper el protocolo y dejar el bot mudo durante días o
    semanas, hasta que la comunidad publique el arreglo.
  * El número puede ser baneado. Si ocurre, se pierde **el número dedicado**, no el principal del
    negocio.
  * El servicio es una validación, no un producto contratado. No hay compromiso de disponibilidad.
  * **Para piloto-02: a partir del segundo mes se cobra un importe simbólico pero real**, acordado de
    antemano en el mismo documento.
* **Compuerta pre-registrada:** umbrales numéricos y criterios de fracaso fijados por escrito antes
  de dar de alta a ningún piloto.
* **Cobro simbólico pero real a piloto-02 desde el segundo mes.** No es una fuente de ingresos —el
  importe es simbólico— sino el **instrumento de medida**: el único dato que distingue el interés
  cortés de la demanda real es una transferencia ejecutada.
* Definición y recogida de las **métricas de validación del negocio**, tanto cuantitativas como
  cualitativas, recogidas **exclusivamente de piloto-02** en su dimensión de negocio.
* Operación diaria durante el periodo del piloto: vigilancia de la sesión, respuesta a desconexiones,
  actualizaciones de conocimiento, revisión de los descartes del GCRA y de la desviación de la
  contabilidad LLM.
* Retroalimentación al plan: los registros de descartes, el consumo real de tokens y el consumo real
  de memoria alimentan la calibración de los parámetros de las etapas A-4 y A-6.
* **Evaluación de la compuerta de salida** y su decisión documentada.

### Qué NO entra

* Cualquier tercer cliente. El tercero es, por definición, el que dispara la Fase B; no se da de alta
  sobre canal no oficial.
* Comercialización a escala, contratos de servicio y facturación automatizada. El cobro simbólico a
  piloto-02 se gestiona a mano, por transferencia o el medio que resulte natural; montar facturación
  para un cliente sería construir infraestructura para validar que no hace falta construirla.
* **El uso de datos de piloto-01 como evidencia de negocio.** Está excluido explícitamente, no por
  descuido: piloto-01 es banco de pruebas técnico.
* Automatización del alta. Con dos células no se justifica.
* El plano de control de la Fase B: Caddy, subdominios y Embedded Signup pertenecen a la etapa B-2.

### Requisitos del PRD cubiertos

* Cierre operativo de **FR-01**, **FR-12** y **FR-02** sobre células reales en producción.
* Validación empírica de la calibración de **FR-08** y **FR-10** con tráfico y gasto reales.

---

## Entregables

* Dos células piloto operativas, `piloto-01` y `piloto-02`, con su conocimiento cargado y su sesión
  de canal estable.
* `docs/runbook-alta-piloto.md`: procedimiento de alta paso a paso, incluidos la adquisición del
  número, el emparejamiento y la carga de conocimiento inicial.
* Documento de expectativas pactadas, firmado o aceptado explícitamente por cada piloto.
* `docs/pilotos/compuerta-preregistrada.md`: **documento fechado y cerrado antes del alta de
  piloto-01**, con los umbrales numéricos de éxito, los criterios de fracaso y la ventana temporal de
  evaluación. Su fecha de creación es parte del entregable: un pre-registro escrito después de ver
  los datos no es un pre-registro.
* `docs/pilotos/metricas-validacion.md`: definición de las métricas, su método de recogida y el
  registro periódico de sus valores, con la separación explícita entre las técnicas (ambas células) y
  las de negocio (solo piloto-02).
* Comprobante del **primer cobro efectivo a piloto-02**, o constancia escrita de su negativa a pagar.
  Ambos resultados son datos válidos; la ausencia de intento de cobro no lo es.
* Informe de la compuerta: estado de cada métrica, decisión sobre la validación del negocio y, si
  procede, disparo de la Fase B.
* Lista de ajustes de calibración derivados de la operación real, con destino a las etapas
  correspondientes.

---

## Tareas

1. **Pre-registrar la compuerta: umbrales de éxito y criterios de fracaso** (1,5 días). Antes de dar
   de alta a nadie, fijar por escrito **qué números** constituyen validación y **qué números**
   constituyen fracaso, con su ventana temporal. Los valores concretos los fija el dueño; lo que este
   plan exige es que existan y estén fechados antes del primer alta. Estructura de partida, con los
   parámetros marcados para que el dueño los cierre:

   *Umbrales de éxito (todos medidos sobre **piloto-02**, salvo los técnicos):*
   * *Uso real:* **≥ N conversaciones por semana** iniciadas por clientes finales, **sostenidas
     durante 4 semanas consecutivas**. No vale una semana buena.
   * *Calidad:* **≥ 70 %** de conversaciones resueltas sin intervención humana.
   * *Retención de clientes finales:* **≥ R %** de clientes que vuelven a escribir en el periodo.
   * *Pago:* **el cobro del segundo mes se ejecuta y el piloto lo paga sin renegociar a la baja.**
   * *Coste (técnico, ambas células):* gasto en LLM y embeddings por conversación **≤ C**, coherente
     con cualquier precio que el negocio pueda sostener.
   * *Estabilidad (técnico, ambas células):* **≥ E %** de tiempo con la sesión de canal activa; número
     de re-emparejamientos necesarios por debajo de un máximo.

   *Criterios de fracaso (kill criteria) — si se cumple cualquiera, el proyecto se cierra o se
   rediseña, y no se pasa a la Fase B:*
   * **piloto-02 se niega a pagar** el importe simbólico, o lo cancela tras el primer cobro.
   * El uso real se queda **por debajo de la mitad del umbral** durante 4 semanas consecutivas.
   * La proporción de respuestas que el dueño del negocio marca como inadecuadas **supera un techo
     acordado**, es decir: el bot avergüenza al cliente delante de sus clientes.
   * El coste por conversación **excede lo que cualquier precio plausible podría cubrir**.
   * El canal resulta inoperativo **más de X semanas acumuladas** por roturas de protocolo o bans.

   Un criterio de fracaso escrito de antemano es lo único que impide seguir invirtiendo por inercia.
2. **Redactar y pactar las expectativas con los pilotos** (0,5 días). Documento breve, en lenguaje
   llano, con los riesgos asumidos. **No se da de alta a ningún piloto sin esta aceptación
   explícita**, y muy especialmente a piloto-02, que no es el dueño del proyecto. Para piloto-02, el
   documento incluye **el importe simbólico y la fecha del primer cobro** (inicio del segundo mes),
   acordados antes de empezar y no negociados después.
3. **Adquirir y preparar los números dedicados** (0,5 días). Un número nuevo por célula, verificado y
   sin historial. Ninguno puede ser el número principal de un negocio.
4. **Dar de alta piloto-01 como banco de pruebas técnico** (1 día). Aprovisionamiento de la célula,
   emparejamiento, carga de conocimiento inicial y prueba de extremo a extremo con mensajes reales.
   Queda registrado desde el primer día que **esta célula no produce evidencia de negocio**.
5. **Operar piloto-01 en solitario y ensayar la recuperación** (varias semanas, con vigilancia diaria
   ligera). Detectar y corregir lo que solo aparece en producción antes de exponer a un tercero, y
   **ejecutar al menos una vez el re-emparejamiento por `PairPhone()`** y una restauración completa
   desde respaldo. Es el único momento en que un fallo de recuperación no cuesta credibilidad.
6. **Dar de alta piloto-02** (1 día). Misma secuencia, con el runbook ya endurecido por la
   experiencia de piloto-01, y con el acuerdo de cobro ya firmado.
7. **Operar ambas células y recoger métricas** (periodo de validación, con revisión semanal).
   Registro sistemático de las métricas de la tarea 1, manteniendo separadas las técnicas (ambas
   células) de las de negocio (**solo piloto-02**).
8. **Ejecutar el primer cobro a piloto-02** (0,5 días, al inicio del segundo mes). Emitir el cobro
   acordado y registrar el resultado: pagado, pagado tras renegociar a la baja, o impagado. **Este
   dato pesa más que todo lo demás**, y por eso el cobro se ejecuta aunque el proyecto vaya bien y dé
   pereza pedir dinero a un conocido. No ejecutarlo invalida la validación entera.
9. **Revisar la calibración con datos reales** (1 día). Contrastar los descartes del GCRA, la
   desviación entre reserva y conciliación, y el consumo de memoria de la célula, frente a los
   valores supuestos en las etapas A-4 y A-6. Ajustar y documentar.
10. **Evaluar la compuerta y decidir** (1 día). Contrastar los valores medidos contra los umbrales
    **pre-registrados en la tarea 1**, sin reinterpretarlos a la luz de lo ocurrido. Documentar la
    decisión: negocio validado y Fase B disparada por el tercer cliente; continuar validando si
    ningún criterio de fracaso se cumple pero los umbrales aún no se alcanzan; o **cerrar o rediseñar
    el proyecto** si se cumple cualquier criterio de fracaso. Si se decide continuar pese a un
    criterio de fracaso cumplido, la excepción se documenta **con su justificación y su nueva fecha
    de revisión**, para que quede constancia de que se está eligiendo ignorar la evidencia.

---

## Criterios de aceptación

* Ambas células están operativas con números nuevos y dedicados, y **ningún número principal de un
  negocio se ha usado en ningún momento**.
* Cada piloto ha aceptado explícitamente el documento de expectativas antes del alta, y el de
  piloto-02 incluye el importe y la fecha del primer cobro.
* **El documento de compuerta pre-registrada existe, está fechado y su fecha es anterior al alta de
  piloto-01.** Contiene umbrales numéricos concretos y criterios de fracaso, no adjetivos.
* El runbook de alta permite dar de alta una célula sin consultar a quien escribió el código.
* Las métricas de validación están definidas, se recogen de forma periódica y tienen valores
  registrados para todo el periodo de operación, **con las de negocio atribuidas exclusivamente a
  piloto-02**.
* **Ningún dato procedente de piloto-01 aparece como evidencia en el informe de compuerta**, salvo en
  su dimensión técnica (estabilidad, coste, consumo).
* **El cobro a piloto-02 se ha ejecutado realmente** al inicio del segundo mes, y su resultado
  —pagado, renegociado a la baja o impagado— está registrado. Una compuerta evaluada sin haber
  intentado cobrar se declara **no concluyente**.
* La restauración de un respaldo real de una célula piloto sobre un entorno limpio se ha ejecutado al
  menos una vez, con éxito, durante el periodo de validación, **terminando en un bot que reconecta y
  responde**. No se espera a que haga falta.
* El re-emparejamiento por `PairPhone()` se ha ensayado sobre piloto-01 **antes** del alta de
  piloto-02.
* Los parámetros de GCRA, del presupuesto LLM y de los límites de memoria han sido revisados contra
  datos reales, y los ajustes están documentados.
* Existe un informe de compuerta con una decisión explícita, tomada **comparando los valores medidos
  contra los umbrales pre-registrados**, no reinterpretándolos. Si se continúa pese a un criterio de
  fracaso cumplido, la excepción está documentada con su justificación y su fecha de revisión.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| **Ban del número de un piloto.** | Medio: se pierde continuidad y confianza, no el negocio del cliente. | Números nuevos y dedicados, calentamiento y política de solo responder de la etapa A-3, y expectativa pactada por escrito de antemano. Procedimiento de sustitución de número documentado en el runbook. |
| **Rotura del protocolo de WhatsApp** durante el piloto. | Alto para la percepción del piloto: el bot enmudece. | Expectativa pactada de posibles semanas de silencio; dependencia fijada y actualizable en un paso; comunicación proactiva al piloto en cuanto se detecta, sin esperar a que pregunte. |
| El piloto-02 se siente experimento y abandona. | Alto: se pierde la única validación externa. | Expectativas honestas desde el principio, comunicación proactiva ante incidentes, y una carga de conocimiento inicial suficientemente buena como para que el bot aporte valor desde el primer día. |
| Las métricas se definen después de empezar a operar. | Alto: se mide lo que se puede en lugar de lo que importa, y la compuerta se decide por impresión. | La compuerta se **pre-registra** en la tarea 1, con umbrales numéricos y criterios de fracaso fechados antes del primer alta. |
| **Sesgo de cortesía: el piloto-02 dice que le gusta y que pagaría, porque es un conocido.** | Muy alto: es el modo de fallo más probable de toda la Fase A, y produce una validación falsa que arrastra a la Fase B y a un gasto real. | **Mitigado por construcción:** la métrica de pago no es una declaración sino **un cobro ejecutado** desde el segundo mes. Nadie paga por cortesía todos los meses. La negativa a pagar es un criterio de fracaso explícito, no una señal a interpretar. |
| **Juez y parte: el dueño evalúa su propio piloto.** | Muy alto: piloto-01 siempre "funciona bien" porque quien lo mide es quien lo construyó y no tiene alternativa a la que irse. | **Mitigado por construcción:** piloto-01 queda degradado a banco de pruebas técnico y **sus datos no cuentan para la validación de negocio**, con criterio de aceptación que lo verifica en el informe de compuerta. |
| Se llega al momento del cobro y da pereza pedir dinero a un conocido. | Muy alto: se pierde el único dato que de verdad valida, y se sustituye por la impresión que se quería evitar. | El cobro es la tarea 8, con fecha pactada por escrito desde antes del alta, y una compuerta evaluada sin intento de cobro se declara **no concluyente**. |
| Los umbrales se reinterpretan a la baja al ver los resultados. | Alto: la compuerta se convierte en una racionalización de lo que ya se quería hacer. | Pre-registro fechado y anterior al alta; cualquier excepción debe documentarse como tal, con justificación y fecha de revisión. |
| Se cede a la tentación de dar de alta un tercer cliente sobre el canal no oficial. | Muy alto: se comercializa sobre un canal que viola los ToS y que puede desaparecer. | La compuerta es explícita en el PRD y en este plan: **el tercer cliente dispara la Fase B**, no se suma a la Fase A. |
| **Modelo de monetización sin definir** (pendiente en STATUS.md). | Medio: el importe simbólico no equivale a un precio de mercado. | El cobro simbólico no pretende fijar el precio: valida que existe **disposición real a pagar algo**, que es el paso previo. El precio se calibra después, con el coste por conversación de esta misma etapa como suelo. |
| Toda la validación de negocio descansa sobre una única célula. | Alto: N=1 es poca base para una decisión de inversión. | Es una limitación asumida y declarada, no un descuido: es preferible una fuente honesta a dos de las cuales una es el propio dueño. Se compensa exigiendo señales **sostenidas en el tiempo** (4 semanas consecutivas) y un pago recurrente, no un instante favorable. |
| Los datos conversacionales de clientes finales de un negocio ajeno se pierden. | Muy alto: daño reputacional y de confianza irreparable. | Respaldo de las tres bases de la etapa A-2 operativo desde el primer día del piloto, con restauración verificada —hasta que el bot responde— durante el periodo, y re-emparejamiento por `PairPhone()` ya ensayado. |

---

## Dependencias

* **De otras etapas:** etapa A-6 completa. Sin imágenes, composición de célula y CLI de operación, el
  alta no debe intentarse.
* **Externas:** dos números de WhatsApp nuevos y dedicados; un teléfono disponible para el
  emparejamiento; y un conocido dispuesto a servir de piloto-02 **aceptando por escrito las
  expectativas y el cobro simbólico desde el segundo mes**. Si no acepta el cobro antes de empezar,
  no es un piloto de validación: es un usuario gratuito, y no aporta la evidencia que esta etapa
  necesita.
* **Decisiones de producto pendientes:** la **lógica de negocio específica** y los **flujos de usuario
  finales** determinan qué responde el bot. Esta etapa los aborda de la única forma honesta que
  existe: descubriéndolos con negocios reales en lugar de suponerlos. El **modelo de monetización**
  recibe de aquí su primera entrada empírica, que no es una opinión sino un cobro ejecutado.
* **Del dueño del proyecto, antes de la tarea 1 y de forma bloqueante:** los valores concretos de los
  umbrales (N conversaciones semanales, porcentaje de resolución, retención, coste máximo por
  conversación, disponibilidad mínima), el importe simbólico del cobro y los techos de los criterios
  de fracaso. El plan fija la **estructura** de la compuerta; los números son decisión de negocio.
