# adr-0024 — Métricas operativas internas expuestas por instantánea estructurada en log periódico

* **Estado:** Vigente (2026-08-27).
* **Etapa que lo produce:** A-4 (tarea 11 del plan de la etapa A-4: `docs/plan/fase-a-4-admision-presupuesto.md`).
* **Relación con otros ADR:** Cita los requerimientos **FR-08**, **FR-09**, **FR-10** de `docs/PRD.md`, y extiende `adr-0019-registro-estructurado.md`.

## Contexto

Para la supervisión operativa de una célula en producción, es necesario poder observar el rendimiento interno y estado financiero de forma remota sin necesidad de acoplar un depurador. En concreto, el operador necesita observar:
1. Eventos admitidos y descartados por control de admisión GCRA (**FR-08**).
2. Eventos descartados por saturación de concurrencia (**FR-09**).
3. Tareas en vuelo concurrentes (**FR-09**).
4. Estado de saldo disponible y reservado en `sessions.db` (**FR-10**).
5. Desviación de conciliación de presupuesto acumulada (**FR-10**).

El diseño debe respetar estrictamente los siguientes límites:
* No introducir endpoints HTTP de entrada (Fase A no tiene red entrante por diseño).
* No añadir tablas de métricas ni escrituras a `sessions.db` en la ruta crítica para evitar contención del escritor único WAL.
* No alterar la CLI `hexcell-admin` ya que es un stub de 10 líneas y un proceso externo no puede consultar semáforos en memoria.
* Minimizar la huella de memoria del binario en reposo.

## Decisión

**1. Emisión periódica de instantánea en log estructurado:**
El mecanismo de exposición elegido consiste en una tarea en segundo plano que, de forma periódica cada 60 segundos (`INTERVALO_DE_INSTANTANEA`), toma una instantánea del estado de la célula y emite una línea de log estructurado con el nombre de evento `metricas_instantanea`. 
El detalle se formatea como una cadena de texto simple en formato `key=value` (espacios como delimitador) asignada al campo `detalle` de `EntradaDeRegistro`. Esto respeta `adr-0019` sin alterar la estructura fija del log ni añadir dependencias JSON complejas.

**2. Almacenamiento local en memoria y base de datos:**
Las métricas se recogen de dos fuentes:
* **En memoria:** Contadores atómicos en `RegistroDeMetricas` (`admitidos`, `descartados_admision`, `descartados_concurrencia`) incrementados con ordenación relajada en el motor, más el cálculo dinámico del indicador `en_vuelo` derivado de los permisos libres del semáforo en `LimitadorDeConcurrencia`.
* **En disco:** Consultas de solo lectura rápidas sobre `sessions.db` (`saldo()` para el saldo disponible/reservado, y `desviacion_de_conciliacion()` para la agregación de movimientos de conciliación).

**3. Inyección aditiva en Motor sin romper firmas:**
El registro de métricas se añade de forma opcional mediante el patrón builder `con_metricas` en el motor, defaulting en `Motor::nuevo` a un registro local para mantener la compatibilidad absoluta con todos los tests unitarios e integrados previos.

## Alternativas consideradas y descartadas

### (a) Endpoint HTTP en servidor de salud `/metrics` — **DESCARTADA**
Agregar un endpoint `/metrics` al servidor de salud HTTP loopback existente fue descartado para respetar de forma estricta la invariante de no añadir nuevas superficies externas de red ni alterar la frontera del servidor de salud, reservado a sondeos sencillos.

### (b) Comando CLI de consulta `hexcell-admin` — **DESCARTADA**
Una herramienta CLI externa puede leer la base de datos pero no tiene acceso al estado en memoria de la célula (contadores y semáforo de concurrencia de tareas activas). Exponerlas requeriría IPC de consulta complejo e innecesario.

### (c) Tabla de historial de métricas en base de datos — **DESCARTADA**
Escribir métricas en `sessions.db` añadiría escrituras periódicas frecuentes al WAL en el hilo único de base de datos, compitiendo con el flujo de mensajes. El operador solo requiere valores vivos en tiempo real, no series temporales durables locales.

## Consecuencias

* Se obtiene visibilidad total del rendimiento de la célula mediante logs agregados tradicionales de producción.
* La huella en caliente de memoria del binario se mantiene insignificante al utilizar atómicos locales y una única tarea en segundo plano.
* No se modifican las firmas existentes en tests de integración previos.

## Referencias

* `docs/PRD.md` (FR-08, FR-09, FR-10).
* `docs/adr/adr-0019-registro-estructurado.md`.
* `crates/hexcell/src/metricas.rs`.
