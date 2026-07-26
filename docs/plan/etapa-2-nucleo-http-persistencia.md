# Etapa 2 — Núcleo del inquilino: HTTP y persistencia dual

**Duración relativa:** Larga.

---

## Objetivo

Esta etapa construye el corazón del producto: el binario que se ejecuta dentro del contenedor de
cada inquilino, recibe los webhooks de Meta y persiste el estado conversacional. Todo lo que viene
después (control de admisión, conocimiento, empaquetado, plano de control) se apoya sobre este
componente, y todo lo que se haga mal aquí se paga multiplicado por el número de inquilinos.

Hay dos razones para tratar el servidor HTTP y la persistencia como una sola etapa en lugar de dos.
La primera es que el diseño de persistencia del PRD no es un detalle de implementación, sino una
decisión arquitectónica que condiciona el manejo de peticiones: FR-05 exige **dos bases SQLite
físicamente separadas** porque mezclar escrituras conversacionales continuas con lecturas intensivas
de RAG en un único archivo produce contención de escritura y errores `SQLITE_BUSY` en cuanto se
introduce latencia de red. La segunda es que el criterio de vida del proceso (`GET /health/ready`,
que la etapa 6 necesita) se define exactamente como "los pools de ambas bases responden", de modo
que HTTP y almacenamiento se validan mutuamente.

Aquí se implementa además el FR-01 que la etapa 1 dejó especificado. El receptor debe cumplir una
regla incómoda pero innegociable: responder a Meta rápido y con `200 OK` **antes** de hacer el
trabajo pesado. Cualquier otra cosa activa la máquina de reintentos de la API Graph.

---

## Alcance

### Qué entra

* Servidor HTTP asíncrono sobre Tokio dentro de `zeroclaw-tenant`, con las rutas del inquilino:
  `GET /webhook` (verificación), `POST /webhook` (recepción de eventos), `GET /health/live` y
  `GET /health/ready`.
* Implementación completa de FR-01: desafío de verificación, validación de firma HMAC-SHA256 sobre
  el cuerpo exacto recibido, respuesta inmediata `200 OK` y desacople del procesamiento real hacia
  una tarea en segundo plano.
* Idempotencia de entrega: detección y descarte de webhooks duplicados por identificador de mensaje.
* Capa `zeroclaw-storage` con dos pools independientes: `sessions.db` en lectura/escritura y
  `knowledge_live.db` en lectura. Configuración de SQLite en modo WAL, con los ajustes de
  `busy_timeout`, `synchronous` y tamaño de pool decididos y documentados.
* Migraciones versionadas y reproducibles para `sessions.db`, y esquema inicial de solo lectura para
  `knowledge_live.db`.
* Modelo de estado conversacional: historial por contacto, con una política de retención definida.
* Apagado ordenado ante `SIGTERM`: dejar de aceptar sockets, drenar las peticiones en vuelo,
  ejecutar el checkpoint de SQLite y salir con código 0, dentro de la ventana de 30 segundos que
  fija el PRD.
* Configuración por variables de entorno y observabilidad básica mediante logs estructurados.

### Qué NO entra

* Control de admisión GCRA, semáforo de concurrencia y contabilidad financiera: etapa 3.
* Construcción o promoción de conocimiento y embeddings: etapa 4. Aquí `knowledge_live.db` solo se
  abre y se lee.
* Llamadas reales al LLM. Se define la interfaz del proveedor de inferencia y se implementa una
  versión simulada; el proveedor real llega con la contabilidad de la etapa 3.
* Lógica de negocio del bot (atención al cliente, catálogo, agendamiento). Depende de decisiones de
  producto pendientes y queda fuera del alcance del plan hasta que existan.

### Requisitos del PRD cubiertos

* **FR-01** — implementación de la recepción y verificación de webhooks.
* **FR-05** — arquitectura de persistencia dual.
* **NFR-01** — cubierto parcialmente: se establece la línea base de consumo de memoria del proceso.
  La verificación formal contra el límite de 50 MB se hace en las etapas 5 y 8.

---

## Entregables

* `zeroclaw-tenant` como binario ejecutable que arranca, sirve las cuatro rutas y se apaga limpio.
* `zeroclaw-meta` con la verificación de firma y los tipos del payload de webhook.
* `zeroclaw-storage` con el gestor de pools duales y los ajustes de SQLite.
* Directorio de migraciones para `sessions.db`.
* `docs/adr/adr-0003-persistencia-dual.md` documentando los parámetros de SQLite elegidos y
  el porqué de cada uno.
* Pruebas de integración que arrancan el servidor sobre bases temporales.
* Un banco de pruebas local (`scripts/`) capaz de emitir webhooks firmados como lo haría Meta.

---

## Tareas

