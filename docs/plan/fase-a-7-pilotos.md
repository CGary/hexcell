# Fase A · Etapa 7 — Células piloto y compuertas de riesgo

**Duración relativa:** Media (semanas de operación, precedidas del trabajo documental y contractual
que exige el primer cliente de pago, y con poco trabajo de ingeniería).

---

## Objetivo

Todas las piezas existen: la célula procesa mensajes reales por WhatsApp, se protege de las ráfagas,
controla su gasto, actualiza su conocimiento sin detenerse, está aislada, se respalda y se puede
gobernar desde la CLI. Falta lo único que justifica todo lo anterior: **averiguar si esto es un
negocio**.

Esta etapa no construye software, pero sí produce el material contractual y operativo sin el cual no
puede existir un primer cliente de pago. Pone en marcha **dos células piloto** y las opera durante el
tiempo necesario para responder tres preguntas que ninguna prueba automatizada puede contestar:

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
* **piloto-02** — negocio de un conocido, y **la única fuente de validación de negocio de esta
  etapa**. Es también el **primer cliente de pago**, y por eso todo el material contractual y de
  recuperación debe existir antes de su alta. Todo el peso de la decisión de compuerta descansa sobre
  esta única célula, lo cual es poco pero es honesto; fingir que son dos sería peor.

La compuerta, además, se **pre-registra**: los umbrales numéricos se fijan por escrito **antes** de
dar de alta a nadie, junto con los **criterios de fracaso** que cierran o rediseñan el proyecto. Una
compuerta cuyos umbrales se deciden después de ver los datos no es una compuerta, es una
racionalización.

Lo que cambió el 28 de julio de 2026 (`adr-0014`) no es la disciplina del pre-registro, sino **qué
decide la compuerta**. Ya no decide si se cambia de canal: el canal propio (whatsmeow) pasa a ser el canal de
producción por defecto y no tiene sustituto programado. La compuerta decide ahora si **el producto
sigue adelante** y si **se abren más altas**. Los umbrales de éxito y los criterios de fracaso
conservan íntegro su valor; lo que queda derogado es su antigua consecuencia.

Ambas usan un **número de WhatsApp dedicado**, distinto del número principal del negocio. Esta
condición no es negociable y merece enunciarse con claridad: el canal es propio y no oficial, y
WhatsApp puede desactivar el número sin aviso previo. Si ese número fuera el número principal del
negocio, un baneo destruiría la línea comercial del cliente. Con un número dedicado, un baneo cuesta
un número y una conversación incómoda, no el negocio de nadie. Ese número, además, **es siempre del
cliente**: suya es la SIM y suya la titularidad, y por eso es él quien puede apelar.

Al final de esta etapa está la **compuerta**: la decisión de si el negocio está validado y, en
consecuencia, si se abren más altas sobre el canal propio —hasta el techo duro de cartera vigente—,
si se sigue validando, o si el proyecto se cierra o se rediseña. Con los umbrales ya escritos, esa
decisión consiste en comparar, no en interpretar.

---

## Alcance

### Qué entra

* Alta de **piloto-01**: aprovisionamiento de la célula, adquisición del número dedicado conforme a
  la higiene del número de la tarea 6, emparejamiento de la sesión whatsmeow por QR o código, carga
  del conocimiento inicial del negocio y verificación de extremo a extremo con mensajes reales.
* Alta de **piloto-02**, con la misma secuencia, una vez piloto-01 acumule operación estable.
* Procedimiento de alta documentado y repetible, apoyado en los comandos de la etapa A-6. No hay
  `cell create` de una sola invocación: el alta de la Fase A es un procedimiento operado, no
  automatizado, porque con dos células automatizarlo no se paga.
* **Expectativas pactadas por escrito con cada piloto**, antes de empezar, y **actualizadas para
  recoger el riesgo de baneo con todas sus letras**:
  * El canal es **propio y no oficial**. WhatsApp puede romper el protocolo y dejar el bot mudo
    durante días o semanas, hasta que la comunidad publique el arreglo.
  * **El número puede ser baneado, sin aviso previo, y ninguna medida de comportamiento elimina ese
    riesgo**: es en buena medida estructural, porque Meta detecta la biblioteca por su huella de
    protocolo. El baneo se declara **evento esperado, no fallo**. Si ocurre, se pierde **el número
    dedicado**, no el principal del negocio.
  * El servicio es una validación, no un producto contratado. No hay compromiso de disponibilidad.
  * **Para piloto-02: a partir del segundo mes se cobra un importe simbólico pero real**, acordado de
    antemano en el mismo documento.
