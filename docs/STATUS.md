# Estado del Proyecto

> Registro vivo del avance. Última actualización: 2026-07-26.

## Fase actual
**Fase A (MVP de validación) — diseño.** No existe todavía código fuente ni scaffold Rust.

El proyecto adopta una estrategia de dos fases: primero se valida el negocio con dos células piloto
sobre canal no oficial (whatsmeow), y solo después —cuando aparezca el tercer cliente— se construye
la infraestructura del canal oficial. Ver [plan/README.md](plan/README.md).

## Definido
* **Estrategia de canal en dos fases con compuerta en el tercer cliente.** Fase A: whatsmeow sobre
  websocket saliente, dos células piloto. Fase B: Meta Cloud API con webhooks, congelada hasta la
  compuerta. Ver [PRD.md](PRD.md), sección "Estrategia de Canal por Fases".
* **whatsmeow como adaptador no oficial de la Fase A.** Riesgos asumidos y documentados: ban del
  número (mitigado con números nuevos y dedicados), roturas de protocolo dependientes de
  mantenedores voluntarios, y violación temporal de los ToS de WhatsApp como riesgo de validación.
* **Puerto de canal (`ChannelAdapter`, FR-12)** como frontera de migración entre fases: el salto de
  canal debe ser un cambio de adaptador, no una reescritura.
* **Arquitectura de célula en la Fase A:** dos contenedores (núcleo Rust + sidecar Go de whatsmeow)
  compartiendo red local y volumen, comunicados por IPC sobre socket local.
* **Docker desde el día 1**, también en la fase de validación.
* **Nomenclatura:** la unidad desplegable por cliente se llama **célula**; en CLI e identificadores de
  código, `cell` (`hexcell-admin cell pause`, `--id <cell_id>`, binario `hexcell`).
* **Células piloto:** `piloto-01` (negocio de prueba del propio dueño) y `piloto-02` (un conocido),
  cada una con un número de WhatsApp nuevo y dedicado.
* **Respaldos adelantados a la etapa A-2**, en lugar de esperar al endurecimiento final: con pilotos
  reales no pueden esperar. Cubren **las tres bases** —`sessions.db`, `knowledge_live.db` y el
  `sqlstore` del sidecar—, este último copiado por el propio sidecar vía `VACUUM INTO` sobre orden
  IPC y con frecuencia alta (cada pocas horas), porque las credenciales del protocolo Signal
  evolucionan. **La restauración solo se da por buena si el bot reconecta y responde**; recuperar
  ficheros con la sesión muerta cuenta como fallo.
* **Re-emparejamiento por `PairPhone()` como procedimiento de recuperación de primera clase**
  (segunda capa, etapa A-3): código de ocho caracteres que el piloto teclea en su propio teléfono,
  sin necesidad de tenerlo en mano. Se ensaya con piloto-01 antes del alta de piloto-02.
* **Puerto de canal abstraído hacia el caso más restrictivo** (FR-12): envío tipado
  (`RespuestaLibre` | `Plantilla`), resultado tipado (`FueraDeVentana`, `PlantillaRequerida`,
  `LimiteDeTasa`, `DestinatarioInvalido`) y estado de la ventana de servicio de 24 h. El adaptador
  simulado de la etapa A-2 imita la semántica de la Cloud API, no la de whatsmeow, y los tests de
  contrato corren contra ese caso difícil.
* **Outbox durable en el sidecar** (etapa A-3): todo evento entrante se persiste con `fsync` como
  primera acción, antes de entregarlo al núcleo; entrega *at-least-once* con confirmación explícita y
  deduplicación en el núcleo. Limitación documentada: el acuse de protocolo hacia WhatsApp es
  automático y no se puede diferir, de modo que queda una ventana de pérdida de microsegundos.
* **Alertas push y dead-man's switch adelantados a la etapa A-6**: bot de Telegram ante sesión
  desvinculada, sidecar sin reconectar más de 5 minutos, bucle de reinicios, saldo agotado,
  descartes GCRA anómalos y descarte de envíos no solicitados (invariante anti-ban); más
  healthchecks.io con ping cada 5 minutos para que la caída total del servidor se notifique desde
  fuera.
