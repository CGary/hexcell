# ADR-0039: Almacén del plano de control en `hexcell-admin`

**Fecha:** 2026-09-22  
**Estado:** Vigente  
**Decidido en:** HEX-083 (tarea 14 de la etapa A-6)

## Contexto

La CLI `hexcell-admin` necesita persistir el estado de control de cada célula para que los comandos `cell pause`, `cell unpause`, `cell status` y `cell list` puedan reportar el estado real de las células, no sólo inferirlo de Docker. Sin un almacén persistente, el estado es incognoscible desde un proceso que termina.

## Decisión

Se crea un almacén SQLite del plano de control en `crates/hexcell-admin/src/almacen_plano_de_control.rs`, siguiendo el mismo patrón de migración versionada que `crates/hexcell-storage/src/migraciones.rs`: guion SQL embebido con `include_str!` y `PRAGMA user_version` subido dentro de la misma transacción.

### Esquema

Tres tablas:

```sql
CREATE TABLE celulas (
    id TEXT PRIMARY KEY,
    estado TEXT NOT NULL,
    motivo TEXT NOT NULL DEFAULT '',
    actualizado_ms INTEGER NOT NULL
);

CREATE TABLE transiciones (
    id INTEGER PRIMARY KEY,
    id_celula TEXT NOT NULL,
    de TEXT NOT NULL,
    a TEXT NOT NULL,
    motivo TEXT NOT NULL,
    registrado_ms INTEGER NOT NULL
);

CREATE TABLE sustituciones (
    id INTEGER PRIMARY KEY,
    id_celula TEXT NOT NULL,
    motivo TEXT NOT NULL,
    registrado_ms INTEGER NOT NULL
);
```

### Ruta

La ruta del almacén se fija mediante la variable de entorno `HEXCELL_ADMIN_ALMACEN`, con valor por omisión `/var/lib/hexcell-admin/plano_de_control.db`. Si el directorio padre no existe, el comando falla con un diagnóstico claro; `hexcell-admin` nunca crea ese directorio.

### Reglas de escritura

* Las transiciones se validan contra `EstadoDeCelula::transiciones_permitidas` ANTES de emitir cualquier petición Docker.
* Una transición se persiste sólo DESPUÉS de que la operación Docker correspondiente tuvo éxito.
* Si una célula no tiene fila previa en `celulas`, se crea con motivo `alta_implicita`.
* `cell status` y `cell list` nunca escriben en el almacén bajo ninguna circunstancia, y esa garantía es **por construcción y no por convención**: los dos comandos abren el almacén con `abrir_solo_lectura`, que usa `SQLITE_OPEN_READ_ONLY` sin `SQLITE_OPEN_CREATE`, no aplica ninguna migración y rechaza con un error tipado un archivo ausente. Cualquier escritura que se colara en esas dos rutas de código fallaría con `SQLITE_READONLY` en vez de tocar el archivo, y un `HEXCELL_ADMIN_ALMACEN` que apunte a una ruta nueva devuelve `Fallo` con diagnóstico en lugar de dejar una base recién creada detrás de una consulta.

### Fuente que falla frente a discrepancia

Una **discrepancia** es un desacuerdo entre fuentes que todas respondieron. Una **fuente que falla** es otra cosa: un `inspect` que devuelve algo distinto de 404 (500 del demonio, socket inalcanzable, respuesta malformada) significa que Docker NO contestó, y entonces no se sabe si los contenedores existen.

`cell status` no confunde las dos. Ante una inspección que falla con cualquier error que no sea `NoEncontrado`, emite un diagnóstico que NOMBRA la fuente Docker y el contenedor, y devuelve `Fallo` sin emitir ningún `DISC-0N`. Derivar DISC-04 («los contenedores no existen en Docker») o DISC-05 («los contenedores existen en Docker») de un fallo de transporte sería afirmar por escrito como observado justamente lo que no se pudo observar. Sólo el 404 es una observación: el contenedor no existe.

### Codec de etiquetas

El almacén persiste las etiquetas de estado en formato ASCII snake_case (`en_ejecucion`, `suspendida`, etc.), no las etiquetas acentuadas con espacios del `Display` de `EstadoDeCelula`. El codec vive en el módulo del almacén, no en `estado_de_celula.rs`, para no violar las guardas de ese módulo.

## Consecuencias

* El almacén no contiene ningún identificador de transporte ni número de teléfono: sólo el id de célula y metadatos de estado del plano de control.
* El reloj no vive en el módulo del almacén: cada método de escritura recibe un `ahora_ms: i64` explícito, de modo que las pruebas son deterministas y la raíz de composición es la única dueña del reloj.
* La tabla `sustituciones` se crea en esta tarea pero sólo se lee; la tarea 13 (cell rebind) es la que escribe en ella.
* El gancho de transición `Retirada` con motivo `sesion_cerrada` queda declarado pero inerte en HEX-083, porque la tarea 12 (cell terminate) no está fusionada en main a fecha de este commit.

## Descartes

* No se añade un crate de migraciones: `PRAGMA user_version` en la misma transacción que el esquema es suficiente, siguiendo el precedente de `hexcell-storage`.
* No se deriva `serde` en `EstadoDeCelula`: el codec de etiquetas persistidas vive en el módulo del almacén, no en el agregado de estado.
* No se añade un método de listado a `ClienteDocker`: vive en un nuevo módulo `docker/inventario.rs` porque `cliente.rs` está fuera del alcance de esta tarea.
* No se usa el crate `tempdir` para las pruebas: se construyen las rutas temporales a mano con `std::env::temp_dir()`, `std::process::id()` y un contador atómico, siguiendo el patrón ya establecido en `tests/comun/mod.rs`.