* **Compuerta pre-registrada:** umbrales numéricos y criterios de fracaso fijados por escrito antes
  de dar de alta a ningún piloto.
* **Compuertas de riesgo de cartera**, que sustituyen a la derogada compuerta del tercer cliente:
  * **Techo duro de cartera** mientras el canal propio sea el único canal: número máximo de células
    vivas por encima del cual no se da ninguna alta más. El número concreto es **decisión de negocio
    pendiente**; este plan solo exige que esté escrito antes de la primera alta.
  * **Umbral de incidentes de baneo que congela las altas**: superada una tasa de incidentes en una
    ventana temporal, **no se da de alta ninguna célula nueva hasta analizar la causa**. El umbral y
    la ventana son **decisión de negocio pendiente**.
* **Material obligatorio antes del primer cliente de pago**, procedente de las capas de contención y
  recuperación de `adr-0015`. Ninguno de estos documentos admite escribirse durante la crisis que los
  hace necesarios:
  * **Contrato con el cliente**, que declara el canal como **propio y no oficial**, con el **riesgo
    de baneo explícito**, **sin garantía de disponibilidad** y con el **modo degradado pactado** —qué
    deja de funcionar, qué sigue funcionando y qué se hace mientras dure—. Requiere **revisión legal
    local**: una cláusula de exoneración inejecutable frente a una microempresa es **peor que
    ninguna**, porque genera falsa seguridad en quien la firma y en quien la redacta.
  * **Política de titularidad del número**: el cliente es **siempre** el titular del número y de la
    SIM; **HexCell nunca**. Es quien puede apelar, y así el baneo no cruza a la identidad del
    proveedor. Incluye la **higiene del número** —SIM física con antigüedad y uso previo, a nombre
    del cliente; nunca número virtual, ni VoIP, ni SIM recién activada; perfil de negocio completo— y
    la obligación de que **el teléfono primario del dueño siga en uso humano real**: un primario
    inerte cuyo único tráfico sale del dispositivo enlazado es un patrón anómalo.
  * **Runbook de baneo**, con un **clasificador de incidente** y una rama por caso: desconexión
    transitoria, baneo temporal con expiración, baneo permanente y desvinculación por el propio
    dueño. Incluye la **prohibición de reconectar en bucle ante un baneo temporal** —persistir con el
    cliente no oficial durante la suspensión escala el baneo a permanente—, el **guion de apelación
    desde la app oficial en el teléfono del titular**, redactado de antemano porque solo el dueño
    puede presentarla y solo sirve en las primeras horas, la **plantilla de comunicación al cliente
    en menos de una hora**, escrita antes de la crisis y no durante, y el **procedimiento de
    sustitución de número**, con su criterio de procedencia, lo que se conserva y lo que se pierde,
    los pasos operativos apoyados en `cell rebind` (etapa A-6), quién debe estar presente y el
    **aviso a los contactos que tenían guardado el número viejo**, que solo puede dar el cliente.
  * **Ensayo cronometrado del re-emparejamiento con `PairPhone()` en el alta de CADA cliente**, no
    solo del primero. La razón es operativa y conviene dejarla escrita: el re-emparejamiento **exige
    al dueño con el teléfono delante**, de modo que, si no se ha practicado, el tiempo de
    recuperación no lo fija el código sino la agenda de esa persona.
  * **Simulacro completo antes del primer cliente de pago**: baneo simulado, restauración,
    re-emparejamiento y bot respondiendo, todo con cronómetro. Criterio de éxito: **el bot reconecta
    y responde**. Nunca "el archivo existe".
  * **Experimento de Meta Verified sobre piloto-01**, registrado con honestidad como lo que es:
    varios usuarios del issue #810 de `tulir/whatsmeow` reportaron que activarlo en la cuenta
    Business detuvo los avisos de *"unauthorized tools"*. Es **correlación anecdótica de 2025, sin
    confirmación de Meta y sin mecanismo conocido**: se ensaya como experimento y se registra su
    resultado, **nunca se documenta como medida probada**.
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

