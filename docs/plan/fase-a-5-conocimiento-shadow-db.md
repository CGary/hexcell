# Fase A · Etapa 5 — Motor de conocimiento: Shadow DB y conmutación por épocas

**Duración relativa:** Larga.

---

## Objetivo

Un bot de atención al cliente vale lo que vale su conocimiento: el catálogo de productos, las
preguntas frecuentes, los horarios, las reglas de negocio. Ese conocimiento cambia, y cambia
precisamente en los momentos en que el negocio está activo. Esta etapa resuelve cómo actualizarlo
**sin detener la producción y sin corromper nada**.

El problema técnico de fondo es que construir el índice de conocimiento implica llamar por lotes a
una API externa de embeddings, una operación lenta, cara y sujeta a fallos parciales. Hacerlo sobre
la base que está sirviendo consultas RAG en ese instante es la receta para bloqueos de escritura y
errores `SQLITE_BUSY`. La solución que fija el PRD es aislar por completo esa construcción en una
base en sombra, `knowledge_staging.db` (FR-06), y promoverla solo cuando esté íntegra.

La promoción, además, no puede consistir en sobrescribir un archivo mientras hay lectores abiertos:
SQLite en modo WAL mantiene descriptores sobre los archivos auxiliares `-wal` y `-shm`, y borrarlos
bajo los pies de un lector es una forma segura de corromper datos. FR-07 define por ello una
secuencia de cuatro pasos —sellar el WAL, renombrar a una época inmutable, reasignar el enlace
simbólico y el puntero en memoria con `ArcSwap`, y drenar el pool antiguo de forma asíncrona— que
consigue una conmutación por debajo de los 10 milisegundos (NFR-03) sin que ningún lector en vuelo
vea el suelo desaparecer.

Es la etapa técnicamente más delicada del plan. Un fallo aquí no se manifiesta como un error
inmediato, sino como corrupción silenciosa de datos días después. Nada de esto depende del canal: el
motor de conocimiento es idéntico en ambas fases y sobrevive intacto al cambio de adaptador.

---

## Alcance

### Qué entra

* Esquema de conocimiento: documentos, fragmentos, metadatos y vectores de embedding.
* Pipeline de ingesta: recepción de un payload JSON de conocimiento, fragmentación del texto,
  llamada por lotes a la API externa de embeddings y escritura en `knowledge_staging.db`.
* Sometimiento de la ingesta a la contabilidad de dos fases de la etapa A-4, para que el coste de los
  embeddings esté presupuestado igual que el de la inferencia.
* Validación de integridad estructural y semántica del índice antes de promoverlo: recuento de
  fragmentos, dimensionalidad de los vectores, ausencia de nulos y una consulta de prueba que debe
  devolver resultados coherentes.
* Secuencia atómica de promoción por épocas: `PRAGMA wal_checkpoint(TRUNCATE)`, renombrado a
  `knowledge_epoch_N.db`, reasignación atómica del enlace simbólico y sustitución del pool en
  memoria mediante `ArcSwap`.
* Drenaje controlado del pool obsoleto, con espera a las lecturas en vuelo y liberación verificada
  de los descriptores `-wal` y `-shm`.
* Retención de épocas históricas y reversión a la época anterior si la nueva resulta defectuosa.
* Motor de recuperación (RAG): búsqueda de los fragmentos más similares al mensaje del usuario y
  construcción del contexto que se envía al modelo.
* Endpoint interno de administración de la célula para disparar una actualización de conocimiento.

### Qué NO entra

* El panel de administración web desde el que un cliente carga su catálogo. Aquí se expone el
  endpoint que lo recibiría; la interfaz de usuario depende de flujos de producto pendientes.
* La curaduría del contenido de conocimiento de cada microempresa, que es trabajo de onboarding
  comercial, no de ingeniería. La carga inicial de las células piloto es de la etapa A-7.
* Cualquier cambio en el plano de control: la CLI es de la etapa A-6 y Caddy de la etapa B-2.

### Requisitos del PRD cubiertos

* **FR-06** — indexación en sombra sin bloquear la producción.
* **FR-07** — conmutación atómica por épocas con drenaje controlado.
* **NFR-03** — conmutación interna de la base de conocimiento por debajo de 10 milisegundos.

---

## Entregables

* Módulo de conocimiento en `hexcell-storage` con el gestor de épocas y el pool intercambiable.
* Módulo de ingesta con fragmentación, llamada por lotes a embeddings y escritura en staging.
* Módulo de recuperación RAG que consume el pool vigente sin conocer su época.
* Cliente de la API externa de embeddings, integrado con la contabilidad de la etapa A-4.
* Migraciones y esquema de la base de conocimiento.
* `docs/adr/adr-0006-epocas-y-conmutacion-atomica.md`, con la secuencia exacta y su
  justificación.
* Prueba de estrés que ejecuta una conmutación mientras se sirven lecturas RAG concurrentes.

---

## Tareas

1. **Diseñar el esquema de conocimiento** (1 día). Documentos, fragmentos, vectores y metadatos;
   decidir y documentar cómo se almacenan y consultan los embeddings.
2. **Implementar la fragmentación de contenido** (1 día). Estrategia de troceado con solapamiento,
   parametrizada y con pruebas sobre casos límite (texto muy corto, muy largo, listas).
3. **Integrar el cliente de embeddings por lotes** (1,5 días). Llamadas agrupadas, tiempos de espera,
   reintentos acotados, reanudación tras fallo parcial y consumo de la contabilidad de dos fases.
