# Fase A · Etapa 2 — Núcleo de la célula: motor de mensajería y persistencia dual

**Duración relativa:** Larga.

---

## Objetivo

Esta etapa construye el corazón del producto: el binario que se ejecuta dentro del contenedor de
cada célula, consume eventos del puerto de canal y persiste el estado conversacional. Todo lo que
viene después (adaptador whatsmeow, control de admisión, conocimiento, empaquetado, plano de control)
se apoya sobre este componente, y todo lo que se haga mal aquí se paga multiplicado por el número de
células.

El cambio de rumbo respecto de la versión anterior del plan es importante y conviene enunciarlo sin
rodeos: **el núcleo ya no es un servidor HTTP para Meta**. Es un motor de mensajería que consume el
flujo normalizado del `ChannelAdapter` declarado en la etapa A-1. En la Fase A ese flujo llega por
IPC desde el sidecar whatsmeow; en la Fase B llegará de un receptor de webhooks. El núcleo no
distingue entre ambos, y esa indiferencia es exactamente el activo que se está construyendo.

Lo que sí se conserva del diseño original es el **endpoint interno `GET /health/ready`**, junto con
`GET /health/live`. No están ahí para recibir tráfico de mensajería, sino porque la CLI de
administración los sondea durante las reactivaciones. Es un servidor HTTP mínimo, interno y sin
exposición pública.

La persistencia sigue siendo la del PRD: FR-05 exige **dos bases SQLite físicamente separadas**
porque mezclar escrituras conversacionales continuas con lecturas intensivas de RAG en un único
archivo produce contención de escritura y errores `SQLITE_BUSY` en cuanto se introduce latencia de
red.

Y hay una incorporación deliberada: **el respaldo y la restauración por célula se adelantan a esta
etapa**. En el plan anterior vivían al final, en el endurecimiento. Con pilotos reales operando desde
la Fase A, sobre datos conversacionales de clientes finales de un negocio ajeno, los respaldos no
pueden esperar al final. Un disco que falla en la semana tres del piloto-02 no es un incidente
técnico: es la pérdida de la confianza que la validación del negocio necesita.

---

## Alcance

### Qué entra

* Motor de mensajería en `zeroclaw-cell`: bucle asíncrono sobre Tokio que consume eventos canónicos
  del `ChannelAdapter`, los procesa y emite respuestas mediante `send(conversation_id, contenido)`.
* Servidor HTTP interno mínimo con `GET /health/live` y `GET /health/ready`. No expone rutas de
  mensajería y no se publica fuera de la red local de la célula.
* Idempotencia de entrega: detección y descarte de eventos duplicados por el identificador de
  deduplicación que provee el puerto de canal, con independencia del transporte que lo originó.
* Mapeo de identidad de conversación: traducción del identificador de transporte a identificador
  interno y su persistencia. **`sessions.db` no almacena identificadores de transporte crudos.**
* Capa `zeroclaw-storage` con dos pools independientes: `sessions.db` en lectura/escritura y
  `knowledge_live.db` en lectura. Configuración de SQLite en modo WAL, con los ajustes de
  `busy_timeout`, `synchronous` y tamaño de pool decididos y documentados.
* Migraciones versionadas y reproducibles para `sessions.db`, y esquema inicial de solo lectura para
  `knowledge_live.db`.
* Modelo de estado conversacional: historial por contacto, con una política de retención definida.
* **Respaldo y restauración por célula:** copia consistente en caliente mediante `VACUUM INTO`,
  traslado de la copia fuera del disco del servidor y un procedimiento de restauración **probado**,
  no solo documentado.
* Apagado ordenado ante `SIGTERM`: dejar de consumir del puerto, drenar las tareas en vuelo,
  ejecutar el checkpoint de SQLite y salir con código 0, dentro de la ventana de 30 segundos que
  fija el PRD.
* Configuración por variables de entorno y observabilidad básica mediante logs estructurados.
* Un adaptador de canal simulado, en memoria, que inyecta eventos canónicos y captura envíos, para
  poder desarrollar y probar el núcleo completo antes de que exista el sidecar.

### Qué NO entra