* **La expansión de la cartera más allá de los dos pilotos.** Ya no está prohibida: la regla de que
  ningún tercer cliente se da de alta sobre el canal propio queda **derogada el 28 de julio de 2026**,
  y el tercer cliente ya no dispara nada. Simplemente no pertenece a esta etapa; las altas
  posteriores se gobiernan por el **techo duro de cartera** y por el **umbral de incidentes que
  congela altas**, no por el cierre de una fase.
* Comercialización a escala y facturación automatizada. El cobro simbólico a
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
* `docs/runbook-baneo.md`: clasificador de incidente con sus cuatro ramas, regla de no reconexión ante
  baneo temporal, guion de apelación, plantilla de comunicación al cliente y **procedimiento de
  sustitución de número** —con su criterio de procedencia, lo conservado y lo perdido, los pasos
  operativos, las personas necesarias y la plantilla de aviso que el cliente difunde por sus propios
  canales—, todos redactados antes del primer incidente.
* Documento de expectativas pactadas, firmado o aceptado explícitamente por cada piloto, **con el
  riesgo de baneo recogido con todas sus letras**.
* `docs/contrato-canal-propio.md`: contrato de cliente que declara el canal como propio y no oficial,
  con el riesgo de baneo, la ausencia de garantía de disponibilidad y el modo degradado pactado.
  Incluye la **constancia escrita de su revisión legal local**, o de que no la ha tenido y por qué.
* `docs/politica-titularidad-numero.md`: titularidad del número y de la SIM siempre en el cliente,
  higiene del número y obligación de mantener el teléfono primario del dueño en uso humano real.
* `docs/pilotos/simulacro-baneo.md`: informe del simulacro completo —baneo simulado, restauración,
  re-emparejamiento y bot respondiendo—, con los tiempos cronometrados de cada tramo.
* Registro del **experimento de Meta Verified** sobre piloto-01, con su resultado y con la
  calificación explícita de la evidencia como anecdótica.
* `docs/pilotos/compuerta-preregistrada.md`: **documento fechado y cerrado antes del alta de
  piloto-01**, con los umbrales numéricos de éxito, los criterios de fracaso, la ventana temporal de
  evaluación y las **compuertas de riesgo de cartera** (techo duro y umbral de incidentes que congela
  altas). Su fecha de creación es parte del entregable: un pre-registro escrito después de ver los
  datos no es un pre-registro.
* `docs/pilotos/metricas-validacion.md`: definición de las métricas, su método de recogida y el
  registro periódico de sus valores, con la separación explícita entre las técnicas (ambas células) y
  las de negocio (solo piloto-02).
* Comprobante del **primer cobro efectivo a piloto-02**, o constancia escrita de su negativa a pagar.
  Ambos resultados son datos válidos; la ausencia de intento de cobro no lo es.
* Informe de la compuerta: estado de cada métrica, decisión sobre la validación del negocio y, si
  procede, apertura de más altas sobre el canal propio dentro del techo duro de cartera.
* Lista de ajustes de calibración derivados de la operación real, con destino a las etapas
  correspondientes.

---

## Tareas

1. **Pre-registrar la compuerta: umbrales de éxito, criterios de fracaso y compuertas de riesgo**
   (1,5 días). Antes de dar de alta a nadie, fijar por escrito **qué números** constituyen validación
   y **qué números** constituyen fracaso, con su ventana temporal. Los valores concretos los fija el
   dueño; lo que este plan exige es que existan y estén fechados antes del primer alta. Estructura de
   partida, con los parámetros marcados para que el dueño los cierre:

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
   rediseña, y no se abre ninguna alta nueva:*
   * **piloto-02 se niega a pagar** el importe simbólico, o lo cancela tras el primer cobro.
   * El uso real se queda **por debajo de la mitad del umbral** durante 4 semanas consecutivas.
   * La proporción de respuestas que el dueño del negocio marca como inadecuadas **supera un techo
     acordado**, es decir: el bot avergüenza al cliente delante de sus clientes.
   * El coste por conversación **excede lo que cualquier precio plausible podría cubrir**.
   * El canal resulta inoperativo **más de X semanas acumuladas** por roturas de protocolo o baneos.

   *Compuertas de riesgo de cartera (sustituyen a la compuerta del tercer cliente, derogada):*
   * **Techo duro de cartera:** **máximo de M células vivas** mientras el canal propio sea el único
     canal. Por encima, no se da ninguna alta.
   * **Umbral de incidentes:** **más de I incidentes de baneo en una ventana de V semanas congela
     todas las altas** hasta que se analice la causa y se documente la conclusión.

   Un criterio de fracaso escrito de antemano es lo único que impide seguir invirtiendo por inercia.
