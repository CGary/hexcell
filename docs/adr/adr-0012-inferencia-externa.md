# ADR-0012 — Inferencia LLM 100 % externa sobre proveedores compatibles con OpenAI

* **Estado:** Vigente (2026-08-26).
* **Supersede a:** nada.
* **Etapa:** A-4 (HEX-044).
* **Requisitos tocados:** FR-10 (integración de inferencia), NFR-01 (presupuesto de recursos), adr-0002, adr-0017.

---

## Contexto

El PRD y las fundaciones del proyecto fijan que la inferencia LLM es 100 % externa: el hardware local de las células no ejecuta modelos locales por restricciones de memoria y procesamiento (NFR-01). En la etapa A-2 (`adr-0017`), se declaró el puerto `ProveedorDeInferencia` en `hexcell-core` y una implementación simulada para pruebas deterministas. La etapa A-4 requiere conectar la célula a proveedores comerciales reales (OpenRouter para MVP, Google AI Studio, DeepSeek V4-Flash) mediante una implementación HTTPS saliente real.

## Decisión

1. **La inferencia LLM es 100 % externa.** La célula nunca ejecuta modelos locales. Todo tráfico de inferencia sale hacia un extremo HTTPS externo.
2. **Un único cliente agnóstico basado en el formato OpenAI chat-completions.** La selección del proveedor (base URL, clave de API, modelo, tiempo de espera y reintentos) se gobierna 100 % por variables de entorno (`HEXCELL_INFERENCIA_*`). No existe código o ramificación específica por proveedor (OpenRouter, AI Studio, DeepSeek): cambiar de proveedor o modelo es un ajuste de configuración en tiempo de ejecución, no un cambio de código.
3. **Pila cliente HTTP seleccionada: hyper + hyper-rustls + rustls/ring.** Se elige `hyper 1.11` con `hyper-util` legacy client, `hyper-rustls 0.27` y `rustls 0.23` usando la biblioteca criptográfica `ring`. Se descarta `reqwest` por añadir aproximadamente 85 crates extra a la compilación, siguiendo el mismo argumento de frugalidad usado contra `axum`. Se elige `ring` sobre el proveedor por defecto de rustls 0.23 (`aws-lc-rs`) porque `ring` solo exige un compilador de C (ya presente por `libsqlite3-sys`), mientras que `aws-lc-rs` exige `cmake`.
4. **Regla estricta contra el reintento de HTTP 429 y 4xx.** Un error HTTP 429 indica agotamiento de cuota por el proveedor. Reintentar un 429 empeora el bloqueo y agota el tiempo de drenaje; por lo tanto, los errores 429 (y cualquier 4xx) fallan de inmediato en el primer intento y devuelven `Err` al procesador para que ejecute la liberación del presupuesto reservado. Únicamente se reintentan fallos de transporte, tiempos de espera (timeout) y errores de servidor (5xx), con una cota fija de reintentos (`HEXCELL_INFERENCIA_REINTENTOS`, por defecto 1, máximo 3) y una pausa fija de 250 ms.
5. **Aislamiento en el crate `hexcell`.** El adaptador HTTPS vive como un módulo (`crates/hexcell/src/proveedor_openai.rs`) dentro del crate del binario. `hexcell-core` se mantiene 100 % `std`-only y libre de dependencias de red (`adr-0002`). La selección entre el proveedor simulado y el real se realiza mediante el enum `ProveedorDeCelula` en `crates/hexcell/src/inferencia.rs`, sin usar `Box<dyn ProveedorDeInferencia>` ya que el puerto no es compatible con objetos de trait.
6. **Protección estricta de credenciales y datos de uso exactos.** La clave de API jamás se imprime en `Debug`, `Display`, registros, trazas ni mensajes de error. La respuesta extrae `unidades_consumidas` exactamente como `usage.prompt_tokens + usage.completion_tokens`, fallando cerrado (`Err`) si los metadatos de tokens están ausentes o malformados, sin inventar ni estimar unidades.

## Consecuencias

### Positivas

* Mantiene la célula liviana sin dependencias pesadas de HTTP o TLS nativo del sistema operativo (`openssl`/`native-tls`).
* Cambiar de proveedor o modelo en producción no requiere recompilar ni actualizar el binario de la célula.
* Garantiza que la contabilidad financiera en dos fases libere el saldo retenido ante cualquier fallo del proveedor sin reintentos infinitos.

### Negativas

* Requiere configuración explícita mediante variables de entorno para habilitar el proveedor real.
* `hyper-rustls` con `ring` exige recompilar la pila TLS estáticamente en Rust.

## Referencias

* `crates/hexcell/src/proveedor_openai.rs`: cliente HTTPS OpenAI-compatible.
* `crates/hexcell/src/configuracion.rs`: validación de variables `HEXCELL_INFERENCIA_*`.
* `docs/adr/README.md`: registro oficial de ADRs.
* `docs/bitacora-de-descartes.md`: D-27 (alternativas descartadas de inferencia HTTPS).
