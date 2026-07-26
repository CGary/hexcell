# Etapa 3 — Defensa perimetral y control presupuestario

**Duración relativa:** Media.

---

## Objetivo

El núcleo de la etapa 2 sabe recibir webhooks, pero es ingenuo: acepta todo lo que llega y gasta sin
mirar el saldo. Esta etapa lo convierte en un componente capaz de sobrevivir a dos amenazas que
tienen la misma forma aunque parezcan distintas, porque ambas son un consumo sin techo.

La primera amenaza es el tráfico. Meta reintenta con insistencia los webhooks que no recibe
confirmados, de modo que un pico de mensajes o una campaña de spam puede multiplicarse hasta
saturar un servidor doméstico. FR-08 obliga a un control de admisión GCRA que decida admitir o
rechazar **antes de reservar memoria en el heap**, y que responda `HTTP 200 OK` inmediato al exceso
de tasa. Es un patrón contraintuitivo: mentimos a Meta diciendo "recibido" para que no reintente. Es
exactamente lo correcto, porque un `429` o un `503` no reduce el tráfico, lo amplifica.

La segunda amenaza es el dinero. La inferencia se delega a APIs externas de pago y el coste real de
una llamada solo se conoce cuando la respuesta llega con sus metadatos de tokens. FR-10 exige por
ello una contabilidad en dos fases: una **reserva previa** basada en la longitud estimada del prompt,
que se descuenta antes de invocar al modelo, y una **conciliación posterior** que ajusta la reserva
al consumo real. Cuando el saldo se agota, el bot no se cae: conmuta a un modo degradado de reglas
fijas locales.

Se añade aquí también FR-09, el semáforo de concurrencia de CPU, porque pertenece a la misma
familia de decisiones: poner un techo explícito a lo que el proceso se permite hacer a la vez.

---

## Alcance

### Qué entra

* Middleware de control de admisión GCRA sin cerrojos, situado lo más al principio posible del
  pipeline HTTP, con respuesta `200 OK` sintética para el exceso (patrón *Fast-Reject*).
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
  la etapa 2 definió, con tiempos de espera y política de reintentos acotada.
* Métricas internas expuestas: peticiones admitidas y rechazadas por GCRA, tareas en vuelo, saldo
  disponible y desviación entre lo reservado y lo conciliado.

### Qué NO entra

* Precios, planes y recargas de saldo. Son decisiones de monetización pendientes; aquí se construye
  el mecanismo, no la política comercial.
* La conmutación de conocimiento y los embeddings: etapa 4. Esta etapa deja preparada la interfaz de
  contabilidad para que la ingesta por lotes la consuma.
* Las respuestas concretas del modo degradado como producto: se implementa el mecanismo con un
  conjunto mínimo de reglas, no un catálogo de mensajes comerciales.

### Requisitos del PRD cubiertos

* **FR-08** — control de admisión anti-spam mediante GCRA con Fast-Reject.
* **FR-09** — semáforo de concurrencia de CPU.
* **FR-10** — contabilidad financiera de dos fases con modo degradado.

---

## Entregables

* Módulo de admisión GCRA en `zeroclaw-core`, reutilizable y con pruebas propias.
* Middleware HTTP que lo integra en `zeroclaw-tenant`.
* Módulo de contabilidad con la máquina de estados de reserva y conciliación.
* Tablas de saldo y de movimientos en las migraciones de `sessions.db`.
* Cliente de inferencia real en un crate o módulo propio, detrás de la interfaz existente.
* `docs/adr/adr-0004-gcra-y-parametros.md` y
  `docs/adr/adr-0005-contabilidad-dos-fases.md`.
* Prueba de carga reproducible que envía 100 peticiones concurrentes contra el endpoint de webhook.

---

## Tareas

1. **Implementar el algoritmo GCRA** (1,5 días). Estructura sin cerrojos basada en operaciones
   atómicas, con una sola marca temporal por clave, y pruebas unitarias que verifiquen la tasa
   sostenida y la ráfaga tolerada.