2. **Redactar y pactar las expectativas con los pilotos** (0,5 días). Documento breve, en lenguaje
   llano, con los riesgos asumidos y **con el riesgo de baneo escrito con todas sus letras**, incluido
   que es en buena medida estructural y que puede llegar sin aviso previo. **No se da de alta a
   ningún piloto sin esta aceptación explícita**, y muy especialmente a piloto-02, que no es el dueño
   del proyecto. Para piloto-02, el documento incluye **el importe simbólico y la fecha del primer
   cobro** (inicio del segundo mes), acordados antes de empezar y no negociados después.
3. **Redactar el contrato de cliente sobre canal propio y enviarlo a revisión legal local** (1 día de
   redacción, más el plazo de la revisión, que no depende de este proyecto). Declara el canal como
   propio y no oficial, el riesgo de baneo, la ausencia de garantía de disponibilidad y el modo
   degradado pactado. La revisión legal no es un trámite: se hace porque una cláusula de exoneración
   inejecutable frente a una microempresa genera falsa seguridad, y eso es peor que no tener
   cláusula. Debe estar listo **antes del primer cobro a piloto-02**.
4. **Escribir la política de titularidad e higiene del número** (0,5 días). Titularidad del número y
   de la SIM siempre en el cliente y nunca en HexCell; SIM física con antigüedad y uso previo, nunca
   virtual, VoIP ni recién activada; perfil de negocio completo; y el teléfono primario del dueño en
   uso humano real.
5. **Escribir el runbook de baneo** (1,5 días). Clasificador de incidente con una rama por caso
   —desconexión transitoria, baneo temporal con expiración, baneo permanente, desvinculación por el
   propio dueño—, prohibición explícita de reconectar en bucle ante baneo temporal, guion de
   apelación desde la app oficial en el teléfono del titular y plantilla de comunicación al cliente
   en menos de una hora. Todo redactado antes del primer incidente: durante la crisis no se redacta,
   se ejecuta.

   El runbook incluye además el **procedimiento de sustitución de número**, que es la salida de la
   rama más grave y por eso no puede quedar implícito. Consta de cinco partes:

   * *Cuándo procede y cuándo no.* Procede ante **baneo permanente** o ante **apelación fracasada**.
     **No procede ante un baneo temporal**: ahí se espera a la expiración con la célula en pausa, y
     sustituir el número antes de tiempo tira un número recuperable y estrena otro sin necesidad.
     Tampoco procede mientras la apelación siga viva, porque una apelación concedida devuelve el
     número original con toda su antigüedad, que es precisamente lo que el número nuevo no tiene.
   * *Qué se conserva y qué se pierde.* Se conservan la célula, su conocimiento, el historial de
     `sessions.db` y el almacén de identidad del adaptador —identidad de conversación y lista de
     exclusión (STOP)—, de modo que cada contacto sigue cayendo en su hilo y el bot no olvida a
     nadie. Se pierden el número, el `sqlstore` del sidecar —que corresponde a un dispositivo que ya
     no existe en el servidor de WhatsApp— y, sobre todo, **el alcance**: quien tuviera guardado el
     número viejo deja de llegar al bot hasta que alguien se lo diga.
   * *Pasos operativos.* Contratar o activar el número de reemplazo a nombre del cliente conforme a
     la higiene de la tarea 6 —usando la SIM de reserva si existe—, ejecutar `cell rebind` (etapa
     A-6) con su confirmación explícita, completar el emparejamiento por QR o por `PairPhone()`,
     verificar que **el bot responde por el número nuevo y reconoce a un contacto anterior**
     —criterio de éxito; "el archivo existe" no cuenta—, y levantar la pausa de envío solo entonces.
   * *Quién debe estar presente.* El **dueño del número**, con su teléfono delante: es el titular de
     la SIM y sin él no hay emparejamiento posible. Y el **operador de HexCell**, que ejecuta el
     comando. Se acuerda una franja con el cliente antes de empezar, porque el tiempo de
     recuperación lo fija su disponibilidad, no el código.
   * *Aviso a los contactos que tenían guardado el número viejo.* Es un paso del procedimiento, no
     una cortesía posterior. **El aviso no puede salir del sistema:** la cuenta baneada está muerta y
     cualquier intento de enviar desde ella es exactamente lo que escala un baneo temporal a
     permanente; y emitirlo desde el número nuevo sería una iniciación de conversación en masa, que
     el invariante de solo-responder prohíbe y que es la forma más rápida de quemar también el
     reemplazo. Lo da **el cliente por sus propios medios** —su número principal, su rótulo, sus
     redes, su web—, y el runbook le entrega redactada la **plantilla de aviso** para que no tenga
     que improvisarla: qué número deja de funcionar, cuál lo sustituye, desde cuándo y que sus
     conversaciones anteriores no se han perdido.
