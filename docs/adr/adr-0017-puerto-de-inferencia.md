# ADR-0017 — Puerto de inferencia LLM `ProveedorDeInferencia`

* **Estado:** Vigente desde el 2026-07-30.
* **Supersede a:** nada.
* **Etapa:** A-2 (HEX-007).
* **Requisitos tocados:** FR-12 (patrón de puerto), preparación de FR-10 y de la etapa A-4.

---

## Contexto

Hasta esta tarea, el motor de mensajería tenía la respuesta cableada en `ProcesadorDeEco`: repetía
el contenido del evento entrante como respuesta libre, sin consultar nada externo. La etapa A-4
sustituirá eso por un proveedor LLM real (Gemini, Groq y el resto de proveedores externos que fije
esa etapa; `adr-0012` ya fijó que la inferencia es 100 % externa y que el hardware local no ejecuta
modelos), envuelto además en la contabilidad financiera de dos fases (reserva previa y conciliación
exacta). Esta tarea no implementa ni la inferencia real ni esa contabilidad: fija el contrato que
la etapa A-4 pueda envolver sin cambiar lo que el motor consume, y una implementación simulada
determinista para tests.

## Decisión

1. **El trait `ProveedorDeInferencia` vive en `crates/hexcell-core/src/inferencia.rs`**, junto a
   `ChannelAdapter`. Igual que el núcleo no conoce whatsmeow ni la Cloud API, tampoco conoce ningún
   proveedor LLM concreto: sumar un proveedor real es escribir un adaptador de este trait, no
   reescribir el motor. El trait usa únicamente tipos de la biblioteca estándar y del propio
   `hexcell-core` (`IdConversacion`, `String`, `std::future::Future`), así que la tabla de
   dependencias de `hexcell-core` sigue vacía — criterio de aceptación comprobable con
   `cargo tree -p hexcell-core`.
2. **El único método, `generar`, se declara `-> impl Future<Output = Result<RespuestaDeInferencia,
   Self::Error>> + Send`, no `async fn`.** Sobre rustc 1.92.0, `async fn` dentro de un trait dispara
   el aviso `async_fn_in_trait`, activo por omisión, que `cargo clippy --workspace -- -D warnings`
   convierte en error — el mismo razonamiento que `adr-0002` ya dejó escrito para
   `ChannelAdapter::send`. La consecuencia es la misma: el trait no es compatible con objetos de
   trait, así que el motor lo consume genérico, nunca como `Box<dyn ProveedorDeInferencia>`.
3. **`PeticionDeInferencia` y `RespuestaDeInferencia` llevan solo lo mínimo que el motor consume
   hoy**: la conversación y el contenido normalizado de entrada, y el texto de la respuesta. Sin
   recuento de tokens, sin coste, sin nombre de modelo, sin variante de streaming. Escribir esa
   firma por adelantado sería exactamente D-09 (`docs/bitacora-de-descartes.md`): una firma que
   compila no garantiza que la etapa A-4 pueda envolver el proveedor sin tocar lo que el motor
   consume — para eso basta con que `generar` conserve su firma, y añadir un campo de uso a
   `RespuestaDeInferencia` cuando exista una respuesta real que modelar no cambia esa firma.
4. **El proveedor simulado (`ProveedorSimulado`) es un módulo de `crates/hexcell`, no un octavo
   crate del workspace.** `hexcell-canal-simulado` ganó su propio crate porque
   `hexcell-canal-contrato` lo consume independientemente del binario; nada fuera de
   `crates/hexcell` consume el proveedor simulado, así que promoverlo a crate el día que haga falta
   es mecánico. Su respuesta es una huella FNV-1a de 64 bits del contenido de entrada, calculada a
   mano en unas pocas líneas sin ninguna dependencia (sin `rand`, sin leer ningún reloj, sin el
   hasher por defecto de la biblioteca estándar, cuya salida no es estable entre procesos), y
   **deliberadamente no es el eco** de la entrada: un eco no se distingue de un valor fijo escrito
   a mano en el procesador, y el motor necesita poder demostrar que la respuesta enviada vino del
   proveedor.
