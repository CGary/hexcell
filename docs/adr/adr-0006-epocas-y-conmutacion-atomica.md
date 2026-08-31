# ADR-0006 — Conmutación atómica por épocas y gestión de bases de conocimiento en sombra

* **Estado:** Vigente desde el 2026-08-30.
* **Supersede a:** nada.
* **Etapa:** A-5 (HEX-055, FR-07/NFR-03).
* **Requisitos tocados:** FR-07 (conmutación atómica por épocas), NFR-03 (latencia de conmutación menor a 10 ms).

---

## Contexto

La indexación del catálogo de conocimiento de una célula genera representaciones vectoriales e índices relacionales en una base de datos en sombra (`knowledge_staging.db`) para no perturbar el tráfico de consultas de producción. Para poner en servicio el nuevo catálogo indexado (FR-07), el sistema debe transformar la base en sombra en la nueva época activa de conocimiento sin exponer a los lectores en vuelo a datos a medio escribir, sin interrumpir consultas concurrentes y garantizando un tiempo de conmutación imperceptible inferior a 10 milisegundos (NFR-03).

## Decisión

1. **Secuencia ordenada de seis pasos.**
   La conmutación se ejecuta como un proceso determinista:
   (1) Revalidación del índice en sombra mediante la lectura de la sonda semántica persistida (`leer_sonda_semantica`) y la ejecución de la compuerta de integridad (`validar_integridad_del_indice`); ante cualquier fallo o ausencia de sonda, la conmutación se aborta limpiamente sin alterar ningún archivo en disco.
   (2) Sellado de la época mediante una sentencia única `UPDATE` que fija simultáneamente `numero_de_epoca` y `sellada_ms`, seguido de la consolidación total del registro diario con `PRAGMA wal_checkpoint(TRUNCATE)`.
   (3) Renombrado físico del archivo `knowledge_staging.db` a `knowledge_epoch_N.db`.
   (4) Reasignación atómica del enlace simbólico `knowledge_live.db` hacia el nuevo archivo de época mediante el modismo POSIX.
   (5) Conmutación atómica del puntero del pool de conexiones en memoria (`ArcSwap`) empleando conexiones precalentadas y midiendo la latencia de intercambio.
   (6) Entrega del pool superseído vivo dentro de la estructura `EpocaSuperseida` para su posterior drenaje ordenado en la tarea 7 de la etapa A-5.

2. **Sellado previo a la consolidación del WAL.**
   El `UPDATE` que escribe el número de época y la marca temporal de sellado se ejecuta estrictamente **antes** de `PRAGMA wal_checkpoint(TRUNCATE)`. Dado que el punto de control vuelca todas las páginas del archivo `-wal` en el archivo principal y lo trunca a cero bytes, ejecutar la actualización después del punto de control escribiría registros en el diario que quedarían huérfanos al renombrar únicamente el archivo principal, provocando que la nueva época se leyera con metadatos nulos.

3. **Verificación estricta del resultado de `wal_checkpoint(TRUNCATE)`.**
   El resultado de `PRAGMA wal_checkpoint(TRUNCATE)` devuelve una tupla `(bloqueado, paginas_en_wal, paginas_consolidadas)`. Únicamente el valor exacto `(0, 0, 0)` se considera exitoso en una base sin lectores concurrentes; cualquier discrepancia aborta la conmutación antes del renombrado, protegiendo al sistema de promocionar bases con páginas no consolidadas.

4. **Identidad intrínseca y cálculo de N a partir del contenido.**
   El número ordinal N de la nueva época se calcula inspeccionando el campo `numero_de_epoca` de la tabla `metadatos_de_epoca` en cada archivo de base de datos del directorio, sumando uno al máximo valor observado. La identidad de la época es una propiedad interna del archivo (`HEX-049`), nunca una deducción a partir de nombres de archivo en el sistema operativo, permitiendo restauraciones consistentes de copias de seguridad con nombres modificados.

5. **Mecanismo único para la primera conmutación y conmutaciones posteriores.**
   En Linux, la llamada del sistema `rename()` sobre un enlace simbólico temporal reemplaza atómicamente tanto un archivo regular existente como un enlace simbólico previo en el mismo sistema de archivos. Por ende, la primera conmutación (donde `knowledge_live.db` era un archivo regular inicial) y las conmutaciones sucesivas comparten el mismo camino de ejecución. La particularidad de la primera época radica en sus consecuencias lógicas: N es 1, el archivo inicial queda desenlazado y se libera cuando se cierren sus descriptores, sin época previa a la cual revertir.

