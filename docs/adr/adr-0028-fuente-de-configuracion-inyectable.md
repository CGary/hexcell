# ADR 0028: Fuente de configuración inyectable como puerto, y prohibición de escribir el entorno del proceso en pruebas

- **Estado**: Vigente (2026-09-01)
- **Fecha**: 2026-09-01
- **Decisores**: Gary (Arquitecto de Sistemas), equipo Hexcell
- **Relaciones**:
  - Complementa [ADR 0023](adr-0023-parametros-gcra-por-variable-de-entorno.md) (Parámetros GCRA por variable de entorno): no cambia qué variables existen ni qué significan, solo de dónde se leen.
  - Aplica al arranque el mismo principio que [ADR 0010](adr-0010-puerto-de-canal.md) aplica al canal y que el reloj inyectable (`RelojDePrueba` / `RelojDelSistema`) aplica al tiempo.

---

## Contexto

`cargo test --workspace` fallaba de forma intermitente sin causa registrada: medido el 2026-09-01, 1 fallo en 25 corridas consecutivas (≈ 4 %), con pánico en `crates/hexcell/src/motor.rs:518`. No era una aserción frágil: era comportamiento indefinido real.

El mecanismo es concreto. `Cargo.toml` fija la edición 2024, donde escribir el entorno del proceso es una operación `unsafe` porque `setenv` de glibc puede **reasignar el array `environ`** mientras otro hilo lo está leyendo. `cargo test` ejecuta los tests de un mismo binario en hilos del **mismo proceso**. En el árbol convivían tres instancias del mismo defecto:

1. `crates/hexcell/src/configuracion.rs` escribía el entorno bajo un mutex local del módulo (`BLOQUEO_ENTORNO`) que ningún otro módulo tomaba, mientras `crates/hexcell/src/motor.rs` leía el entorno con `std::env::temp_dir()` —que es un `getenv`— desde otro hilo del mismo binario.
2. `crates/hexcell/tests/configuracion.rs` (708 líneas, 66 escrituras, 18 tests) tenía su propio cerrojo (`CERROJO_DE_ENTORNO`) y 15 lecturas de `temp_dir` en el mismo binario.
3. `crates/hexcell/tests/promocion.rs` escribía el entorno **sin cerrojo alguno**.

Un cerrojo local solo excluye a quien lo toma. El escritor y el lector estaban en módulos distintos, así que la exclusión mutua nunca fue tal. Arreglar solo el binario de biblioteca habría dejado vivas las otras dos instancias.

Además, el ayudante de pruebas de `motor.rs` destruía la evidencia de su propio fallo cada vez que la carrera se disparaba: descartaba el error de `create_dir_all` con `let _ =` y luego entraba en pánico con un `panic!()` sin mensaje. Por eso el defecto sobrevivió varias tareas sin diagnóstico.

---

## Decisión

1. **La configuración se lee por un puerto inyectado, no de estado ambiental.** Se declara en `crates/hexcell/src/configuracion.rs` el trait `FuenteDeConfiguracion` con un único método `leer(&self, nombre: &str) -> Option<String>`, y dos implementaciones: `EntornoDelProceso` (producción, único punto del crate que llama a `std::env::var`, y **solo lee**) y `FuenteEnMemoria` (doble de prueba sobre una tabla ordenada, valor local de quien la construye).

2. **La fuente es un parámetro de constructor, nunca un campo ni un global.** `Configuracion::desde_fuente(&dyn FuenteDeConfiguracion)` concentra toda la lógica; `Configuracion::desde_entorno()` queda como envoltorio delgado de producción que delega en `desde_fuente(&EntornoDelProceso)`, de modo que la raíz de composición (`main`) no cambia. Se prohíbe expresamente sostener la fuente en un `static`, un `thread_local`, un `OnceLock` o un campo de `Configuracion`: la fuente se consulta una vez, durante la construcción, y retenerla conservaría un asa viva sobre el entorno del proceso, que es justo el acoplamiento que este ADR elimina.

3. **Los cuatro grupos de lectores quedan parametrizados, no solo uno.** Además de `Configuracion`, reciben la fuente por parámetro `respaldar::ejecutar_cli`, `emparejar::ejecutar_cli` y las dos funciones libres de `promocion` (renombradas a `limite_de_drenaje_de_epoca_desde_fuente` y `ventana_de_retencion_de_epocas_desde_fuente`). Parametrizar solo `Configuracion` habría dejado pasar la guarda de grep con el acoplamiento intacto.

4. **Ningún archivo bajo `crates/hexcell/` escribe el entorno del proceso.** La prohibición se verifica mecánicamente en CI con una guarda de grep que también prohíbe la reaparición de los dos cerrojos (`BLOQUEO_ENTORNO`, `CERROJO_DE_ENTORNO`): si nadie escribe, no hay nada que serializar, y un cerrojo nuevo sería la señal de que alguien volvió a escribir.

5. **El ayudante de pruebas de `motor.rs` deja de destruir su evidencia.** El error de `create_dir_all` se propaga en un pánico que nombra la ruta y el error de origen; el fallo al abrir el gestor de pools nombra ruta y error en vez de un `panic!()` vacío; y el nombre del directorio temporal se deriva de un contador atómico de proceso (`AtomicU64`) en vez de una lectura de nanosegundos del reloj, de modo que dos ayudantes concurrentes se distinguen **por construcción** y no por una propiedad de granularidad del reloj.

6. **`FuenteEnMemoria` no va detrás de `#[cfg(test)]`.** Los tests de integración de `crates/hexcell/tests/` compilan como crates externos y no verían un elemento condicionado a la compilación de pruebas de la biblioteca.

---

## Consecuencias

### Positivas
- **El comportamiento indefinido desaparece por construcción**, no por serialización: no queda ningún escritor del entorno contra el que competir. La verificación empírica es la corrida de 25 ejecuciones consecutivas de `cargo test --workspace` en verde.
- **Los tests de configuración pueden correr en paralelo** sin cerrojo y sin limpieza posterior: cada uno prepara su caso en una tabla local que nadie más ve.
- **Un fallo intermitente futuro será diagnosticable** desde la salida del test, porque el ayudante ya no descarta errores ni entra en pánico sin mensaje.

### Negativas / Mitigaciones
- **Los servicios que leen configuración cargan un parámetro más.** Es el coste explícito de que la dependencia sea visible en la firma en vez de estar escondida en una llamada a `getenv`; el mismo coste que ya se aceptó para el reloj y para el puerto de canal.
- **La guarda de grep es un instrumento romo**: prohíbe el texto, no la semántica, y obliga a redactar la documentación del propio módulo sin citar literalmente la llamada prohibida. Se acepta porque es verificable en CI sin analizar el árbol sintáctico.
- **`crates/hexcell/src/procesador.rs` conserva el mismo patrón de nombrado de directorio temporal por granularidad de reloj** (líneas 300 y 364). Queda deliberadamente fuera de alcance por decisión humana del 2026-09-01: ningún criterio de aceptación de esta tarea lo cubre. Su riesgo de colisión es benigno una vez que ningún hilo escribe el entorno, y se trata como tarea de seguimiento.

---

## Alternativas descartadas

Registradas con su motivo y su condición de reapertura en [../bitacora-de-descartes.md](../bitacora-de-descartes.md): **D-33** (serializar el binario de test con `--test-threads=1`) y **D-34** (mover los tests que mutan el entorno a un binario de integración aparte).
