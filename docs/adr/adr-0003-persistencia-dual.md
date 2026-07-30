# ADR-0003 — Persistencia dual SQLite y parámetros elegidos

* **Estado:** Vigente desde el 2026-07-30.
* **Supersede a:** nada.
* **Etapa:** A-2 (HEX-006).
* **Requisitos tocados:** FR-05, NFR-01, NFR-02.

---

## Contexto

`docs/PRD.md` (FR-05) fija que cada célula persiste en **dos** bases SQLite separadas:
`sessions.db`, de lectura y escritura caliente, y `knowledge_live.db`, de solo lectura en
producción. Hasta HEX-005 esa decisión estaba tomada en el PRD y no formalizada, y el estado que
debía vivir en `sessions.db` —el registro de deduplicación y el historial de conversación— vivía en
un `HashMap` y un `Vec` del proceso, que un reinicio borraba.

Esta tarea escribe esa persistencia, así que es el momento de formalizar la decisión y, sobre todo,
de **escribir la contrapartida de cada parámetro de SQLite elegido**. La tabla de riesgos de
`docs/plan/fase-a-2-nucleo-persistencia.md` nombra explícitamente «copiar ajustes de SQLite sin
entenderlos» como riesgo, y la mitigación que pide es exactamente este documento: ningún parámetro
sin su trade escrito, aquí y en el punto del código donde se aplica.

El hardware objetivo encuadra todas las decisiones: un i7 de diez años con 8 GB de RAM compartidos
entre todas las células, disco compartido, y un presupuesto de línea base de ≤ 80 MB de memoria por
célula sobre canal propio (NFR-01).

## Decisión

### 1. Dos bases separadas, no una

`sessions.db` y `knowledge_live.db` se derivan de la ruta de datos ya validada de la célula
(`HEXCELL_RUTA_DATOS`) y **no** se configuran por ninguna variable de entorno propia: un mando
ajustable sobrevive siempre al motivo por el que se creó.

La separación no es organizativa. Las dos bases tienen patrones de acceso opuestos —una se escribe
en el camino caliente de cada mensaje, la otra se lee y no se escribe nunca en producción— y
juntarlas haría que una lectura de conocimiento tuviera que esperar detrás del escritor de sesiones.
Además, la separación es lo que hace posible la conmutación atómica por épocas de FR-07 que diseña
la etapa A-5: una base que se sustituye entera no puede ser la misma que guarda el estado vivo.

### 2. El motor es `rusqlite` de la serie 0.39, con la característica `bundled`, y nada más

`rusqlite` es un enlace directo a SQLite sin capa de abstracción de bases de datos, que es
exactamente lo que esta capa necesita: el SQL de la célula es corto, explícito y revisable.

**La serie 0.39 está fijada a propósito.** Comprobado el 2026-07-30: la serie siguiente arrastra
`libsqlite3-sys` 0.38.1, cuyo script de compilación usa la macro todavía inestable `cfg_select!` y
falla con E0658 sobre el canal 1.92.0 que fija `rust-toolchain.toml`; la 0.39 arrastra
`libsqlite3-sys` 0.37.0 y compila limpio. El motivo está escrito en el comentario de
`[workspace.dependencies]` porque, sin él, la próxima actualización de dependencias reintroduce un
fallo de compilación cuya causa está a tres crates de distancia de cualquier cosa que se haya
tocado.

`bundled` compila SQLite dentro del binario. La célula se despliega en una imagen mínima (etapa
A-6) y no puede depender de qué versión de la biblioteca de SQLite tenga el sistema anfitrión.

**Se descartan los pools de conexiones externos** —la familia de `r2d2`, `deadpool` y equivalentes—
por el mismo argumento que este repositorio ya aplicó a `axum` y a `tiny-http` en
`crates/hexcell/Cargo.toml`: pagan generalidad que aquí no compra nada. SQLite **serializa a los
escritores por diseño**, así que un pool de N conexiones de escritura no escribiría en paralelo:
convertiría una espera ordenada dentro del proceso en `SQLITE_BUSY`. Encima, un pool de ese tipo
mantiene un hilo de fondo segando conexiones ociosas, coste puro sobre el hardware objetivo.

**Se descarta `sqlx`** por su árbol de dependencias y por su modelo asíncrono, que impondría un
ejecutor a una capa que no debe tenerlo.

**Se descartan los crates de migraciones** (`refinery`, `rusqlite_migration` y equivalentes):
añadirían una tabla de versiones que duplica lo que `PRAGMA user_version` ya guarda en la cabecera
del archivo, con la diferencia de que la tabla puede desincronizarse del esquema y la cabecera no.

### 3. Tamaño de los pools

| Pool | Conexiones | Motivo |
| :--- | :--- | :--- |
| `sessions.db`, escritura | 1 | SQLite serializa a los escritores; más de una no escribe más rápido, solo produce `SQLITE_BUSY`. |
| `sessions.db`, lectura | 1 | Separada de la de escritura para que una lectura de historial no espere detrás de la escritura en curso, que es justo lo que WAL permite. Una basta: una célula sirve tráfico conversacional bajo. |
| `knowledge_live.db`, lectura | 2 | Reparto por turno rotatorio. Dos y no más: cada conexión paga su propia caché de páginas contra los 8 GB compartidos entre todas las células. |

