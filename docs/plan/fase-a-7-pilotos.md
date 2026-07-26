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
3. ¿El piloto **pagaría** por esto? Es la pregunta incómoda, y es la única que importa.

Las dos células son:

* **piloto-01** — negocio de prueba del propio dueño del proyecto. Va primero porque los fallos se
  pagan en casa.
* **piloto-02** — negocio de un conocido. Va después, y solo cuando piloto-01 lleve tiempo estable.

Ambas usan un **número de WhatsApp nuevo y dedicado**. Esta condición no es negociable y merece
enunciarse con claridad: el canal es no oficial y WhatsApp puede desactivar el número sin aviso ni
apelación. Si ese número fuera el número principal del negocio, un ban destruiría la línea comercial
del cliente. Con un número dedicado, un ban cuesta un número desechable y una conversación
incómoda, no el negocio de nadie.

Al final de esta etapa está la **compuerta**: la decisión de si el negocio está validado y si, por
tanto, el tercer cliente dispara la Fase B.

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
* Definición y recogida de las **métricas de validación del negocio**, tanto cuantitativas como
  cualitativas.
* Operación diaria durante el periodo del piloto: vigilancia de la sesión, respuesta a desconexiones,
  actualizaciones de conocimiento, revisión de los descartes del GCRA y de la desviación de la
  contabilidad LLM.
* Retroalimentación al plan: los registros de descartes, el consumo real de tokens y el consumo real
  de memoria alimentan la calibración de los parámetros de las etapas A-4 y A-6.
* **Evaluación de la compuerta de salida** y su decisión documentada.

### Qué NO entra

* Cualquier tercer cliente. El tercero es, por definición, el que dispara la Fase B; no se da de alta
  sobre canal no oficial.
* Comercialización, contratos de servicio y facturación. Esta etapa no vende.
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
* `docs/pilotos/metricas-validacion.md`: definición de las métricas, su método de recogida y el
  registro periódico de sus valores.
* Informe de la compuerta: estado de cada métrica, decisión sobre la validación del negocio y, si
  procede, disparo de la Fase B.
* Lista de ajustes de calibración derivados de la operación real, con destino a las etapas
  correspondientes.

---

## Tareas

1. **Definir las métricas de validación** (1 día). Antes de dar de alta a nadie, fijar qué se va a
   medir y cómo. Propuesta de partida, a cerrar con el responsable de producto:
   * *Calidad de respuesta:* proporción de conversaciones resueltas sin intervención humana;
     proporción de respuestas que el dueño del negocio marca como inadecuadas.
   * *Uso real:* número de conversaciones iniciadas por clientes finales por semana; proporción de
     clientes que vuelven.
   * *Disposición a pagar:* respuesta explícita del piloto a la pregunta de si pagaría, y cuánto,
     recogida al menos dos veces separadas en el tiempo.
   * *Coste por célula:* gasto real en LLM y embeddings por conversación atendida.
   * *Estabilidad:* tiempo con la sesión de canal activa sobre el tiempo total; número de
     re-emparejamientos necesarios.
2. **Redactar y pactar las expectativas con los pilotos** (0,5 días). Documento breve, en lenguaje
   llano, con los tres riesgos asumidos. **No se da de alta a ningún piloto sin esta aceptación
   explícita**, y muy especialmente a piloto-02, que no es el dueño del proyecto.
3. **Adquirir y preparar los números dedicados** (0,5 días). Un número nuevo por célula, verificado y
   sin historial. Ninguno puede ser el número principal de un negocio.
4. **Dar de alta piloto-01** (1 día). Aprovisionamiento de la célula, emparejamiento, carga de
   conocimiento inicial y prueba de extremo a extremo con mensajes reales.
5. **Operar piloto-01 en solitario** (varias semanas, con vigilancia diaria ligera). Detectar y
   corregir lo que solo aparece en producción antes de exponer a un tercero.
6. **Dar de alta piloto-02** (1 día). Misma secuencia, con el runbook ya endurecido por la
   experiencia de piloto-01.
