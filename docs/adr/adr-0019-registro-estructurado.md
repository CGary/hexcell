# ADR-0019 — Registro estructurado sin crate de logging

* **Estado:** Vigente desde el 2026-07-30.
* **Supersede a:** nada.
* **Etapa:** A-2 (HEX-007).
* **Requisitos tocados:** NFR-01 (presupuesto de memoria), operabilidad mínima de una célula.

---

## Contexto

Hasta esta tarea, el motor de mensajería escribía su progreso con `println!`/`eprintln!` sueltos,
sin ninguna estructura ni campo consistente. Diagnosticar una célula en producción — cuánto tarda
en responder, si un evento se duplicó, si un envío se difirió — exige algo más comprobable que
texto libre, pero el presupuesto de memoria de la célula (≤ 80 MB sobre canal propio, NFR-01) y el
tamaño del binario descartan de entrada una biblioteca de logging completa.

## Decisión

1. **El registro se escribe a mano: un objeto JSON por línea en `stdout`, sin ningún crate de
   logging.** `tracing` más una capa de serialización JSON arrastraría un serializador y alrededor
   de una docena de crates para emitir, como mucho, un puñado de campos por evento — el mismo
   argumento que este árbol ya aplicó contra `axum`, `tiny-http` y los pools de conexión externos.
   El módulo completo (`crates/hexcell/src/registro.rs`) son unas pocas decenas de líneas.
2. **El conjunto de campos tipado es el mecanismo de privacidad, no una convención.**
   `EntradaDeRegistro::evento` es un `&'static str`: un valor construido en tiempo de ejecución —una
   cadena que viniera de un mensaje entrante— no se puede convertir en uno, así que ese campo no
   puede llevar nunca el texto de un mensaje aunque alguien lo intente por descuido. El resto de
   campos son identificadores opacos (`id_evento`, `id_conversacion`) y una medida de latencia
   (`latencia_ms`), salvo `detalle`, el único campo de texto libre, reservado al propio texto del
   proceso — una dirección vinculada, un error de almacenamiento — y nunca al texto de un mensaje.
3. **`registro::formatear` está separado de `registro::emitir`.** `formatear` es una función pura
   que devuelve el `String` ya serializado, incluido el escapado JSON de comillas, barras
   invertidas y caracteres de control, así que el formato se comprueba con un test normal sin
   capturar la salida de ningún proceso; `emitir` toma `stdout().lock()` una sola vez y escribe la
   línea ya formada.
4. **`id_celula` se fija una única vez, en un `std::sync::OnceLock`, por `registro::inicializar`**,
   llamado desde `main` justo tras analizar la configuración. No se pasa como parámetro a cada
   llamada del motor: `Motor::nuevo` mantiene sus cinco parámetros, y toda línea posterior a la
   inicialización lleva ya el identificador de célula estampado.
5. **Ningún módulo que pueda ver el texto de un mensaje importa `crate::registro`.** El motor
   (`crates/hexcell/src/motor.rs`) es el único punto de este binario que emite líneas de registro;
   `inferencia.rs`, `procesador.rs`, `conversaciones.rs` y `deduplicacion.rs` no importan el
   módulo. Esta prohibición es la mitad estructural de la garantía de que el contenido de un
   mensaje jamás llega a un log, verificada por una comprobación léxica del contrato de esta tarea
   y por un test de proceso real que inyecta un marcador distintivo y comprueba su ausencia de
   toda la salida capturada.

## Consecuencias

### Positivas

* La observabilidad mínima de una célula (identificador de célula, de evento, de conversación y
  latencia) queda disponible sin ninguna dependencia nueva de logging.
* El formato es comprobable sin capturar un proceso: `formatear` es una función pura con sus
  propios tests unitarios, incluida la corrección del escapado JSON.
* La ausencia de contenido de mensaje en los logs es una propiedad estructural del tipo
  (`evento: &'static str`, un único campo `detalle` documentado) y no solo una convención de uso.

### Negativas

* No hay niveles configurables en tiempo de ejecución, ni rotación de archivo, ni envío a un
  colector externo: es un registro de línea de comandos, pensado para la CLI de administración de
  la etapa A-6, no para una pila de observabilidad completa.
* Un desarrollador que añada un campo nuevo a `EntradaDeRegistro` sin revisar esta decisión podría
  reintroducir un campo de texto libre adicional; la defensa contra eso es la revisión de código y
  el propio conteo de campos de este ADR, no un mecanismo del compilador.

## Alternativas consideradas y descartadas

### A. `tracing` + `tracing-subscriber` con una capa JSON (D-17)

Se descartó por presupuesto: arrastra `serde`, un serializador JSON y alrededor de una docena de
crates transitivos para emitir, como mucho, un puñado de campos por evento en una célula
presupuestada en 80 MB. Registrado como D-17 en `docs/bitacora-de-descartes.md`.

### B. Un `HashMap<String, String>` de campos libres en vez de un tipo cerrado

Se descartó porque un mapa de clave-valor sin cerrar admite cualquier clave, incluida una que
alguien llame `mensaje` o `contenido` sin que nada lo impida en tiempo de compilación. El tipo
cerrado de `EntradaDeRegistro`, con un único campo de texto libre y documentado, hace la garantía
verificable por su propia forma.

## Referencias

* `crates/hexcell/src/registro.rs`: `EntradaDeRegistro`, `formatear`, `emitir`, `inicializar`.
* `crates/hexcell/src/motor.rs`: único punto de emisión de líneas de registro de este binario.
* `docs/bitacora-de-descartes.md`, D-17: rechazo de `tracing` más una capa JSON.
* `docs/STATUS.md`: entrada Definido de esta decisión, fechada 2026-07-30.