1. **Definir la configuración del proceso** (0,5 días). Variables de entorno, rutas de datos,
   secretos, validación al arranque con fallo temprano y mensaje claro si falta algo.
2. **Levantar el servidor HTTP y las rutas de salud** (1 día). `GET /health/live` responde en cuanto
   el proceso vive; `GET /health/ready` queda inicialmente en un esqueleto que la tarea 5 completa.
3. **Implementar la verificación de suscripción de webhook** (0,5 días). `GET /webhook` con
   comparación en tiempo constante del token de verificación y devolución del `hub.challenge`.
4. **Implementar la recepción firmada de eventos** (1,5 días). `POST /webhook` con validación
   HMAC-SHA256 sobre el cuerpo crudo, respuesta `200 OK` inmediata y encolado del procesamiento en
   una tarea en segundo plano. Incluye rechazo de peticiones sin firma o con firma inválida.
5. **Construir el gestor de pools duales** (1,5 días). Dos pools separados, modo WAL, parámetros de
   `busy_timeout` y `synchronous` justificados, y comprobación de vitalidad de cada pool que alimenta
   `GET /health/ready`.
6. **Definir el esquema y las migraciones de `sessions.db`** (1 día). Contactos, conversaciones,
   mensajes, marcas temporales e índices necesarios. Migraciones versionadas que se aplican al
   arrancar.
7. **Implementar la idempotencia de webhooks** (1 día). Registro de identificadores de mensaje ya
   procesados con ventana de retención, de modo que un reintento de Meta no duplique el trabajo.
8. **Definir la interfaz del proveedor de inferencia y su implementación simulada** (1 día). Un
   contrato que la etapa 3 pueda envolver con la contabilidad sin cambiar el consumidor.
9. **Implementar el apagado ordenado** (1 día). Captura de `SIGTERM`, cierre del listener, drenaje de
   tareas en vuelo con límite temporal, checkpoint de SQLite y salida con código 0.
10. **Instrumentar logs estructurados** (0,5 días). Identificador de inquilino, identificador de
    petición y latencia en cada entrada, sin volcar contenido de mensajes de usuarios.
11. **Escribir el banco de pruebas de webhooks y las pruebas de integración** (1 día). Un script que
    firme y envíe payloads realistas, y pruebas automatizadas que cubran el camino feliz, la firma
    inválida y el duplicado.

---

## Criterios de aceptación

* Un `GET /webhook` con el token correcto devuelve el `hub.challenge` tal cual; con token incorrecto
  devuelve un error y nunca el desafío.
* Un `POST /webhook` con firma válida responde `200 OK` y el evento queda registrado en
  `sessions.db`; con firma inválida se rechaza y no se escribe nada.
* Alterar un solo byte del cuerpo invalida la firma y la petición se rechaza.
* Reenviar el mismo webhook dos veces produce un único registro conversacional.
* `GET /health/ready` responde `200 OK` únicamente cuando ambos pools SQLite están operativos, y
  responde con error si se retira cualquiera de los dos archivos de base de datos.
* Ante `SIGTERM`, el proceso termina con código 0 en menos de 30 segundos, sin dejar peticiones a
  medias y habiendo ejecutado el checkpoint del WAL.
* El consumo de memoria residente del proceso en reposo queda medido y registrado como línea base
  para NFR-01.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Reserializar el cuerpo del webhook antes de validar la firma. | Alto: la firma falla de forma intermitente e inexplicable. | Validar siempre sobre los bytes crudos recibidos, antes de cualquier deserialización, y cubrirlo con una prueba explícita. |
| Procesar el mensaje antes de responder a Meta. | Alto: se superan los tiempos de espera y se dispara la tormenta de reintentos que FR-08 pretende evitar. | Responder `200 OK` y encolar; la prueba de integración mide el tiempo hasta la respuesta. |
| Ajustes de SQLite copiados sin entenderlos. | Medio: aparecen `SQLITE_BUSY` bajo carga real, ya en producción. | Documentar cada parámetro en el ADR y validarlos con la prueba de consistencia WAL de la etapa 8. |
| La ausencia de lógica de negocio definida tienta a improvisarla. | Medio: se construye producto sobre supuestos no aprobados. | El alcance la excluye explícitamente; el procesador de mensajes queda como punto de extensión con una implementación mínima de eco. |

---

## Dependencias

* **De otras etapas:** etapa 1 completa. En particular, el texto aprobado de FR-01 y el workspace
  con sus cinco crates.
* **Externas:** ninguna decisión de producto bloquea esta etapa.
* **Decisiones de producto pendientes que afectan al alcance:** la lógica de negocio específica y los
  flujos de usuario finales de STATUS.md determinan qué hace el bot con un mensaje. Mientras no
  existan, esta etapa entrega la infraestructura y un procesador mínimo, no el comportamiento
  comercial.
