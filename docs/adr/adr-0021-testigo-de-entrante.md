# ADR-0021: Testigo de entrante (TestigoDeEntrante) y variantes non_exhaustive de MensajeSaliente

## Estado
**Vigente** (2026-08-09)

## Contexto
HEX-016, etapa A-3 ítem 11. El invariante de solo-respuesta necesita una prueba en el sistema de tipos para garantizar que no se puedan enviar mensajes proactivos.

## Decisión
* **`TestigoDeEntrante` como *Value Object*** con campo privado, solo construible desde un `EventoEntrante`.
* **`MensajeSaliente` con variantes struct `#[non_exhaustive]`** (no tuple, verificado en rustc 1.92.0).
* **Constructores con testigo** que validan la conversación (el testigo se exige para instanciar el mensaje de salida).
* **`compile_fail` doctest emparejado con doctest positivo** (dado que E0639 no se refuerza en stable).
* **Contador de rechazos** usando `AtomicU64` con ordenamiento `Relaxed`.
* **`SalienteHistorico` en `hexcell-storage`** para replay sin necesidad de testigo.
* **Centinela Go basado en AST** para asegurar la ausencia de ruta de envío en el sidecar.

## Consecuencias
* Cada `match arm` externo necesita el patrón `{ field, .. }`.
* `bateria.rs` fabrica su propio `EventoEntrante`.
