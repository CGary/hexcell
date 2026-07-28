# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Qué es este repositorio

**HexCell Orchestrator**: orquestador multi-célula (multi-tenant) en Rust para desplegar bots de WhatsApp para microempresas sobre hardware local modesto (i7 de 10 años, 8 GB RAM).

**Estado actual: solo documentación.** No existe código fuente, `Cargo.toml`, ni comandos de build/test todavía. El scaffold del workspace Rust y del sidecar Go llegará en la etapa A-1 del plan.

Todo el contenido del repositorio está en **español**, incluidos los mensajes de commit (conventional commits: `docs:`, `feat:`, etc., sin atribución de AI).

## Jerarquía documental (rango normativo)

Ante contradicciones, manda el orden siguiente:

1. **`docs/PRD.md`** — fuente normativa: requisitos FR-01..FR-12, NFR-01..NFR-05 y criterios de QA.
2. **`README.md`** — detalle operativo y de arquitectura que el PRD no recoge (CLI, onboarding Fase B).
3. **`docs/plan/README.md`** — índice del plan de implementación; un archivo por etapa (`fase-a-N-*.md`, `fase-b-N-*.md`). Cada etapa declara qué FR/NFR cubre.
4. **`docs/STATUS.md`** — registro vivo del avance (Definido / Pendiente). **Actualizarlo cuando una decisión cambie de estado.**
5. **`docs/adr/README.md`** — tabla de ADRs; su numeración es fuente de verdad, correlativa, nunca se reutiliza ni reordena. Formato de archivo: `adr-NNNN-titulo.md`.
6. **`docs/bitacora-de-descartes.md`** — registro de lo que se estudió y **no** se hizo, con el motivo y las condiciones de reapertura. No es normativo: no decide nada, deja rastro. Numeración `D-NN`, correlativa, nunca reutilizada; las entradas no se editan ni se borran, se marcan `REABIERTO`.

## Arquitectura (lo esencial para no romper el diseño)

* **Dos canales que conviven, no dos fases en secuencia** (rumbo fijado el 28 de julio de 2026). Los nombres "Fase A" y "Fase B" y los archivos `fase-*.md` se conservan, pero su significado cambió:
  * **Fase A = canal propio en producción.** **whatsmeow** (sidecar Go, websocket saliente, sin webhook/Caddy/TLS entrante) es el canal **por defecto y permanente**, con clientes de pago reales. `piloto-01` y `piloto-02` son las dos primeras células, no el alcance total.
  * **Fase B = canal oficial adicional** (Meta Cloud API + webhooks) que **convive** con el propio. Sigue congelada, pero ahora se activa por **demanda de un cliente que la justifique**, no por número de clientes ni por fecha.
  * **La compuerta del tercer cliente está DEROGADA**, igual que la regla "no se comercializa sobre canal no oficial". **Nunca escribir que la Fase B sustituye, reemplaza o cierra la Fase A, ni que el sidecar se retira.** Lo que disciplina el crecimiento son las compuertas de riesgo (techo duro de cartera y umbral de incidentes que congela altas, etapa A-7); sus valores son decisiones de negocio pendientes.
* **Puerto de canal (`ChannelAdapter`, FR-12)** — la frontera de **coexistencia**: dos adaptadores vivos a la vez en células distintas. El núcleo Rust nunca conoce el transporte de WhatsApp; sumar un canal = escribir otro adaptador, no reescribir el producto. Se abstrae hacia el caso más restrictivo (Cloud API), con esta distinción: **el TIPO admite el resultado restrictivo; la POLÍTICA de cada adaptador decide si lo produce** — el adaptador del canal propio no impone ventana de 24 h artificial. El adaptador simulado de tests imita la semántica restrictiva de la Cloud API (ventana de 24 h, `FueraDeVentana`, `PlantillaRequerida`), no la de whatsmeow. `sessions.db` nunca almacena identificadores de transporte crudos.
* **Célula** (`cell` en CLI/código): unidad desplegable por cliente. Sobre canal propio = dos contenedores (núcleo Rust + sidecar Go) con red local y volumen compartidos, IPC por socket local, **con el sidecar como coste permanente**; sobre canal oficial = un contenedor. Presupuesto de línea base: ≤ 80 MB RAM por célula sobre canal propio, < 50 MB sobre canal oficial. **Ninguna de las dos cifras está validada bajo carga sostenida**, y el techo de células por servidor es desconocido hasta medirlo (probablemente lo limite la CPU y la E/S, no la memoria).
* **Persistencia dual SQLite por célula**: `sessions.db` (lectura/escritura caliente) + `knowledge_live.db` (solo lectura en producción). Actualizaciones de conocimiento vía Shadow DB (`knowledge_staging.db`) → épocas inmutables (`knowledge_epoch_N.db`) con conmutación atómica (symlink + `ArcSwap` + Graceful Drain).
* **GCRA sobre el flujo normalizado del puerto** (no sobre HTTP) para admisión, y contabilidad financiera de LLM en dos fases (reserva previa + conciliación exacta). La inferencia LLM es 100 % externa (Gemini Flash/Groq/OpenRouter); el hardware local nunca ejecuta modelos.
* **Orden del plan**: nada se conecta a un canal real hasta que el consumidor sabe protegerse (admisión y presupuesto antes que pilotos); los respaldos se diseñan en A-2 y cubren **cuatro** bases (`sessions.db`, `knowledge_live.db`, el almacén de identidad del adaptador y el `sqlstore` del sidecar) — una restauración solo es válida si el bot reconecta y responde, criterio que exige sidecar y canal real y por eso se ejecuta en A-3, no en A-2.

## Reglas prácticas

* Nunca versionar `*.db`, `*.db-wal`, `*.db-shm` ni `.env*` (ya en `.gitignore`).
* El plan no inventa requisitos: toda etapa nueva o cambio de alcance debe trazarse a FR/NFR del PRD o registrarse como decisión pendiente en STATUS.md.
* Decisiones de producto abiertas (monetización, flujos de usuario, excepciones comerciales, entrada pública de la Fase B — `adr-0013`, techo duro de cartera, umbral de incidentes) se tratan como bloqueos declarados, no se resuelven de pasada. No inventar números de clientes, de células ni de precios que la documentación no fije.
* **El riesgo de baneo del canal propio es estructural**, no conductual: Meta detecta la biblioteca por su huella de protocolo. Se documenta como evento esperado, no como fallo; las medidas de mayor valor son las que reducen el daño, no las que reducen la probabilidad. No introducir folclore de proveedores de envío masivo (jitter, protocolos de "calentamiento"), ni proxies, VPN o rotación de IP.
* Una decisión derogada se **supersede con un ADR nuevo**; nunca se reescribe el viejo ni se reordena la numeración. Las fechas se escriben en formato absoluto (28 de julio de 2026 / 2026-07-28), nunca relativas.
* **Antes de proponer un cambio de rumbo, un atajo o una técnica nueva, consultar `docs/bitacora-de-descartes.md`.** Si la idea ya está allí, no se vuelve a debatir desde cero: se lee su motivo y su condición de reapertura, y solo se reabre si esa condición se cumple. Todo descarte nuevo se anota en la bitácora **en el mismo commit en que se descarta**; un descarte sin motivo escrito es un descarte perdido.