* El adaptador whatsmeow y el protocolo IPC con el sidecar: etapa A-3. Aquí solo existe el simulado.
* El adaptador de Cloud API y la verificación de webhooks: etapa B-1.
* Control de admisión GCRA, semáforo de concurrencia y contabilidad financiera: etapa A-4.
* Construcción o promoción de conocimiento y embeddings: etapa A-5. Aquí `knowledge_live.db` solo se
  abre y se lee.
* Llamadas reales al LLM. Se define la interfaz del proveedor de inferencia y se implementa una
  versión simulada; el proveedor real llega con la contabilidad de la etapa A-4.
* Lógica de negocio del bot (atención al cliente, catálogo, agendamiento). Depende de decisiones de
  producto pendientes y queda fuera del alcance del plan hasta que existan.

### Requisitos del PRD cubiertos

* **FR-01** — implementación del consumo de mensajes entrantes por el puerto de canal, en su parte
  independiente del transporte.
* **FR-05** — arquitectura de persistencia dual.
* **FR-12** — implementación del lado consumidor del puerto: el núcleo opera contra el trait, no
  contra un canal concreto.
* **NFR-01** — cubierto parcialmente: se establece la línea base de consumo de memoria del proceso
  del núcleo. La verificación formal contra el presupuesto de fase se hace en las etapas A-6 y B-3.

---

## Entregables

* `zeroclaw-cell` como binario ejecutable que arranca, consume del puerto, sirve las dos rutas de
  salud y se apaga limpio.
* `zeroclaw-storage` con el gestor de pools duales y los ajustes de SQLite.
* Adaptador de canal simulado en memoria, reutilizable por todas las pruebas del plan.
* Directorio de migraciones para `sessions.db`.
* `docs/adr/adr-0003-persistencia-dual.md` documentando los parámetros de SQLite elegidos y
  el porqué de cada uno, con la numeración que fija el [índice de ADR](../adr/README.md).
* `docs/runbook-respaldo.md`: procedimiento de respaldo y de restauración por célula, con el
  resultado de la restauración real ejecutada como prueba.
* Script de respaldo ejecutable de forma programada, y script de restauración.
* Pruebas de integración que arrancan el núcleo sobre bases temporales y un adaptador simulado.

---

## Tareas

1. **Definir la configuración del proceso** (0,5 días). Variables de entorno, rutas de datos,
   secretos, validación al arranque con fallo temprano y mensaje claro si falta algo.
2. **Levantar el servidor HTTP interno y las rutas de salud** (0,5 días). `GET /health/live` responde
   en cuanto el proceso vive; `GET /health/ready` queda inicialmente en un esqueleto que la tarea 5
   completa. Vinculado exclusivamente a la interfaz interna de la célula.
3. **Construir el motor de mensajería sobre el puerto de canal** (1,5 días). Bucle de consumo de
   eventos canónicos, despacho al procesador, emisión de respuestas por `send` y tratamiento de los
   acuses normalizados. El motor no conoce ningún transporte.
4. **Implementar el adaptador simulado en memoria** (0,5 días). Inyección de eventos y captura de
   envíos, con control determinista del orden y de los tiempos para las pruebas.
5. **Construir el gestor de pools duales** (1,5 días). Dos pools separados, modo WAL, parámetros de
   `busy_timeout` y `synchronous` justificados, y comprobación de vitalidad de cada pool que alimenta
   `GET /health/ready`.
6. **Definir el esquema y las migraciones de `sessions.db`** (1 día). Contactos, conversaciones,
   mensajes, marcas temporales e índices necesarios, con el identificador interno de conversación
   como clave y **sin ninguna columna que almacene identificadores de transporte crudos**.
7. **Implementar el mapeo de identidad de conversación** (1 día). Traducción estable y reversible
   entre el identificador que provee el adaptador y el identificador interno, de modo que un cambio
   de canal no invalide el historial.
8. **Implementar la idempotencia de entrega** (1 día). Registro de identificadores de deduplicación
   ya procesados con ventana de retención, de modo que un reenvío del canal no duplique el trabajo.
9. **Definir la interfaz del proveedor de inferencia y su implementación simulada** (1 día). Un
   contrato que la etapa A-4 pueda envolver con la contabilidad sin cambiar el consumidor.
10. **Implementar el apagado ordenado** (1 día). Captura de `SIGTERM`, cese del consumo del puerto,
    drenaje de tareas en vuelo con límite temporal, checkpoint de SQLite y salida con código 0.
