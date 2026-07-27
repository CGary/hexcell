# Fase A · Etapa 4 — Control de admisión y presupuesto

**Duración relativa:** Media.

---

## Objetivo

El núcleo de la etapa A-2, ya conectado al canal real por la etapa A-3, es ingenuo: procesa todo lo
que llega y gasta sin mirar el saldo. Esta etapa lo convierte en un componente capaz de sobrevivir a
dos amenazas que tienen la misma forma aunque parezcan distintas, porque ambas son un consumo sin
techo.

La primera amenaza es el tráfico. Un pico de mensajes o una campaña de spam contra el número de una
célula puede saturar un servidor doméstico. FR-08 obliga a un control de admisión GCRA que decida
admitir o descartar **antes de reservar memoria en el heap**. La diferencia con el plan original está
en el punto de aplicación: el GCRA **opera sobre el flujo normalizado del puerto de canal**, no sobre
un middleware HTTP. En la Fase A no hay petición entrante que contestar —los mensajes llegan por un
websocket saliente—, de modo que el exceso simplemente no se procesa y el descarte queda registrado.
El patrón *Fast-Reject* con `HTTP 200 OK` hacia Meta no desaparece del diseño: se pospone a la etapa
B-1, donde vuelve a tener sentido porque vuelve a haber alguien esperando una respuesta.

Situar el GCRA en el puerto y no en el transporte tiene una ventaja que compensa con creces el
esfuerzo: el mecanismo de admisión se escribe **una sola vez** y sobrevive intacto al cambio de fase.

La segunda amenaza es el dinero. La inferencia se delega a APIs externas de pago y el coste real de
una llamada solo se conoce cuando la respuesta llega con sus metadatos de tokens. FR-10 exige por
ello una contabilidad en dos fases: una **reserva previa** basada en la longitud estimada del prompt,
que se descuenta antes de invocar al modelo, y una **conciliación posterior** que ajusta la reserva
al consumo real. Cuando el saldo se agota, el bot no se cae: conmuta a un modo degradado de reglas
fijas locales. Esta parte no cambia respecto del diseño original, porque nunca dependió del
transporte.

Se añade aquí también FR-09, el semáforo de concurrencia de CPU, porque pertenece a la misma
familia de decisiones: poner un techo explícito a lo que el proceso se permite hacer a la vez.

---

## Alcance

### Qué entra

* Control de admisión GCRA sin cerrojos, interpuesto **en el flujo de eventos canónicos del puerto de
  canal**, lo más cerca posible de su origen, de modo que el descarte ocurra antes de asignar memoria
  de procesamiento.
* Registro explícito de cada descarte con su clave, porque en la Fase A un evento descartado es un
  mensaje de un cliente final que nunca recibe respuesta y no hay ningún código HTTP que lo delate.
* Parametrización del GCRA: tasa sostenida, ráfaga tolerada y granularidad de la clave de
  limitación, con los valores documentados y configurables.
* Semáforo de concurrencia sobre las tareas Tokio en vuelo, con límite estricto por contenedor y
  comportamiento definido cuando se alcanza.
* Contabilidad financiera de dos fases: reserva previa atómica, invocación del proveedor,
  conciliación con los tokens reales devueltos, y liberación de la reserva si la llamada falla.
* Persistencia del saldo y del libro de movimientos en `sessions.db`, con las operaciones de reserva
  y conciliación protegidas contra condiciones de carrera.
* Modo degradado: cuando el saldo se agota, las respuestas se generan con reglas fijas locales sin
  invocar al LLM, y el hecho queda registrado.
* Cliente real de al menos un proveedor de inferencia externo, integrado detrás de la interfaz que
  la etapa A-2 definió, con tiempos de espera y política de reintentos acotada.
* Métricas internas expuestas: eventos admitidos y descartados por GCRA, tareas en vuelo, saldo
  disponible y desviación entre lo reservado y lo conciliado.

### Qué NO entra

* El patrón *Fast-Reject* con `HTTP 200 OK` hacia Meta. No hay petición entrante en la Fase A; se
  añade en la etapa B-1 reutilizando este mismo módulo de admisión.
* Precios, planes y recargas de saldo. Son decisiones de monetización pendientes; aquí se construye
  el mecanismo, no la política comercial.