* **Endurecimiento contra el patrón "compila ≠ correcto"** (2026-07-27), aplicado transversalmente:
  validación semántica del puerto en A-1 (`match` exhaustivo y cotejo contra la documentación
  oficial de la Cloud API), `hexcell-meta` vacío hasta resolver el ADR-0013, CI de A-1 con alcance
  declarado, `/health/ready` condicionado a sesión de canal activa (A-2/A-3/A-6, README y PRD
  alineados), ventana de deduplicación dimensionada frente al horizonte de reentrega (A-2),
  invariante continuo anti-envíos-no-solicitados con alerta (A-3/A-6), criterio de no-falso-positivo
  en GCRA (A-4), reversión de épocas con la misma validación semántica que la promoción (A-5), y
  eliminación de la vía de escape del criterio del núcleo intacto en B-1 (ahora bloquea la
  aceptación y exige revisar el ADR-0010). Descongela deliberadamente un mínimo de la observabilidad de la
  etapa B-3, porque hay usuarios reales desde la Fase A.
* **Compuerta pre-registrada y roles asimétricos de los pilotos** (etapa A-7): los umbrales numéricos
  y los **criterios de fracaso** se fijan por escrito antes del primer alta. **piloto-01 es banco de
  pruebas técnico y sus datos no cuentan para la validación de negocio** (el dueño no puede ser su
  propio cliente); **piloto-02 paga un importe simbólico pero real desde el segundo mes**, porque el
  acto de pagar es la métrica y "sí pagaría" no es evidencia.
* La pila tecnológica: Rust (backend nativo), Docker (aislamiento por célula), SQLite dual
  (persistencia); Caddy (proxy inverso + SSL) solo en la Fase B.
* El modelo de despliegue por contenedores aislados (imágenes Alpine/Scratch), con presupuesto de
  memoria por fase: ≤ 80 MB por célula en la Fase A (núcleo + sidecar) y < 50 MB en la Fase B.
* La viabilidad técnica del hardware (Intel i7 de 10 años, 8 GB RAM, SSD).
* Requisitos funcionales y no funcionales: ver [PRD.md](PRD.md).
* **FR-01 reconstruido y aprobado**, redactado por fases. Ya no hay marcador de TODO en el PRD.
* **Plan de implementación en dos fases (7 etapas de Fase A + 3 de Fase B): ver
  [plan/README.md](plan/README.md).** Cubre FR-01..FR-12 y NFR-01..NFR-05, y sitúa los pendientes de
  producto de más abajo como bloqueos declarados en las etapas que los necesitan.

## Pendiente
* **ADR de entrada pública de la Fase B: Cloudflare Tunnel (capa gratuita) frente a VPS ~3 USD/mes +
  WireGuard.** Condiciona la vigencia de FR-04 y NFR-04. — *Primera tarea de la etapa B-1; determina
  la mitad del alcance de la etapa B-2.*
* **Fijar los valores numéricos de la compuerta pre-registrada**: umbrales de éxito (conversaciones
  semanales sostenidas, porcentaje de resolución sin intervención, retención de clientes finales,
  coste máximo por conversación, disponibilidad mínima), **importe del cobro simbólico a piloto-02**
  y techos de los criterios de fracaso. El plan fija la estructura; los números son decisión de
  negocio. — *Tarea 1 de la etapa A-7, bloqueante y anterior a cualquier alta de piloto.*
* **Definir la política del núcleo ante `FueraDeVentana`**: encolar hasta que el cliente vuelva a
  escribir o escalar a un humano. — *Etapa A-2, tarea 6. No se dispara en la Fase A, pero se decide
  con calma antes de que importe.*
* Lógica de negocio específica. — *Bloquea el alcance funcional de la etapa A-2 y se descubre en la
  etapa A-7 con los pilotos reales.*
* Flujos de usuario finales. — *Bloquean la superficie de carga de catálogo de la etapa A-5 y el alta
  comercial automatizada de la etapa B-2.*
* Manejo de excepciones comerciales. — *Condiciona el modo degradado (etapa A-4) y las alertas
  (etapa B-3).*
* Modelo de monetización. — *Bloquea la calibración de saldos (etapa A-4) y la suspensión por impago
  (etapa B-2). La etapa A-7 le aporta su primera entrada empírica.*
* Proceso exacto de alta (onboarding) comercial de una nueva microempresa. — *Bloquea la etapa B-2.
  El alta operada manualmente de los dos pilotos se resuelve en la etapa A-7.*
* Decidir licencia (`LICENSE`). — *Etapa A-1, tarea 3. El repositorio git ya está inicializado.*
* Scaffold del workspace Rust (`Cargo.toml`, `src/`) y del módulo Go del sidecar cuando arranque la
  fase de implementación. — *Etapa A-1, tareas 5 y 6.*
