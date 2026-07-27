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

## Arquitectura (lo esencial para no romper el diseño)

* **Estrategia de dos fases con compuerta.** Fase A: MVP de validación con **whatsmeow** (sidecar Go, websocket saliente, sin webhook/Caddy/TLS entrante), cerrada a dos células piloto (`piloto-01`, `piloto-02`). El **tercer cliente cierra la Fase A** y abre la Fase B (Meta Cloud API + webhooks). La Fase B está **congelada**: no se estima ni se implementa hasta la compuerta.
* **Puerto de canal (`ChannelAdapter`, FR-12)** — la frontera de migración. El núcleo Rust nunca conoce el transporte de WhatsApp; cambiar de fase = escribir otro adaptador, no reescribir el producto. El adaptador simulado de tests imita la semántica restrictiva de la Cloud API (ventana de 24 h, `FueraDeVentana`, `PlantillaRequerida`), no la de whatsmeow. `sessions.db` nunca almacena identificadores de transporte crudos.
* **Célula** (`cell` en CLI/código): unidad desplegable por cliente. Fase A = dos contenedores (núcleo Rust + sidecar Go) con red local y volumen compartidos, IPC por socket local. Presupuesto: ≤ 80 MB RAM por célula (Fase A), < 50 MB (Fase B).
* **Persistencia dual SQLite por célula**: `sessions.db` (lectura/escritura caliente) + `knowledge_live.db` (solo lectura en producción). Actualizaciones de conocimiento vía Shadow DB (`knowledge_staging.db`) → épocas inmutables (`knowledge_epoch_N.db`) con conmutación atómica (symlink + `ArcSwap` + Graceful Drain).
* **GCRA sobre el flujo normalizado del puerto** (no sobre HTTP) para admisión, y contabilidad financiera de LLM en dos fases (reserva previa + conciliación exacta). La inferencia LLM es 100 % externa (Gemini Flash/Groq/OpenRouter); el hardware local nunca ejecuta modelos.
* **Orden del plan**: nada se conecta a un canal real hasta que el consumidor sabe protegerse (admisión y presupuesto antes que pilotos); los respaldos van en A-2 y cubren **tres** bases (`sessions.db`, `knowledge_live.db` y el `sqlstore` del sidecar) — una restauración solo es válida si el bot reconecta y responde.

## Reglas prácticas

* Nunca versionar `*.db`, `*.db-wal`, `*.db-shm` ni `.env*` (ya en `.gitignore`).
* El plan no inventa requisitos: toda etapa nueva o cambio de alcance debe trazarse a FR/NFR del PRD o registrarse como decisión pendiente en STATUS.md.
* Decisiones de producto abiertas (monetización, flujos de usuario, excepciones comerciales, entrada pública de la Fase B — `adr-0013`) se tratan como bloqueos declarados, no se resuelven de pasada.