11. **Implementar el respaldo por célula** (1 día). `VACUUM INTO` sobre ambas bases con el proceso en
    caliente, verificación de integridad de la copia, y traslado fuera del disco del servidor. El
    respaldo no debe bloquear la operación de la célula ni disparar `SQLITE_BUSY`.
12. **Implementar y probar la restauración** (1 día). Reconstrucción completa de una célula a partir
    de sus copias sobre un entorno limpio, y verificación de que el historial y el conocimiento
    quedan íntegros. **Un respaldo sin restauración probada no cuenta como respaldo.**
13. **Instrumentar logs estructurados** (0,5 días). Identificador de célula, identificador de evento y
    latencia en cada entrada, sin volcar contenido de mensajes de usuarios.
14. **Escribir las pruebas de integración** (1 día). Camino feliz, evento duplicado, apagado bajo
    carga y ciclo completo de respaldo y restauración.

---

## Criterios de aceptación

* Un evento canónico inyectado por el adaptador simulado se procesa y queda registrado en
  `sessions.db`, y la respuesta se emite por `send` con el identificador interno correcto.
* Inyectar el mismo evento dos veces (mismo identificador de deduplicación) produce un único registro
  conversacional.
* Una inspección del esquema y de los datos de `sessions.db` no encuentra **ningún** identificador de
  transporte crudo.
* `GET /health/ready` responde `200 OK` únicamente cuando ambos pools SQLite están operativos y el
  puerto de canal está enlazado, y responde con error si se retira cualquiera de los dos archivos de
  base de datos.
* Ante `SIGTERM`, el proceso termina con código 0 en menos de 30 segundos, sin dejar eventos a
  medias y habiendo ejecutado el checkpoint del WAL.
* Un respaldo ejecutado con la célula en operación produce copias íntegras de ambas bases sin generar
  errores `SQLITE_BUSY` ni interrumpir el procesamiento de mensajes.
* Una restauración sobre un entorno limpio devuelve la célula a un estado funcional verificado, con
  su historial conversacional y su conocimiento intactos.
* El consumo de memoria residente del proceso en reposo queda medido y registrado como línea base
  para NFR-01.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| El núcleo se acopla al transporte por comodidad, saltándose el puerto. | Muy alto: se pierde la frontera de migración y la Fase B se convierte en una reescritura. | El núcleo se desarrolla íntegramente contra el adaptador simulado; el sidecar no existe todavía, de modo que el acoplamiento es imposible por construcción. |
| Un identificador de transporte acaba persistido en `sessions.db`. | Alto: migrar de canal obligaría a migrar datos históricos de clientes reales. | Criterio de aceptación explícito con inspección del esquema y de los datos. |
| Ajustes de SQLite copiados sin entenderlos. | Medio: aparecen `SQLITE_BUSY` bajo carga real, ya con pilotos vivos. | Documentar cada parámetro en `adr-0003` y validarlos con la prueba de consistencia WAL de la etapa A-5. |
| El respaldo se implementa pero nunca se prueba la restauración. | Muy alto: se descubre que no funciona el día que hace falta, con datos de un cliente real perdidos. | La restauración probada es criterio de aceptación bloqueante, no un entregable documental. |
| Las copias de respaldo se quedan en el mismo disco que los datos. | Muy alto: un fallo de disco se lleva original y copia. | El traslado fuera del disco forma parte del procedimiento y se verifica en la prueba. |
| La ausencia de lógica de negocio definida tienta a improvisarla. | Medio: se construye producto sobre supuestos no aprobados. | El alcance la excluye explícitamente; el procesador de mensajes queda como punto de extensión con una implementación mínima de eco. |

---

## Dependencias

* **De otras etapas:** etapa A-1 completa. En particular, el trait `ChannelAdapter` con sus tipos
  canónicos y el workspace con sus cinco crates.
* **Externas:** un destino de almacenamiento fuera del disco del servidor para las copias de
  respaldo. Es bloqueante para las tareas 11 y 12.
* **Decisiones de producto pendientes que afectan al alcance:** la lógica de negocio específica y los
  flujos de usuario finales de STATUS.md determinan qué hace el bot con un mensaje. Mientras no
  existan, esta etapa entrega la infraestructura y un procesador mínimo, no el comportamiento
  comercial.
