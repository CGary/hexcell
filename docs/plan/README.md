# Plan de Implementación por Etapas — ZeroClaw Orchestrator

> Documento índice. Última actualización: 2026-07-24.
> Fuentes normativas: [PRD.md](../PRD.md) (requisitos FR/NFR y criterios de QA), [README.md](../../README.md) (arquitectura y CLI), [STATUS.md](../STATUS.md) (avance).

---

## 1. Visión general

ZeroClaw es un orquestador multi-inquilino que despliega bots de WhatsApp para microempresas
sobre hardware local modesto. El plan que sigue traduce en una secuencia de ocho etapas ejecutables
lo que ya está documentado en el proyecto, sin añadir requisitos de cosecha propia. Se apoya en dos
fuentes con rango distinto:

* El **PRD es la fuente normativa**. Cada etapa declara explícitamente qué requisitos funcionales
  (FR) y no funcionales (NFR) cubre, y entre todas se cubren FR-01 a FR-11 y NFR-01 a NFR-05.
* El **README.md aporta detalle operativo** que el PRD no recoge, principalmente el flujo de
  onboarding con Meta Embedded Signup y `override_callback_uri` (etapa 7) y el comando de
  eliminación definitiva de un inquilino (etapa 6). Esos elementos van marcados con una nota de
  fuente en la etapa correspondiente. Ante cualquier contradicción entre ambos documentos, manda el
  PRD.

La idea que gobierna el orden es sencilla de enunciar y difícil de respetar bajo presión: **nada
se expone a la red pública de Meta hasta que el componente que recibe ese tráfico sabe protegerse
a sí mismo**. Meta reintenta agresivamente los webhooks que no reciben un `200 OK`, de modo que un
despliegue prematuro no produce un fallo silencioso sino una tormenta de reintentos que degrada el
servidor completo y, en el peor caso, provoca que Meta desuscriba la aplicación. Por eso el control
de admisión (GCRA) y la contabilidad financiera se construyen *antes* de que exista un onboarding
real, y el plano de control (Caddy y la CLI) se construye *antes* de dar de alta al primer cliente
de verdad.

La segunda idea rectora es que el conocimiento (RAG) es un subsistema con su propio ciclo de vida.
Necesita persistencia estable debajo, así que se aborda después del núcleo de datos, pero antes del
empaquetado en contenedor, porque el diseño del volumen de disco y de los puntos de montaje depende
de cómo se materialicen las épocas de conocimiento en el sistema de archivos.

### Nomenclatura mínima

Definiciones de los términos que se repiten a lo largo del plan, en su primera aparición:

* **Inquilino** (*tenant*): una microempresa cliente. Cada inquilino corresponde a un contenedor
  Docker, un subdominio, una cuenta de WhatsApp Business (WABA) y un par de bases de datos propias.
* **WABA**: *WhatsApp Business Account*, la cuenta de Meta a la que se asocian los números de
  teléfono y las suscripciones de webhook de un cliente.
* **Webhook**: petición HTTP que Meta envía a nuestro servidor cuando ocurre un evento (mensaje
  entrante, cambio de estado de entrega, etc.).
* **GCRA** (*Generic Cell Rate Algorithm*): algoritmo de control de tasa que decide, con una sola
  marca temporal por clave y sin cerrojos, si una petición se admite o se rechaza.
* **Shadow DB**: base de datos en sombra donde se compila el nuevo conocimiento sin tocar la que
  está sirviendo tráfico en producción.
* **Época**: versión inmutable y numerada de la base de conocimiento (`knowledge_epoch_N.db`).
* **Fast-Reject**: patrón por el cual respondemos `HTTP 200 OK` inmediato a una petición que no
  vamos a procesar, para que Meta la dé por entregada y no la reintente.
* **Blackholing**: sustitución temporal del proxy inverso por una respuesta estática, de modo que
  el tráfico se absorbe sin llegar a ningún backend.
* **Drenaje controlado** (*Graceful Drain*): cierre ordenado de un pool de conexiones antiguo,
  esperando a que terminen las operaciones en vuelo antes de liberar los descriptores de archivo.

---

## 2. Tabla de etapas