6. **Adquirir y preparar los números dedicados** (0,5 días). Un número dedicado por célula, **a
   nombre del cliente**, sobre SIM física con antigüedad y uso previo, con perfil de negocio
   completo. Ninguno puede ser el número principal de un negocio, ni un número virtual o VoIP, ni una
   SIM recién activada.

   En la misma alta se contrata además una **SIM de reserva a nombre del cliente, que empieza a
   envejecer desde el día uno** [precautorio]. El razonamiento es el de la propia regla de higiene y
   no va más allá: si la sustitución de número solo se activa tras un baneo permanente, y la higiene
   exige que la SIM tenga antigüedad y uso previo, entonces una SIM comprada el día del incidente
   **entra más débil que la que sustituye** y abre la puerta a encadenar baneos. Una reserva que
   lleva meses activa rompe esa cadena. Va marcada **[precautorio]** y no [causa documentada]: **no
   hay evidencia publicada de su eficacia**, solo la coherencia con una regla que a su vez es
   precautoria, y documentarla de otro modo sería exactamente el folclore que `adr-0015` excluye.
   Tiene un **coste recurrente por cliente** —una línea que se paga y no se usa—, y su repercusión al
   cliente queda ligada al **modelo de monetización**, que sigue siendo decisión de negocio pendiente
   en `docs/STATUS.md`.
7. **Dar de alta piloto-01 como banco de pruebas técnico** (1 día). Aprovisionamiento de la célula,
   emparejamiento, carga de conocimiento inicial y prueba de extremo a extremo con mensajes reales.
   Queda registrado desde el primer día que **esta célula no produce evidencia de negocio**.
8. **Operar piloto-01 en solitario y ensayar la recuperación** (varias semanas, con vigilancia diaria
   ligera). Detectar y corregir lo que solo aparece en producción antes de exponer a un tercero, y
   **ejecutar al menos una vez el re-emparejamiento por `PairPhone()`, cronometrado,** y una
   restauración completa desde respaldo. Es el único momento en que un fallo de recuperación no
   cuesta credibilidad.
9. **Ejecutar el simulacro completo de baneo** (1 día), **antes del primer cliente de pago**. Baneo
   simulado sobre piloto-01, restauración, re-emparejamiento y bot respondiendo, con cronómetro en
   cada tramo. El simulacro solo se da por superado cuando **el bot reconecta y responde**; "el
   archivo existe" no es un resultado.
10. **Activar Meta Verified en piloto-01 como experimento** (0,5 días, más el plazo de verificación
    de Meta). Se registra la fecha de activación y si cambian los avisos de *"unauthorized tools"*.
    El resultado se documenta como **señal anecdótica**, positivo o negativo, y no se promueve a
    medida del plan en ningún caso.
11. **Dar de alta piloto-02** (1 día). Misma secuencia, con el runbook ya endurecido por la
    experiencia de piloto-01, con el contrato y el acuerdo de cobro ya firmados, y con el
    **re-emparejamiento por `PairPhone()` ensayado y cronometrado en esta misma alta**: se ensaya en
    cada alta, no una sola vez en el proyecto.