6. **Modismo POSIX de enlace temporal para conmutación atómica.**
   Para evitar ventanas de inexistencia donde `knowledge_live.db` apunte a la nada, la reasignación crea un enlace simbólico temporal en el mismo directorio relativo al archivo de época y lo renombra de forma atómica sobre `knowledge_live.db`. La implementación asume un entorno Unix/Linux (CachyOS en desarrollo, contenedores Docker en producción) mediante `std::os::unix::fs::symlink`.

7. **Adopción de `ArcSwap` frente a cerrojos `Mutex`/`RwLock`.**
   El gestor de almacenamiento `GestorDePools` se comparte entre múltiples servicios tras un puntero `Arc`. Utilizar un cerrojo tradicional impondría una penalización de sincronización en cada consulta de lectura de conocimiento en el camino crítico conversacional. `ArcSwap` permite lecturas concurrentes sin esperas ni contención, limitando la mutación al instante puntual de conmutación.

8. **Precalentamiento de conexiones para NFR-03.**
   El nuevo `PoolDeConocimiento` se instancia abriendo sus conexiones de solo lectura y aplicando los parámetros SQLite antes de realizar la conmutación en `ArcSwap`. Esto evita trasladar la latencia de apertura de descriptores y configuración de pragmas al intervalo medido de intercambio, asegurando una latencia menor a 10 milisegundos en el hardware objetivo.

9. **Frontera de entrega viva para drenaje ordenado.**
   El pool de lectura reemplazado no se cierra ni se destruye en esta operación. Se entrega intacto dentro de `EpocaSuperseida`, preservando los descriptores abiertos para que las consultas en vuelo finalicen normalmente hasta que el mecanismo de drenaje de la tarea 7 de A-5 gestione su clausura.

10. **Alcance de la medición de latencia.**
    La medición de latencia registrada en esta tarea corresponde a un intercambio en reposo sobre un único hilo coordinador, verificando el cumplimiento de NFR-03. La prueba de estrés con 20 lectores concurrentes simultáneos corresponde a la tarea 11 de la etapa A-5.

11. **Ruta reportada en la sonda de vitalidad.**
    Tras una conmutación, `PoolDeConocimiento::ruta` pasa a reportar la ruta canónica del archivo de época (`knowledge_epoch_N.db`), reflejándose en los mensajes de diagnóstico de la sonda de vitalidad sin alterar el identificador del componente.

## Consecuencias

### Positivas

* Conmutación de catálogos atómica y sin caída de servicio para lectores en vuelo.
* Prevención estructural de corrupción de datos gracias a compuertas estrictas previas al sellado y renombrado.
* Cálculo determinista del número de época basado en el contenido real de los archivos.
* Cumplimiento del presupuesto de latencia NFR-03 (< 10 ms) mediante precalentamiento y `ArcSwap`.
* Desacoplamiento limpio con la futura lógica de drenaje ordenado (tarea 7).

### Negativas

* Requiere soporte de enlaces simbólicos y llamadas `rename()` atómicas en el mismo sistema de archivos (restringido a Linux/POSIX).
* El archivo de la base inicial queda huérfano en disco tras la primera conmutación hasta que se cierren sus descriptores.

## Alternativas consideradas y descartadas

Las alternativas descartadas (envolver el pool en un cerrojo Mutex/RwLock, reasignación con borrado previo `unlink`+`symlink`, copia destructiva sobre el archivo vivo y reinicio del proceso) se encuentran registradas en la [Bitácora de Descartes](../bitacora-de-descartes.md) bajo la entrada D-29.

## Referencias

* `crates/hexcell-storage/src/promocion.rs`: implementación síncrona de la conmutación.
* `crates/hexcell-storage/src/pools.rs`: integración de `ArcSwap` y apertura de pools.
* `crates/hexcell/src/promocion.rs`: orquestación asíncrona en el binario.
* `docs/bitacora-de-descartes.md`: entrada D-29.
* `docs/adr/adr-0003-persistencia-dual.md`: persistencia dual SQLite.
* `docs/adr/adr-0025-puerto-de-embeddings.md`: esquema de conocimiento y vectores.
