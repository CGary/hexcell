# Architecture Decision Records (ADR)

Decisiones de arquitectura del proyecto, una por archivo, con el nombre `adr-NNNN-titulo.md`.

La numeración de esta tabla es la **fuente de verdad**: cada etapa del
[plan de implementación](../plan/README.md) referencia sus ADR por estos mismos números. Los números
se asignan de forma correlativa y no se reutilizan ni se reordenan, aunque el orden en que se
escriban los registros no coincida con el orden numérico.

| Archivo | Decisión | Etapa que lo produce | Estado |
| :--- | :--- | :--- | :--- |
| `adr-0001-licencia.md` | Licencia del proyecto. | A-1 | Por escribir |
| `adr-0002-estructura-workspace.md` | División en crates del workspace Rust y sus fronteras. | A-1 | Por escribir |
| `adr-0003-persistencia-dual.md` | Persistencia dual SQLite (`sessions.db` + `knowledge_live.db`) y parámetros de SQLite elegidos. | A-2 | Tomada en el PRD, por formalizar |
| `adr-0004-gcra-y-parametros.md` | Control de admisión GCRA sobre el flujo normalizado del puerto de canal, con Fast-Reject HTTP 200 hacia Meta únicamente en la Fase B. | A-4 | Tomada en el PRD, por formalizar |
| `adr-0005-contabilidad-dos-fases.md` | Contabilidad financiera de reserva previa y conciliación posterior. | A-4 | Tomada en el PRD, por formalizar |
| `adr-0006-epocas-y-conmutacion-atomica.md` | Shadow DB con conmutación atómica por épocas (`ArcSwap` + symlink). | A-5 | Tomada en el PRD, por formalizar |
| `adr-0007-imagen-y-aislamiento.md` | Imágenes base, composición de dos contenedores por célula, permisos de volumen y límites de recursos. | A-6 | Por escribir |
| `adr-0008-estrategia-canal-dos-fases.md` | **Estrategia de canal en dos fases con compuerta en el tercer cliente.** La Fase A valida el negocio sobre canal no oficial con dos células piloto; la Fase B, comercial, adopta la Meta Cloud API. El tercer cliente no se suma a la Fase A: la cierra. | A-1 | Tomada, por formalizar |
| `adr-0009-whatsmeow-adaptador-fase-a.md` | **whatsmeow como adaptador no oficial de la Fase A**, elegido sobre [Baileys](https://github.com/WhiskeySockets/Baileys/issues/2488) por su binario Go liviano —adecuado al presupuesto de memoria del hardware objetivo— y por su recuperación rápida ante roturas de protocolo, con el precedente de [abril de 2026](https://github.com/lharries/whatsapp-mcp/issues/216) resuelto en días mediante un *bump* de versión. | A-1 | Tomada, por formalizar |
| `adr-0010-puerto-de-canal.md` | **Puerto de canal `ChannelAdapter` como frontera de migración.** El núcleo no conoce ningún transporte: el salto de fase es un cambio de adaptador, no una reescritura. Incluye la regla de que `sessions.db` nunca almacena identificadores de transporte crudos. | A-1 | Tomada, por formalizar |
| `adr-0011-whatsmeow-sidecar-e-ipc.md` | Arquitectura de sidecar que impone la elección de `adr-0009`: proceso Go separado, mecanismo IPC con el núcleo, persistencia de sesión y política anti-ban no desactivable por configuración. | A-3 | Por escribir |
| `adr-0012-inferencia-externa.md` | Inferencia LLM 100 % externa (Gemini/Groq/OpenRouter); el hardware local no ejecuta modelos. | A-4 | Tomada en el PRD, por formalizar |
| `adr-0013-entrada-publica-fase-b.md` | **Entrada pública de la Fase B: Cloudflare Tunnel (capa gratuita) frente a VPS ~3 USD/mes + WireGuard.** La primera opción termina el TLS en el edge y elimina el handshake anti-Hairpin (FR-04) y el On-Demand TLS de Caddy (NFR-04); la segunda lo termina en el propio Caddy y conserva la arquitectura original a cambio de un coste fijo mensual. | B-1 | **PENDIENTE** — primera tarea de la etapa B-1; condiciona la mitad del alcance de la etapa B-2 |

Los ADR restantes de la Fase B —adaptador de Cloud API, plano de control y handshake sintético—
recibirán su número correlativo cuando se abra la compuerta. No se les asigna todavía porque su
existencia y su alcance dependen de la decisión de `adr-0013`.