* La conmutación de conocimiento y los embeddings: etapa A-5. Esta etapa deja preparada la interfaz de
  contabilidad para que la ingesta por lotes la consuma.
* Las respuestas concretas del modo degradado como producto: se implementa el mecanismo con un
  conjunto mínimo de reglas, no un catálogo de mensajes comerciales.

### Requisitos del PRD cubiertos

* **FR-08** — control de admisión anti-spam mediante GCRA sobre el flujo normalizado del puerto.
* **FR-09** — semáforo de concurrencia de CPU.
* **FR-10** — contabilidad financiera de dos fases con modo degradado.

---

## Entregables

* Módulo de admisión GCRA en `hexcell-core`, reutilizable, independiente del transporte y con
  pruebas propias.
* Integración del módulo en el consumo del puerto de canal dentro de `hexcell`.
* Módulo de contabilidad con la máquina de estados de reserva y conciliación.
* Tablas de saldo y de movimientos en las migraciones de `sessions.db`.
* Cliente de inferencia real en un crate o módulo propio, detrás de la interfaz existente.
* `docs/adr/adr-0004-gcra-y-parametros.md` y
  `docs/adr/adr-0005-contabilidad-dos-fases.md`.
* Prueba de carga reproducible que inyecta 100 eventos concurrentes por el puerto de canal.

---

## Tareas

1. **Implementar el algoritmo GCRA** (1,5 días). Estructura sin cerrojos basada en operaciones
   atómicas, con una sola marca temporal por clave, y pruebas unitarias que verifiquen la tasa
   sostenida y la ráfaga tolerada. Sin ninguna dependencia de HTTP.
2. **Integrarlo en el consumo del puerto de canal** (1 día). Colocarlo antes de cualquier
   deserialización pesada o carga de contexto conversacional, de modo que el descarte no asigne
   memoria de procesamiento.
3. **Parametrizar y documentar los límites** (0,5 días). Elegir tasa, ráfaga y clave de limitación;
   dejarlos configurables por variable de entorno y justificarlos en el ADR.
4. **Instrumentar el registro de descartes** (0,5 días). Cada evento descartado deja constancia con su
   clave y su motivo, con visibilidad suficiente para detectar que se está perdiendo tráfico legítimo.
5. **Implementar el semáforo de concurrencia** (1 día). Límite de tareas en vuelo, adquisición sin
   bloqueo indefinido y comportamiento explícito ante saturación, coherente con la política de
   descarte.
6. **Diseñar el esquema de saldo y movimientos** (0,5 días). Migración con las tablas y sus
   restricciones de integridad.
7. **Implementar la reserva previa** (1 día). Estimación de coste a partir de la longitud del
   prompt, descuento atómico y rechazo limpio si no hay saldo suficiente.
8. **Implementar la conciliación posterior** (1 día). Ajuste con los tokens reales, devolución del
   sobrante, cargo del defecto y liberación de la reserva ante fallo o tiempo de espera agotado.
9. **Integrar el proveedor de inferencia real** (1,5 días). Cliente HTTPS saliente con tiempos de
   espera, reintentos acotados y extracción de los metadatos de tokens de la respuesta.
10. **Implementar el modo degradado** (1 día). Detección de saldo agotado, conmutación a reglas fijas
    locales, registro del evento y retorno automático al modo normal cuando hay saldo.
11. **Exponer métricas internas** (0,5 días). Contadores de admisión, descarte, tareas en vuelo,
    saldo y desviación de conciliación, accesibles para la operación.
12. **Construir la prueba de carga** (1 día). Script reproducible que inyecta 100 eventos concurrentes
    por el puerto y mide latencia, tasa de descarte y crecimiento de memoria residente.

---

## Criterios de aceptación

* **Ligado al criterio de QA "Prueba de Carga del Canal" del PRD:** con 100 eventos concurrentes
  inyectados por el puerto, el control de admisión GCRA se activa, el exceso se descarta sin
  procesarse y el consumo de memoria residente no crece más de un 15 % respecto de la línea base
  medida en la etapa A-2.
* Todo descarte GCRA queda registrado desde el primer día con su clave, marca temporal y motivo; el
  descarte silencioso está prohibido, de modo que la pérdida de tráfico legítimo sea detectable sin
  depender de un código de respuesta.