12. **Operar ambas células y recoger métricas** (periodo de validación, con revisión semanal).
    Registro sistemático de las métricas de la tarea 1, manteniendo separadas las técnicas (ambas
    células) de las de negocio (**solo piloto-02**).
13. **Ejecutar el primer cobro a piloto-02** (0,5 días, al inicio del segundo mes). Emitir el cobro
    acordado y registrar el resultado: pagado, pagado tras renegociar a la baja, o impagado. **Este
    dato pesa más que todo lo demás**, y por eso el cobro se ejecuta aunque el proyecto vaya bien y
    dé pereza pedir dinero a un conocido. No ejecutarlo invalida la validación entera.
14. **Revisar la calibración con datos reales** (1 día). Contrastar los descartes del GCRA, la
    desviación entre reserva y conciliación, y el consumo de memoria de la célula, frente a los
    valores supuestos en las etapas A-4 y A-6. Ajustar y documentar.
15. **Evaluar la compuerta y decidir** (1 día). Contrastar los valores medidos contra los umbrales
    **pre-registrados en la tarea 1**, sin reinterpretarlos a la luz de lo ocurrido. Documentar la
    decisión: negocio validado y **apertura de más altas sobre el canal propio**, dentro del techo
    duro de cartera y siempre que el umbral de incidentes no esté superado; continuar validando si
    ningún criterio de fracaso se cumple pero los umbrales aún no se alcanzan; o **cerrar o rediseñar
    el proyecto** si se cumple cualquier criterio de fracaso. Si se decide continuar pese a un
    criterio de fracaso cumplido, la excepción se documenta **con su justificación y su nueva fecha
    de revisión**, para que quede constancia de que se está eligiendo ignorar la evidencia.

---

## Criterios de aceptación

* Ambas células están operativas con números dedicados, y **ningún número principal de un negocio se
  ha usado en ningún momento**.
* **La titularidad del número y de la SIM de cada célula está a nombre del cliente.** Ningún número
  está a nombre de HexCell, ninguno es virtual o VoIP y ninguno procede de una SIM recién activada.
* Cada piloto ha aceptado explícitamente el documento de expectativas antes del alta —con el riesgo
  de baneo escrito con todas sus letras—, y el de piloto-02 incluye el importe y la fecha del primer
  cobro.
* **El contrato de canal propio existe y piloto-02 lo ha aceptado antes del primer cobro.** Declara
  el canal como propio y no oficial, el riesgo de baneo, la ausencia de garantía de disponibilidad y
  el modo degradado. Consta por escrito el resultado de su **revisión legal local**, o el motivo de
  no haberla realizado.
* **El runbook de baneo existe antes del primer incidente**, con sus cuatro ramas de clasificación,
  la prohibición de reconectar en bucle ante baneo temporal, el guion de apelación y la plantilla de
  comunicación al cliente.
* **El runbook contiene el procedimiento de sustitución de número**, y contiene las cinco partes:
  cuándo procede —baneo permanente o apelación fracasada— y cuándo **no** procede —baneo temporal,
  donde se espera en lugar de sustituir—; qué se conserva y qué se pierde; los pasos operativos
  apoyados en `cell rebind`; quién debe estar presente, incluido el dueño con su teléfono; y el
  **aviso a los contactos que tenían guardado el número viejo**, con la plantilla ya redactada y con
  la constancia explícita de que **lo emite el cliente y no el sistema**, porque desde la cuenta
  baneada no se puede enviar y desde la nueva sería una iniciación de conversación en masa.
* **El re-emparejamiento por `PairPhone()` se ha ensayado y cronometrado en el alta de cada célula**,
  y los tiempos medidos están registrados.
* **El simulacro completo de baneo se ha ejecutado antes del primer cobro a piloto-02**, terminando
  en un bot que reconecta y responde, con los tiempos registrados. Un simulacro que termina en "el
  archivo existe" no cuenta como ejecutado.
* El resultado del **experimento de Meta Verified** está registrado y calificado como evidencia
  anecdótica; ningún documento del plan lo cita como medida probada.
* **El documento de compuerta pre-registrada existe, está fechado y su fecha es anterior al alta de
  piloto-01.** Contiene umbrales numéricos concretos, criterios de fracaso y las dos compuertas de
  riesgo de cartera —techo duro y umbral de incidentes— con valores numéricos, no adjetivos.
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
* Los parámetros de GCRA, del presupuesto LLM y de los límites de memoria han sido revisados contra
  datos reales, y los ajustes están documentados.