| Nº | Nombre | Objetivo (una línea) | FR / NFR cubiertos | Depende de |
| :-- | :--- | :--- | :--- | :--- |
| 1 | [Fundaciones del repositorio y contrato con Meta](etapa-1-fundaciones.md) | Dejar el repositorio, la licencia, el workspace Rust y la CI listos, y reconstruir formalmente FR-01. | FR-01 (especificación) | — |
| 2 | [Núcleo del inquilino: HTTP y persistencia dual](etapa-2-nucleo-http-persistencia.md) | Construir el binario del inquilino que recibe y verifica webhooks sobre dos bases SQLite independientes. | FR-01 (implementación), FR-05, NFR-01 (parcial) | 1 |
| 3 | [Defensa perimetral y control presupuestario](etapa-3-admision-y-presupuesto.md) | Impedir que ráfagas de tráfico o el coste del LLM desestabilicen el sistema. | FR-08, FR-09, FR-10 | 2 |
| 4 | [Motor de conocimiento: Shadow DB y épocas](etapa-4-conocimiento-shadow-db.md) | Actualizar el conocimiento del bot sin detener la producción ni corromper el WAL. | FR-06, FR-07, NFR-03 | 2 (y 3 para el coste de embeddings) |
| 5 | [Empaquetado y aislamiento por contenedor](etapa-5-empaquetado-aislamiento.md) | Convertir el binario en una imagen mínima con aislamiento de disco verificable. | FR-02, NFR-01, NFR-05 | 2, 3, 4 |
| 6 | [Plano de control: Caddy y CLI de administración](etapa-6-plano-de-control.md) | Gobernar rutas, certificados y ciclo de vida de contenedores sin exponer errores 502 a Meta. | FR-03, FR-11, NFR-02, NFR-04 | 5 |
| 7 | [Onboarding de inquilinos y handshake de red](etapa-7-onboarding.md) | Dar de alta una microempresa real de extremo a extremo, sorteando la ausencia de Hairpin NAT. | FR-04, y cierre operativo de FR-01, FR-03, NFR-04 | 6 |
| 8 | [Endurecimiento, QA y operación](etapa-8-endurecimiento-qa.md) | Demostrar con pruebas medibles que se cumplen los criterios de aceptación y los NFR. | NFR-01 a NFR-05, verificación cruzada de FR-02, FR-07, FR-08 | 7 |

---

## 3. Justificación de la secuencia

El orden no es arbitrario; cada salto responde a una dependencia técnica concreta.

**1 → 2.** No se puede escribir código de producción sin un workspace, una licencia y una CI que
impida que la primera semana de trabajo se convierta en deuda. Además, FR-01 llegó truncado en el
documento original: implementar el receptor de webhooks sin haber reconstruido y validado ese
requisito sería construir sobre una suposición.

**2 → 3.** El control de admisión GCRA y la contabilidad financiera operan *sobre* un servidor HTTP
y *sobre* un estado persistente. Necesitan que exista el pipeline de peticiones y las tablas de saldo
antes de poder interponerse en ellos.

**3 → 4.** El motor de conocimiento consume APIs externas de embeddings, que cuestan dinero. Tener
antes la contabilidad de dos fases permite que la ingesta por lotes se someta al mismo presupuesto
que el resto de llamadas externas, en lugar de convertirse en un agujero de gasto sin instrumentar.

**2, 3, 4 → 5.** La imagen de contenedor y su diseño de volúmenes solo pueden fijarse cuando se sabe
qué archivos existen en disco (dos bases activas, la de staging y las épocas históricas) y qué
recursos consume el proceso en reposo. Empaquetar antes obliga a rehacer el `Dockerfile` en cada
iteración.

**5 → 6.** La CLI de administración manipula contenedores y rutas de Caddy. Solo tiene sentido
cuando existe una imagen que arrancar y un endpoint `GET /health/ready` al que interrogar.

**6 → 7.** El onboarding real registra una URL en Meta. Antes de hacerlo debe existir la capacidad de
crear el subdominio en Caddy, obtener el certificado y suspender o eliminar al inquilino si algo sale
mal. Registrar en Meta sin poder revertir la operación es el peor orden posible.

**7 → 8.** Los criterios de QA del PRD (carga de red, resiliencia TLS, consistencia WAL) son pruebas
de sistema completo. Requieren un inquilino real desplegado para ser significativas.

---

## 4. Cómo leer el plan

Cada archivo de etapa sigue la misma estructura, pensada para que un desarrollador pueda tomarla y
empezar sin leer las demás:

* **Objetivo** — por qué existe la etapa, en prosa.
* **Alcance** — qué entra, qué queda explícitamente fuera y qué FR/NFR del PRD cubre.
* **Entregables** — artefactos concretos que quedan en el repositorio al terminar.
* **Tareas** — lista ordenada; cada tarea está dimensionada entre medio día y dos días de trabajo.
* **Criterios de aceptación** — comprobaciones verificables, ligadas a los criterios de QA del PRD
  cuando aplica.
* **Riesgos y mitigaciones**.
* **Dependencias** — de otras etapas y de decisiones externas.

### Sobre las decisiones de producto pendientes

STATUS.md registra varios asuntos sin resolver: el modelo de monetización, los flujos de usuario
finales, el manejo de excepciones comerciales y el proceso exacto de alta de una microempresa. Este
plan **no los resuelve**, porque no son decisiones de ingeniería. Aparecen en las etapas que los
necesitan bajo el epígrafe de dependencias externas o de riesgos, con una indicación clara de qué
parte del trabajo queda bloqueada mientras no exista una respuesta. La lista consolidada de esos
bloqueos está en la etapa correspondiente y se resume aquí:

* **Modelo de monetización** — bloquea la calibración de los saldos y de la política de degradación
  en la etapa 3, y el criterio de suspensión por falta de pago en la etapa 6.
* **Proceso exacto de onboarding y flujos de usuario** — bloquea la etapa 7.
* **Manejo de excepciones comerciales** — condiciona el comportamiento del modo degradado en la
  etapa 3 y el alcance de la lógica de negocio del bot, que este plan trata como fuera de alcance
  hasta que exista definición.

### Estimación de duración

Las etapas no llevan fechas absolutas, porque el equipo aún no está dimensionado. Cada una declara
una **duración relativa** en una escala de tres niveles (Corta, Media, Larga) derivada de la suma de
sus tareas. La escala sirve para planificar capacidad, no para comprometer entregas.
