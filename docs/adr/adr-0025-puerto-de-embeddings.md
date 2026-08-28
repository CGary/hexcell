# ADR-0025 — Puerto de incrustaciones vectoriales `ProveedorDeEmbeddings` y adaptador OpenRouter

* **Estado:** Vigente desde el 2026-08-27.
* **Supersede a:** nada.
* **Etapa:** A-5 (HEX-051-a, FR-06).
* **Requisitos tocados:** FR-06 (indexación de catálogo mediante embeddings en lotes), preparación de la base de conocimiento en Shadow DB.

---

## Contexto

La indexación de catálogo para la base de conocimiento de la célula (etapa A-5) requiere transformar fragmentos de texto en vectores numéricos de incrustación (*embeddings*) mediante llamadas a proveedores externos. Siguiendo los principios de arquitectura limpia y la división en crates de `adr-0002`, el núcleo de dominio no debe depender de bibliotecas de transporte HTTP ni de estructuras propietarias de proveedores comerciales. Además, el consumo financiero de estas llamadas debe quedar estrictamente auditado y gobernado por la contabilidad en dos fases de `adr-0005` y `hexcell-storage`, asegurando que ninguna llamada de red se ejecute sin saldo disponible ni deje reservas de presupuesto huérfanas.

## Decisión

1. **Declaración del puerto en `hexcell-core` sin dependencias externas (`adr-0002`).**
   El trait `ProveedorDeEmbeddings` se declara en `crates/hexcell-core/src/embeddings.rs`. Define la operación asíncrona `incrustar_lote`, que recibe `PeticionDeEmbeddings` y retorna un futuro con `RespuestaDeEmbeddings`. La tabla `[dependencies]` de `hexcell-core` permanece rigurosamente vacía.

2. **Retorno `-> impl Future` y despacho estático por enumeración.**
   Para evitar el aviso `async_fn_in_trait` en rustc 1.92.0 (convertido en error por `-D warnings`), el método del trait retorna `impl Future<Output = ...> + Send`. Al no ser compatible con objetos de trait (`dyn`), la selección de adaptadores en el binario se resuelve mediante la enumeración `ProveedorDeEmbeddingsDeCelula` (`Simulado` | `OpenRouter`). Esto permite que la futura variante de Google AI Studio / Gemini (HEX-051-b) se incorpore como una adición pura sin alterar el puerto ni reestructurar el enum.

3. **Correspondencia posicional estricta y colocación por índice explícito.**
   `RespuestaDeEmbeddings` contiene un vector `vectores: Vec<Option<VectorDeEmbedding>>` cuya longitud coincide exactamente con la cantidad de textos de la petición. El adaptador OpenRouter procesa el arreglo `data` asignando cada vector según su campo numérico `index`, nunca mediante emparejamiento posicional secuencial. Si el proveedor devuelve menos elementos o en orden arbitrario, los fragmentos no resueltos quedan en `None`. Cualquier índice duplicado, fuera de rango o con vector de longitud cero se rechaza como `RespuestaInvalida`.

4. **Tipos de serialización desacoplados del flujo de chat.**
   El adaptador OpenRouter define sus propias estructuras de serialización JSON en `crates/hexcell/src/proveedor_embeddings.rs`. A diferencia del endpoint de chat-completions (`proveedor_openai.rs`), que exige obligatoriamente `completion_tokens`, el endpoint `/embeddings` solo reporta `prompt_tokens`. Duplicar el conector y aislar los tipos evita relajar la validación estricta de chat, protegiendo al sistema contra subfacturación silenciosa.

5. **Suelo financiero ante ausencia de metadatos de uso.**
   Cuando la respuesta del proveedor omite el bloque `usage` o no incluye `prompt_tokens`, el servicio de aplicación `ServicioDeEmbeddings` concilia la reserva contra la estimación previa calculada por `estimar_coste_de_lote`, nunca contra cero. Esto evita que una llamada de red facturada externamente resulte gratuita en el saldo local.