* Existe un informe de compuerta con una decisión explícita —seguir adelante y abrir altas, seguir
  validando, o cerrar o rediseñar—, tomada **comparando los valores medidos contra los umbrales
  pre-registrados**, no reinterpretándolos. Si se continúa pese a un criterio de fracaso cumplido, la
  excepción está documentada con su justificación y su fecha de revisión.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| **Baneo del número de un piloto.** | Medio: se pierde continuidad y confianza, no el negocio del cliente. | Números dedicados y a nombre del cliente, política de solo responder de la etapa A-3, expectativa y contrato pactados por escrito de antemano, y **runbook de baneo** con la apelación desde el teléfono del titular. **Procedimiento de sustitución de número documentado en el runbook** (tarea 5), con su criterio de procedencia, su aviso a cargo del cliente y su ejecución por `cell rebind` (etapa A-6), y **SIM de reserva envejeciendo desde el alta** [precautorio] para que el reemplazo no entre más débil que lo reemplazado. |
| **Rotura del protocolo de WhatsApp** durante el piloto. | Alto para la percepción del piloto: el bot enmudece. | Expectativa pactada de posibles semanas de silencio; dependencia fijada y actualizable en un paso; comunicación proactiva al piloto en cuanto se detecta, sin esperar a que pregunte. |
| El piloto-02 se siente experimento y abandona. | Alto: se pierde la única validación externa. | Expectativas honestas desde el principio, comunicación proactiva ante incidentes, y una carga de conocimiento inicial suficientemente buena como para que el bot aporte valor desde el primer día. |
| Las métricas se definen después de empezar a operar. | Alto: se mide lo que se puede en lugar de lo que importa, y la compuerta se decide por impresión. | La compuerta se **pre-registra** en la tarea 1, con umbrales numéricos y criterios de fracaso fechados antes del primer alta. |
| **Sesgo de cortesía: el piloto-02 dice que le gusta y que pagaría, porque es un conocido.** | Muy alto: es el modo de fallo más probable de toda la Fase A, y produce una validación falsa que arrastra a abrir más altas y a un gasto real. | **Mitigado por construcción:** la métrica de pago no es una declaración sino **un cobro ejecutado** desde el segundo mes. Nadie paga por cortesía todos los meses. La negativa a pagar es un criterio de fracaso explícito, no una señal a interpretar. |
| **Juez y parte: el dueño evalúa su propio piloto.** | Muy alto: piloto-01 siempre "funciona bien" porque quien lo mide es quien lo construyó y no tiene alternativa a la que irse. | **Mitigado por construcción:** piloto-01 queda degradado a banco de pruebas técnico y **sus datos no cuentan para la validación de negocio**, con criterio de aceptación que lo verifica en el informe de compuerta. |
| Se llega al momento del cobro y da pereza pedir dinero a un conocido. | Muy alto: se pierde el único dato que de verdad valida, y se sustituye por la impresión que se quería evitar. | El cobro es la tarea 13, con fecha pactada por escrito desde antes del alta, y una compuerta evaluada sin intento de cobro se declara **no concluyente**. |
| Los umbrales se reinterpretan a la baja al ver los resultados. | Alto: la compuerta se convierte en una racionalización de lo que ya se quería hacer. | Pre-registro fechado y anterior al alta; cualquier excepción debe documentarse como tal, con justificación y fecha de revisión. |
| **Se comercializa sobre un canal que viola los ToS de Meta y que puede desaparecer sin aviso.** | Muy alto y **permanente**: es el riesgo estructural del producto, no un riesgo de transición que caduque. | **Riesgo asumido de forma explícita y permanente el 28 de julio de 2026** (`adr-0014`), no evitado: la compuerta del tercer cliente queda derogada. Se contiene, no se elimina, con las medidas de contención del daño: **techo duro de cartera** mientras el canal propio sea el único, **umbral de incidentes que congela las altas**, **titularidad del número siempre en el cliente**, **contrato** que declara el canal como no oficial sin garantía de disponibilidad y con modo degradado pactado, y **aislamiento estricto por célula**. |
| **Se confunde reducir la probabilidad de baneo con evitarlo.** | Muy alto: se invierte en higiene de comportamiento y se descuida la recuperación, que es donde está el valor. | El riesgo es en buena medida **estructural** —Meta detecta la biblioteca por su huella de protocolo— y el baneo se documenta como **evento esperado**. Por eso esta etapa exige runbook, simulacro cronometrado y ensayo de re-emparejamiento en cada alta, no solo buenas prácticas de envío. |
| **La cláusula de exoneración del contrato resulta inejecutable frente a una microempresa.** | Alto: es peor que no tener cláusula, porque genera falsa seguridad en ambas partes. | **Revisión legal local obligatoria** del contrato (tarea 3), con su resultado registrado por escrito antes del primer cobro. |
| El re-emparejamiento se necesita de urgencia y depende de la agenda del dueño del negocio. | Alto: el tiempo de recuperación deja de fijarlo el código. | **Ensayo cronometrado de `PairPhone()` en el alta de cada célula**, con el tiempo medido registrado, y expectativa pactada de que el dueño debe estar disponible con su teléfono. |
| **Modelo de monetización sin definir** (pendiente en STATUS.md). | Medio: el importe simbólico no equivale a un precio de mercado. | El cobro simbólico no pretende fijar el precio: valida que existe **disposición real a pagar algo**, que es el paso previo. El precio se calibra después, con el coste por conversación de esta misma etapa como suelo. |
| Toda la validación de negocio descansa sobre una única célula. | Alto: N=1 es poca base para una decisión de inversión. | Es una limitación asumida y declarada, no un descuido: es preferible una fuente honesta a dos de las cuales una es el propio dueño. Se compensa exigiendo señales **sostenidas en el tiempo** (4 semanas consecutivas) y un pago recurrente, no un instante favorable. |
| Los datos conversacionales de clientes finales de un negocio ajeno se pierden. | Muy alto: daño reputacional y de confianza irreparable. | Respaldo de las cuatro bases —diseñado en la etapa A-2 y completado en la A-3 con la copia del `sqlstore` y el ensayo extremo a extremo— operativo desde el primer día del piloto, con restauración verificada —hasta que el bot responde— durante el periodo, y re-emparejamiento por `PairPhone()` ya ensayado. |