5. **El motor consume el proveedor a través de `ProcesadorDeInferencia<I>`**, un nuevo tipo de
   `crates/hexcell/src/procesador.rs` genérico sobre `I: ProveedorDeInferencia`, y no a través de un
   sexto parámetro de `Motor::nuevo`. `ProcesadorDeEco` se conserva íntegro: cinco archivos de test
   ya existentes lo usan para ejercitar deduplicación, historial, reinicio y la política ante
   `FueraDeVentana`, y no deben convertirse en tests del proveedor de inferencia.
6. **Un fallo del proveedor produce `None` y ninguna respuesta**, nunca un texto fijo de disculpa
   ni un reintento. Qué contesta una célula cuando la inferencia falla es una decisión de producto
   ligada al modo degradado de la etapa A-4 (FR-10), y `docs/STATUS.md` la registra como bloqueo
   declarado, no algo que se resuelve de pasada aquí.

## Consecuencias

### Positivas

* La etapa A-4 puede sustituir `ProveedorSimulado` por un proveedor real sin tocar
  `crates/hexcell/src/motor.rs` ni la firma de `ProcesadorDeMensajes`: solo escribe un nuevo tipo
  que implemente `ProveedorDeInferencia`.
* `hexcell-core` sigue sin ninguna dependencia de infraestructura, verificable con una orden.
* El determinismo del proveedor simulado hace que los tests de esta tarea (incluidos los de
  proceso real) no dependan de ninguna llamada de red ni de temporización arbitraria.

### Negativas

* `ProveedorDeInferencia`, igual que `ChannelAdapter`, no es compatible con objetos de trait: la
  selección de proveedor en `main.rs` es estática, no dinámica en tiempo de ejecución. Se acepta
  por la misma razón que ya se aceptó para el canal: el número de proveedores es pequeño y
  conocido en tiempo de compilación.
* El campo `detalle` de un fallo del proveedor no llega hoy a ningún sitio observable más allá del
  registro estructurado del motor (`adr-0019`): no hay todavía ninguna vía de notificación a un
  operador humano.

## Alternativas consideradas y descartadas

### A. Adelantar la firma con recuento de tokens y coste

Se descartó por ser exactamente D-09: una firma anticipada como mitigación de compatibilidad no
sustituye a la garantía real, que son los tests de contrato contra el proveedor simulado. La etapa
A-4 añadirá esos campos cuando tenga una respuesta real de la que modelarlos, sin que eso cambie la
firma que el motor consume.

### B. Un sexto parámetro de `Motor::nuevo` para el proveedor de inferencia

Se descartó en favor de `ProcesadorDeInferencia<I>`: el `constraint` del spec permite
explícitamente que el proveedor lo consuma «el procesador que el motor invoca», y mantener
`Motor::nuevo` en cinco parámetros evita ensanchar una firma que ya tiene un adaptador, un
procesador, un receptor, una ventana de deduplicación y un repositorio.

## Referencias

* `crates/hexcell-core/src/inferencia.rs`: declaración del trait y de los dos tipos de datos.
* `crates/hexcell/src/inferencia.rs`: `ProveedorSimulado` y la huella FNV-1a.
* `crates/hexcell/src/procesador.rs`: `ProcesadorDeInferencia<I>`.
* `adr-0002-estructura-workspace.md`: razonamiento de `-> impl Future` para `ChannelAdapter`.
* `adr-0012-inferencia-externa.md`: la inferencia LLM es 100 % externa.
* `docs/bitacora-de-descartes.md`, D-09: firma anticipada como mitigación de compatibilidad.
* `docs/STATUS.md`: entrada Pendiente sobre el modo degradado ante fallo de inferencia (FR-10).