6. **Granularidad de reserva por llamada y soporte de reanudación.**
   La contabilidad opera con una reserva por cada llamada individual al proveedor, invocando `reservar_presupuesto_de_ingesta` en `RepositorioDeSesiones`. El tipo `LoteDeEmbeddings` administra la reanudación estructurada: `peticion_pendiente` entrega únicamente los textos no resueltos con sus índices de origen, garantizando que los fragmentos completados no vuelvan a solicitarse ni a presupuestarse.

7. **Aritmética temporal y acotamiento del tamaño de lote.**
   Los valores por defecto (`timeout`: 8000 ms, `reintentos`: 1 con retroceso fijo de 250 ms) garantizan que el tiempo máximo por intento (`2 * 8000 + 250 = 16250 ms`) sea estrictamente menor que el límite de drenaje de apagado (`LIMITE_DE_DRENAJE_POR_DEFECTO = 20000 ms`). Ante fragmentos extensos, la palanca operativa es limitar el tamaño de lote (`HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE`, por defecto 32, máximo 128) y no aumentar el tiempo de espera, preservando la ventana de drenaje.

8. **Protección de credenciales en repositorios públicos.**
   La configuración se alimenta exclusivamente de variables de entorno (`HEXCELL_EMBEDDINGS_*`). `ConfiguracionDeEmbeddings` y `ProveedorDeEmbeddingsOpenRouter` implementan `fmt::Debug` manual redactando la clave de API como `«redactado»`.

## Consecuencias

### Positivas

* Frontera de dominio pura en `hexcell-core` sin dependencias de red ni tipos propietarios.
* Eliminación estructural de desalineaciones entre fragmentos y vectores mediante índices explícitos y acumuladores ordenados.
* Contabilidad financiera estricta y a prueba de fugas, con liberación garantizada ante errores y suelo de estimación previa ante respuestas sin metadatos.
* Compatibilidad directa con futuras implementaciones de adaptadores (Gemini) sin reapertura del contrato del puerto.

### Negativas

* `ProveedorDeEmbeddings` no es compatible con `dyn`, requiriendo mantenimiento del enum de despacho en el binario.
* Duplicación controlada de la construcción del conector HTTPS entre `proveedor_openai.rs` y `proveedor_embeddings.rs`.
* Las reservas de presupuesto que queden activas por una terminación abrupta del proceso (p. ej. `SIGKILL`) requieren un mecanismo de limpieza periódico en arranque, registrado como decisión pendiente.

## Alternativas consideradas y descartadas

Las alternativas descartadas durante el diseño de este puerto (compartir el deserializador de chat con una bandera de modo, unión posicional de vectores, granularidad de reserva por fragmento o por ingesta global, elevación del tiempo límite de drenaje, codificación base64 y asignación de identificadores ficticios de conversación) se encuentran detalladas en la [Bitácora de Descartes](../bitacora-de-descartes.md) bajo la entrada D-28.

## Referencias

* `crates/hexcell-core/src/embeddings.rs`: declaración del puerto y Value Objects.
* `crates/hexcell-core/src/presupuesto.rs`: función `estimar_coste_de_lote`.
* `crates/hexcell/src/proveedor_embeddings.rs`: adaptador OpenRouter HTTPS.
* `crates/hexcell/src/embeddings.rs`: selector `ProveedorDeEmbeddingsDeCelula` y `ServicioDeEmbeddings`.
* `crates/hexcell-storage/src/presupuesto.rs`: método `reservar_presupuesto_de_ingesta`.
* `docs/adr/adr-0002-estructura-workspace.md`: frontera de dependencias de `hexcell-core`.
* `docs/adr/adr-0005-contabilidad-dos-fases.md`: contabilidad financiera en dos fases.
* `docs/bitacora-de-descartes.md`: entrada D-28.