2. **Integrarlo como middleware HTTP** (1 día). Colocarlo antes de la lectura del cuerpo de la
   petición para que el rechazo no asigne memoria, y devolver `200 OK` con cuerpo vacío al exceso.
3. **Parametrizar y documentar los límites** (0,5 días). Elegir tasa, ráfaga y clave de limitación;
   dejarlos configurables por variable de entorno y justificarlos en el ADR.
4. **Implementar el semáforo de concurrencia** (1 día). Límite de tareas en vuelo, adquisición sin
   bloqueo indefinido y comportamiento explícito ante saturación, coherente con el Fast-Reject.
5. **Diseñar el esquema de saldo y movimientos** (0,5 días). Migración con las tablas y sus
   restricciones de integridad.
6. **Implementar la reserva previa** (1 día). Estimación de coste a partir de la longitud del
   prompt, descuento atómico y rechazo limpio si no hay saldo suficiente.
7. **Implementar la conciliación posterior** (1 día). Ajuste con los tokens reales, devolución del
   sobrante, cargo del defecto y liberación de la reserva ante fallo o tiempo de espera agotado.
8. **Integrar el proveedor de inferencia real** (1,5 días). Cliente HTTPS saliente con tiempos de
   espera, reintentos acotados y extracción de los metadatos de tokens de la respuesta.
9. **Implementar el modo degradado** (1 día). Detección de saldo agotado, conmutación a reglas fijas
   locales, registro del evento y retorno automático al modo normal cuando hay saldo.
10. **Exponer métricas internas** (0,5 días). Contadores de admisión, rechazo, tareas en vuelo,
    saldo y desviación de conciliación, accesibles para la operación.
11. **Construir la prueba de carga** (1 día). Script reproducible de 100 peticiones concurrentes que
    mide latencia, códigos de respuesta y crecimiento de memoria residente.

---

## Criterios de aceptación

* **Ligado al criterio de QA "Prueba de Carga de Red" del PRD:** con 100 peticiones concurrentes, el
  middleware GCRA se activa, el exceso recibe `HTTP 200 OK` rápido y el consumo de memoria residente
  no crece más de un 15 % respecto de la línea base medida en la etapa 2.
* En ningún escenario de sobrecarga el servidor devuelve `429`, `502` o `503` hacia Meta.
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
| Parámetros de GCRA mal calibrados que rechazan tráfico legítimo. | Alto: mensajes de clientes reales perdidos silenciosamente, porque respondemos 200 sin procesar. | Registrar cada rechazo con su clave, empezar con límites holgados y ajustar con datos de la etapa 8. |
| Colocar el GCRA después de leer el cuerpo de la petición. | Medio: se pierde el beneficio de no asignar heap y la prueba de carga falla por consumo de memoria. | Fijar la posición del middleware por diseño y verificarlo con la métrica de memoria. |
| Estimación de prompt sistemáticamente inferior al coste real. | Medio: se permite gastar por encima del presupuesto. | Métrica de desviación entre reserva y conciliación, y factor de seguridad configurable en la estimación. |
| **Modelo de monetización sin definir** (pendiente en STATUS.md). | Medio: no se sabe cómo se recarga el saldo ni qué umbral dispara la degradación. | Se construye el mecanismo con valores configurables. La política comercial se inyecta como configuración cuando exista la decisión, sin tocar código. |
| El modo degradado se percibe como avería por el usuario final. | Medio. | El manejo de excepciones comerciales está pendiente de definición de producto; se deja el punto de extensión y se documenta el bloqueo. |

---

## Dependencias

* **De otras etapas:** etapa 2 completa. La contabilidad necesita `sessions.db` y sus migraciones; el
  middleware necesita el pipeline HTTP; el cliente real necesita la interfaz de inferencia.
* **Externas:** credenciales de al menos un proveedor de inferencia (Gemini, Groq u OpenRouter) y una
  cuenta con saldo para las pruebas de integración.
* **Decisiones de producto pendientes:** el **modelo de monetización** condiciona la calibración de
  saldos, umbrales y política de degradación. No bloquea la construcción del mecanismo, pero sí su
  puesta en producción con valores definitivos.