---

## Dependencias

* **De otras etapas:** etapa A-6 completa. Sin imágenes, composición de célula y CLI de operación, el
  alta no debe intentarse. Las medidas técnicas que esta etapa presupone —instrumentación de las
  variantes de desconexión, aislamiento por célula, regla de restauración del `sqlstore`— vienen de
  las etapas A-2, A-3 y A-6 bajo `adr-0015`.
* **Externas:** dos números de WhatsApp dedicados, sobre SIM física y **a nombre del cliente**; un
  teléfono disponible para el emparejamiento y para las apelaciones; **una revisión legal local del
  contrato**, cuyo plazo no depende del proyecto; y un conocido dispuesto a servir de piloto-02
  **aceptando por escrito las expectativas, el contrato y el cobro simbólico desde el segundo mes**.
  Si no acepta el cobro antes de empezar, no es un piloto de validación: es un usuario gratuito, y no
  aporta la evidencia que esta etapa necesita.
* **Decisiones de producto pendientes:** la **lógica de negocio específica** y los **flujos de usuario
  finales** determinan qué responde el bot. Esta etapa los aborda de la única forma honesta que
  existe: descubriéndolos con negocios reales en lugar de suponerlos. El **modelo de monetización**
  recibe de aquí su primera entrada empírica, que no es una opinión sino un cobro ejecutado.
* **Del dueño del proyecto, antes de la tarea 1 y de forma bloqueante:** los valores concretos de los
  umbrales (N conversaciones semanales, porcentaje de resolución, retención, coste máximo por
  conversación, disponibilidad mínima), el importe simbólico del cobro, los techos de los criterios
  de fracaso y **los valores de las dos compuertas de riesgo de cartera**: el techo duro de células
  vivas (M) y el umbral de incidentes de baneo con su ventana temporal (I incidentes en V semanas).
  El plan fija la **estructura** de la compuerta; los números son decisión de negocio y se registran
  como pendientes en `docs/STATUS.md` hasta que se cierren.
