# Architecture Decision Records (ADR)

Decisiones de arquitectura del proyecto, una por archivo, con el nombre `adr-NNNN-titulo.md`.

La numeración de esta tabla es la **fuente de verdad**: cada etapa del
[plan de implementación](../plan/README.md) referencia sus ADR por estos mismos números. Los números
se asignan de forma correlativa y no se reutilizan ni se reordenan, aunque el orden en que se
escriban los registros no coincida con el orden numérico.

| Archivo | Decisión | Etapa que lo produce | Estado |
| :--- | :--- | :--- | :--- |
| `adr-0001-licencia.md` | **Licencia del proyecto: AGPL-3.0**, con dual licensing conservado por el titular del copyright, frente a Apache-2.0 y BUSL-1.1. | A-1 | **Vigente** (2026-07-29) |
| `adr-0002-estructura-workspace.md` | **División en crates del workspace Rust y sus fronteras.** Cinco crates: `hexcell-core` (dominio y puerto de canal, **sin dependencias**, comprobable con una orden), `hexcell` (binario de la célula), `hexcell-admin` (CLI central), `hexcell-storage` (persistencia) y `hexcell-meta` (canal oficial, **vacío** hasta que se resuelva `adr-0013`). Incluye la consecuencia de declarar los métodos del puerto devolviendo `impl Future`: el trait no es compatible con objetos de trait. | A-1 | **Vigente** (2026-07-29) |
| `adr-0003-persistencia-dual.md` | Persistencia dual SQLite (`sessions.db` + `knowledge_live.db`) y parámetros de SQLite elegidos. | A-2 | Tomada en el PRD, por formalizar |
| `adr-0004-gcra-y-parametros.md` | Control de admisión GCRA sobre el flujo normalizado del puerto de canal, con Fast-Reject HTTP 200 hacia Meta únicamente en la Fase B. | A-4 | Tomada en el PRD, por formalizar |
| `adr-0005-contabilidad-dos-fases.md` | Contabilidad financiera de reserva previa y conciliación posterior. | A-4 | Tomada en el PRD, por formalizar |
| `adr-0006-epocas-y-conmutacion-atomica.md` | Shadow DB con conmutación atómica por épocas (`ArcSwap` + symlink). | A-5 | Tomada en el PRD, por formalizar |
| `adr-0007-imagen-y-aislamiento.md` | Imágenes base, composición de dos contenedores por célula, permisos de volumen y límites de recursos. | A-6 | Por escribir |
| `adr-0008-estrategia-canal-dos-fases.md` | **Estrategia de canal en dos fases con compuerta en el tercer cliente.** La Fase A valida el negocio sobre canal no oficial con dos células piloto; la Fase B, comercial, adopta la Meta Cloud API. El tercer cliente no se suma a la Fase A: la cierra. | A-1 | **Derogada** — *superseded* por `adr-0014` (2026-07-28) |
| `adr-0009-whatsmeow-adaptador-fase-a.md` | **whatsmeow como adaptador no oficial de la Fase A**, elegido sobre [Baileys](https://github.com/WhiskeySockets/Baileys/issues/2488) por su binario Go liviano —adecuado al presupuesto de memoria del hardware objetivo— y por su recuperación rápida ante roturas de protocolo, con el precedente de [abril de 2026](https://github.com/lharries/whatsapp-mcp/issues/216) resuelto en días mediante un *bump* de versión. | A-1 | **Vigente** (2026-07-29) |
| `adr-0010-puerto-de-canal.md` | **Puerto de canal `ChannelAdapter` como frontera entre el núcleo y el transporte.** El núcleo no conoce ningún transporte: cada canal es un adaptador más, y los dos pueden estar vivos a la vez sin tocar el dominio. Incluye la regla de que `sessions.db` nunca almacena identificadores de transporte crudos; que el **mapeo de identidad pertenece al adaptador** y el núcleo trata el identificador interno como opaco; y que ese mapeo persiste en un **almacén propio del adaptador, separado del `sqlstore`** —para sobrevivir al re-emparejamiento que sigue a `device_removed`— que pasa a ser la **cuarta base del respaldo**. | A-1 | **Vigente** (2026-07-28) |
| `adr-0011-whatsmeow-sidecar-e-ipc.md` | Arquitectura de sidecar que impone la elección de `adr-0009`: proceso Go separado, mecanismo IPC con el núcleo, persistencia de sesión y política anti-ban no desactivable por configuración. | A-3 | Por escribir |
| `adr-0012-inferencia-externa.md` | Inferencia LLM 100 % externa (Gemini/Groq/OpenRouter); el hardware local no ejecuta modelos. | A-4 | Tomada en el PRD, por formalizar |
| `adr-0013-entrada-publica-fase-b.md` | **Entrada pública de la Fase B: Cloudflare Tunnel (capa gratuita) frente a VPS ~3 USD/mes + WireGuard.** La primera opción termina el TLS en el edge y elimina el handshake anti-Hairpin (FR-04) y el On-Demand TLS de Caddy (NFR-04); la segunda lo termina en el propio Caddy y conserva la arquitectura original a cambio de un coste fijo mensual. | B-1 | **PENDIENTE** — primera tarea de la etapa B-1; condiciona la mitad del alcance de la etapa B-2 |
| `adr-0014-canal-propio-permanente.md` | **Canal propio permanente y canal oficial pospuesto a segunda etapa.** *Supersede a `adr-0008`.* whatsmeow pasa a ser el canal de producción por defecto, permanente y con clientes de pago; la Meta Cloud API se pospone a una segunda etapa como canal adicional que convive, activada por demanda de un cliente que la justifique. Deroga la regla "no se comercializa sobre canal no oficial" y la compuerta del tercer cliente, sustituida por techo duro de cartera y umbral de incidentes. | A-1 | **Vigente** (2026-07-28) |
| `adr-0015-politica-de-convivencia-con-el-baneo.md` | **Política de convivencia con el riesgo de baneo del canal propio.** Cuatro capas de defensa —reducir la probabilidad, detectar pronto, contener el daño, recuperar— con el baneo tratado como evento esperado y no como fallo, la marca obligatoria [causa documentada] / [precautorio], y la lista de lo que no debe hacerse. | A-3 (transversal A-2, A-6 y A-7) | **Vigente** (2026-07-28) |

Estos ADR registran lo que se **decidió**. Las alternativas evaluadas y no elegidas, las decisiones
derogadas y los supuestos que se demostraron falsos se recogen además en
[../bitacora-de-descartes.md](../bitacora-de-descartes.md), con su motivo y —lo que un ADR no
declara— **qué tendría que cambiar para reabrirlas**. Al escribir un ADR nuevo, anota allí las
alternativas que descarte.

Los ADR restantes del canal oficial —adaptador de Cloud API, plano de control y handshake sintético—
recibirán su número correlativo cuando la segunda etapa se active por demanda de un cliente que la
justifique (`adr-0014`). No se les asigna todavía porque su existencia y su alcance dependen de la
decisión de `adr-0013`.