7. **Operar ambas células y recoger métricas** (periodo de validación, con revisión semanal).
   Registro sistemático de las métricas definidas en la tarea 1.
8. **Revisar la calibración con datos reales** (1 día). Contrastar los descartes del GCRA, la
   desviación entre reserva y conciliación, y el consumo de memoria de la célula, frente a los
   valores supuestos en las etapas A-4 y A-6. Ajustar y documentar.
9. **Evaluar la compuerta y decidir** (1 día). Con las métricas sobre la mesa, responder a las tres
   preguntas de validación y documentar la decisión: continuar validando, abandonar, o declarar el
   negocio validado y disparar la Fase B con el tercer cliente.

---

## Criterios de aceptación

* Ambas células están operativas con números nuevos y dedicados, y **ningún número principal de un
  negocio se ha usado en ningún momento**.
* Cada piloto ha aceptado explícitamente el documento de expectativas antes del alta.
* El runbook de alta permite dar de alta una célula sin consultar a quien escribió el código.
* Las métricas de validación están definidas, se recogen de forma periódica y tienen valores
  registrados para todo el periodo de operación.
* La restauración de un respaldo real de una célula piloto sobre un entorno limpio se ha ejecutado al
  menos una vez, con éxito, durante el periodo de validación. No se espera a que haga falta.
* Los parámetros de GCRA, del presupuesto LLM y de los límites de memoria han sido revisados contra
  datos reales, y los ajustes están documentados.
* Existe un informe de compuerta con una decisión explícita, tomada sobre datos y no sobre impresión.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| **Ban del número de un piloto.** | Medio: se pierde continuidad y confianza, no el negocio del cliente. | Números nuevos y dedicados, calentamiento y política de solo responder de la etapa A-3, y expectativa pactada por escrito de antemano. Procedimiento de sustitución de número documentado en el runbook. |
| **Rotura del protocolo de WhatsApp** durante el piloto. | Alto para la percepción del piloto: el bot enmudece. | Expectativa pactada de posibles semanas de silencio; dependencia fijada y actualizable en un paso; comunicación proactiva al piloto en cuanto se detecta, sin esperar a que pregunte. |
| El piloto-02 se siente experimento y abandona. | Alto: se pierde la única validación externa. | Expectativas honestas desde el principio, comunicación proactiva ante incidentes, y una carga de conocimiento inicial suficientemente buena como para que el bot aporte valor desde el primer día. |
| Las métricas se definen después de empezar a operar. | Alto: se mide lo que se puede en lugar de lo que importa, y la compuerta se decide por impresión. | La definición de métricas es la tarea 1, anterior a cualquier alta. |
| Se cede a la tentación de dar de alta un tercer cliente sobre el canal no oficial. | Muy alto: se comercializa sobre un canal que viola los ToS y que puede desaparecer. | La compuerta es explícita en el PRD y en este plan: **el tercer cliente dispara la Fase B**, no se suma a la Fase A. |
| **Modelo de monetización sin definir** (pendiente en STATUS.md). | Medio: la pregunta "¿pagaría?" se hace sin un precio concreto que contrastar. | Se recoge la disposición a pagar en términos abiertos y se usa como entrada para fijar el modelo, no al revés. |
| Los datos conversacionales de clientes finales de un negocio ajeno se pierden. | Muy alto: daño reputacional y de confianza irreparable. | Respaldo de la etapa A-2 operativo desde el primer día del piloto, con restauración verificada durante el periodo. |

---

## Dependencias

* **De otras etapas:** etapa A-6 completa. Sin imágenes, composición de célula y CLI de operación, el
  alta no debe intentarse.
* **Externas:** dos números de WhatsApp nuevos y dedicados; un teléfono disponible para el
  emparejamiento; y un conocido dispuesto a servir de piloto-02 con las expectativas aceptadas.
* **Decisiones de producto pendientes:** la **lógica de negocio específica** y los **flujos de usuario
  finales** determinan qué responde el bot. Esta etapa los aborda de la única forma honesta que
  existe: descubriéndolos con dos negocios reales en lugar de suponerlos. El **modelo de
  monetización** recibe de aquí su primera entrada empírica.