* **Criterio de no-falso-positivo:** bajo una simulación de tráfico legítimo a la tasa normal de una
  conversación —patrones realistas de mensajería, no ráfagas—, el número de descartes GCRA es cero;
  los umbrales de tasa y ráfaga se calibran contra este perfil antes de exponer el mecanismo a
  tráfico real.
* Existe un umbral de descartes anómalos que alimenta las alertas de la etapa A-6: un cliente
  legítimo siendo descartado debe disparar una alerta activa, no descubrirse semanas después al
  revisar los registros en la etapa A-7.
* El módulo de admisión no tiene ninguna dependencia de HTTP ni del transporte, verificable porque sus
  pruebas unitarias se ejecutan sin levantar ningún servidor.
* El número de tareas Tokio en vuelo nunca supera el límite configurado, verificado por métrica
  durante la prueba de carga.
* Una llamada al LLM que falla o agota su tiempo de espera libera íntegramente la reserva: el saldo
  final es idéntico al inicial.
* Tras una llamada exitosa, el saldo refleja el coste real de los tokens devueltos, no la estimación.
* Con saldo agotado, el bot sigue respondiendo mediante reglas fijas locales, no invoca al proveedor
  externo y registra la conmutación.
* Ejecuciones concurrentes de reserva sobre el mismo saldo no producen sobregiro.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Parámetros de GCRA mal calibrados que descartan tráfico legítimo. | **Muy alto en la Fase A:** un mensaje descartado es un cliente final de un piloto que nunca recibe respuesta, y no hay ningún código de error que lo delate. | Registrar cada descarte con su clave, empezar con límites holgados y revisar los registros con los datos reales de los pilotos en la etapa A-7. |
| Mensajes reales de clientes son descartados por GCRA sin que ningún check lo detecte, quedando oculto hasta la revisión manual de registros en la etapa A-7. | Muy alto en la Fase A: se quema la confianza del único piloto sin ninguna señal temprana que lo advierta. | Criterio de aceptación de no-falso-positivo contra tráfico legítimo simulado, registro no silencioso desde el primer día con clave, marca temporal y motivo, y umbral de descartes anómalos conectado a las alertas activas de la etapa A-6. |
| Aplicar el GCRA después de cargar el contexto conversacional. | Medio: se pierde el beneficio de no asignar heap y la prueba de carga falla por consumo de memoria. | Fijar la posición del control por diseño y verificarlo con la métrica de memoria. |
| Acoplar el módulo de admisión a un detalle del transporte. | Alto: habría que reescribirlo en la Fase B en lugar de reutilizarlo. | Vive en `hexcell-core`, sin dependencias de infraestructura, y sus pruebas corren sin servidor. |
| Estimación de prompt sistemáticamente inferior al coste real. | Medio: se permite gastar por encima del presupuesto. | Métrica de desviación entre reserva y conciliación, y factor de seguridad configurable en la estimación. |
| **Modelo de monetización sin definir** (pendiente en STATUS.md). | Medio: no se sabe cómo se recarga el saldo ni qué umbral dispara la degradación. | Se construye el mecanismo con valores configurables. La política comercial se inyecta como configuración cuando exista la decisión, sin tocar código. Los pilotos de la etapa A-7 aportarán el dato de consumo real. |
| El modo degradado se percibe como avería por el usuario final. | Medio. | El manejo de excepciones comerciales está pendiente de definición de producto; se deja el punto de extensión y se documenta el bloqueo. |

---

## Dependencias

* **De otras etapas:** etapa A-2 completa (la contabilidad necesita `sessions.db` y sus migraciones;
  el control de admisión necesita el flujo del puerto) y etapa A-3 para poder medir con tráfico real
  en lugar de solo simulado.
* **Externas:** credenciales de al menos un proveedor de inferencia (Gemini, Groq u OpenRouter) y una
  cuenta con saldo para las pruebas de integración.
* **Decisiones de producto pendientes:** el **modelo de monetización** condiciona la calibración de
  saldos, umbrales y política de degradación. No bloquea la construcción del mecanismo, pero sí su
  puesta en producción con valores definitivos.