4. **Construir el pipeline de ingesta a `knowledge_staging.db`** (1,5 días). Creación de la base en
   sombra desde cero en cada ejecución, escritura de fragmentos y vectores, y aislamiento total
   respecto de la base viva.
5. **Implementar la validación de integridad del índice** (1 día). Comprobaciones estructurales y una
   consulta semántica de prueba con umbral de aceptación; si falla, la promoción se aborta y la
   producción sigue intacta.
6. **Implementar la secuencia de promoción** (2 días). Checkpoint con truncado del WAL, renombrado a
   la época siguiente, reasignación atómica del enlace simbólico y sustitución del puntero del pool
   con `ArcSwap`. Es la tarea de mayor riesgo de la etapa.
7. **Implementar el drenaje controlado del pool antiguo** (1,5 días). Cierre asíncrono que espera a
   las lecturas en vuelo, con límite temporal, y verificación de que no quedan archivos `-wal` ni
   `-shm` huérfanos.
8. **Implementar retención y reversión de épocas** (1 día). Cuántas épocas se conservan, cómo se
   purgan las antiguas y cómo se vuelve a la anterior ante un problema detectado en producción.
9. **Implementar el motor de recuperación RAG** (1,5 días). Búsqueda por similitud sobre el pool
   vigente, selección de los fragmentos más relevantes y construcción del contexto del prompt.
10. **Exponer el endpoint interno de actualización** (0,5 días). Ruta administrativa de la célula,
    accesible solo desde la red interna, que dispara la ingesta y devuelve el estado del proceso.
11. **Construir la prueba de estrés de conmutación** (1 día). Intercambio de conocimiento bajo 20
    lecturas RAG simultáneas, con medición del tiempo de conmutación y verificación del sistema de
    archivos al terminar.
12. **Verificar la interacción con el respaldo** (0,5 días). Comprobar que una conmutación de época
    durante un respaldo en curso no produce copias inconsistentes ni épocas huérfanas, y ajustar el
    procedimiento de la etapa A-2 si hiciera falta.

---

## Criterios de aceptación

* **Ligado al criterio de QA "Prueba de Consistencia en Modo WAL" del PRD:** una conmutación de
  conocimiento ejecutada durante 20 lecturas RAG simultáneas no produce ninguna excepción
  `SQLITE_BUSY` ni deja archivos `.db-wal` o `.db-shm` huérfanos en disco.
* El tiempo transcurrido entre el inicio de la reasignación del puntero y la primera lectura servida
  por la nueva época es inferior a 10 milisegundos, medido y registrado (NFR-03).
* Ninguna lectura RAG en vuelo durante la conmutación falla ni devuelve resultados de una época
  parcialmente construida.
* Un fallo a mitad de la ingesta deja `knowledge_live.db` intacto y sirviendo la época anterior.
* Si la validación de integridad falla, la promoción se aborta y el sistema continúa en la época
  vigente sin intervención manual.
* Es posible revertir a la época anterior mediante una operación explícita, y las lecturas pasan a
  servirse de ella sin reiniciar el proceso.
* Tras el drenaje, el número de descriptores de archivo abiertos por el proceso vuelve al valor
  previo a la conmutación.
* Un respaldo ejecutado durante una conmutación produce una copia consistente y restaurable.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Liberar los archivos de la época antigua antes de que terminen las lecturas en vuelo. | Muy alto: corrupción de datos y caídas intermitentes difíciles de reproducir. | Drenaje explícito con espera y verificación de descriptores; prueba de estrés obligatoria antes de cerrar la etapa. |
| El checkpoint con truncado no se completa por haber lectores activos sobre staging. | Alto: la época se sella a medias. | Garantizar que la base de staging no tiene lectores por construcción, y comprobar el resultado del `PRAGMA` antes de renombrar. |
| Coste descontrolado de la API de embeddings en catálogos grandes. | Medio: gasto imprevisto por célula. | La ingesta pasa por la contabilidad de dos fases de la etapa A-4 y se aborta si no hay saldo. |
| Búsqueda vectorial demasiado lenta en hardware modesto. | Medio: latencia de respuesta del bot fuera de lo aceptable. | Medir con catálogos representativos desde el principio y acotar el número de fragmentos por célula; si no basta, revisar la estrategia de indexado antes de la etapa A-6. |
| El diseño de rutas y enlaces simbólicos no sobrevive al montaje de volúmenes en Docker. | Medio: retrabajo en la etapa A-6. | Fijar aquí la disposición definitiva del directorio de datos y validarla en la etapa A-6 antes de cerrar el `Dockerfile`. |
| Una conmutación durante un respaldo produce una copia inconsistente. | Alto: el respaldo existe pero no restaura. | Tarea 12 explícita, con ajuste del procedimiento de la etapa A-2 si es necesario. |

---

## Dependencias

* **De otras etapas:** etapa A-2 (pools duales, `knowledge_live.db`, respaldo y apagado ordenado) y
  etapa A-4 (contabilidad de dos fases para presupuestar los embeddings).
* **Externas:** credenciales y cuota de una API de embeddings; un conjunto de datos de catálogo
  representativo para las pruebas de rendimiento.
* **Decisiones de producto pendientes:** la forma en que un cliente entrega su catálogo (panel web,
  carga de archivo, integración) depende de los **flujos de usuario finales** de STATUS.md. Esta
  etapa entrega el endpoint interno; la superficie de cara al cliente queda bloqueada. Para los dos
  pilotos de la etapa A-7 la carga se hace manualmente contra ese endpoint.