Ninguno de estos números es configurable por variable de entorno: son constantes con nombre y con
su justificación en el punto de declaración.

### 4. Parámetros de SQLite, cada uno con su contrapartida

| Parámetro | Valor | Qué compra | Qué cuesta |
| :--- | :--- | :--- | :--- |
| `journal_mode` | `WAL` | Lecturas y escritura avanzan a la vez en vez de excluirse; es el patrón exacto de una célula (escrituras cortas y frecuentes junto a lecturas de historial). | Un archivo adicional (`-wal`) y la necesidad de puntos de control, que SQLite hace por tamaño sin intervención. |
| `busy_timeout` | 5000 ms | Un choque breve entre conexiones espera en vez de fallar. Sin él, el valor por defecto es cero y el primer choque devuelve `SQLITE_BUSY`, que en producción se vería como pérdida de mensajes. | Una operación que choque de verdad tarda hasta cinco segundos en rendirse, en un proceso que además atiende el servidor de salud. |
| `synchronous` | `NORMAL` | Evita un `fsync` por transacción sobre el disco de un equipo de diez años, en el camino caliente de cada mensaje. | **Un corte de luz o una caída del sistema operativo pueden perder transacciones ya confirmadas desde el último punto de control. Una caída del proceso no pierde ninguna**, porque los datos ya están en manos del sistema de archivos. |
| `foreign_keys` | `ON` | Las referencias declaradas en la migración son restricción y no documentación: un parámetro de plantilla no puede quedar apuntando a un mensaje inexistente. | SQLite las trae desactivadas por compatibilidad histórica; activarlas en cada conexión es un paso explícito que no se puede olvidar. |

El escenario que `synchronous = NORMAL` acepta —corte de luz— es precisamente del que se restaura
con la política de respaldos que diseña esta misma etapa A-2. Es un cambio de una pérdida posible y
recuperable por un coste continuo en el camino caliente.

### 5. Migraciones por `PRAGMA user_version`, en la misma transacción que el esquema

Los guiones `.sql` viven en `crates/hexcell-storage/migraciones/` y entran en el binario por
`include_str!`: son legibles como SQL, con su propio historial en el repositorio, y a la vez no
crean ninguna dependencia de archivos en tiempo de ejecución que la imagen de la etapa A-6 pudiera
no copiar. El cambio de esquema y la subida de versión ocurren en **una sola** transacción: o quedan
los dos, o no queda ninguno.

### 6. La sonda de vitalidad comprueba el archivo, no solo la consulta

Comprobado el 2026-07-30: en Linux, borrar el archivo de una base **no** perturba a una conexión ya
abierta, porque el descriptor sigue apuntando al inodo. Una sonda que solo lanzara una consulta
seguiría respondiendo que todo va bien sobre una base que ya no existe en disco. La sonda comprueba
las dos cosas: que la ruta sigue existiendo **y** que una consulta barata contra una tabla real
responde.

### 7. `knowledge_live.db` nace con una tabla de metadatos y nada más

Su esquema real lo diseña la etapa A-5, con la Shadow DB y las épocas inmutables. La tabla mínima
existe por una razón operativa: abrir en `SQLITE_OPEN_READ_ONLY` un archivo que no existe falla, así
que la célula crea la base una vez en lectura y escritura, la migra, la cierra y solo entonces abre
el pool de producción.

## Consecuencias

* El registro de deduplicación y el historial de conversación **sobreviven a un reinicio**, y
  `sessions.db` es su **única** fuente de verdad: no queda ninguna caché en memoria delante. La cola
  de respuestas diferidas es la excepción documentada y sigue en memoria, con su motivo escrito en
  `crates/hexcell/src/conversaciones.rs`.
* `GET /health/ready` deja de ser un esqueleto: responde la conjunción de las dos vitalidades y del
  estado de sesión del canal.
* La capa es **síncrona**. Una escritura larga bloquea el hilo único de la célula (`current_thread`,
  NFR-01). Se acepta a sabiendas: las escrituras son de una fila y la contención esperada es
  mínima con una sola célula por base. Revisar si la etapa A-7 mide latencias que lo contradigan.
* `synchronous = NORMAL` debe revisarse cuando la etapa A-4 añada contabilidad financiera de LLM:
  perder una transacción confirmada de saldo no es lo mismo que perder una anotación de historial.
  Queda registrado como decisión `Pendiente` en `docs/STATUS.md`.
* El canal propio sigue siendo el canal por defecto y permanente, y esta capa le sirve igual que
  servirá al canal oficial cuando se incorpore: la persistencia no conoce ningún transporte.

## Alternativas descartadas

* **Una sola base con todas las tablas.** Haría imposible la conmutación por épocas de FR-07 sin
  reescribir también el estado vivo, y ataría las lecturas de conocimiento al escritor de sesiones.
* **`synchronous = FULL`.** Un `fsync` por transacción en el camino caliente sobre el disco del
  hardware objetivo, para cubrir un escenario del que ya se restaura con respaldos.
* **Guardar los instantes como texto ISO-8601.** Ordenar y podar sobre texto es más caro de comparar
  e indexar que sobre un entero, y no aporta nada que un entero de milisegundos no dé.
* **Serializar los parámetros de plantilla en una sola columna.** La lista es ordenada y de longitud
  variable, y cualquier separador rompe en cuanto un parámetro lo contiene.
