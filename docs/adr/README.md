# Architecture Decision Records (ADR)

Decisiones de arquitectura del proyecto, una por archivo (`NNNN-titulo.md`).

Candidatas iniciales, ya tomadas en el [PRD](../PRD.md) y pendientes de formalizar:
1. Persistencia dual SQLite (`sessions.db` + `knowledge_live.db`).
2. Shadow DB con conmutación atómica por épocas (`ArcSwap` + symlink).
3. Control de admisión GCRA con Fast-Reject HTTP 200 hacia Meta.
4. Inferencia LLM 100% externa (Gemini/Groq/OpenRouter); el hardware local no ejecuta modelos.
