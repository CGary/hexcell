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
  código, `cell` (`zeroclaw-admin cell pause`, `--id <cell_id>`, binario `zeroclaw-cell`).
* **Células piloto:** `piloto-01` (negocio de prueba del propio dueño) y `piloto-02` (un conocido),
  cada una con un número de WhatsApp nuevo y dedicado.
* **Respaldos adelantados a la etapa A-2** (`VACUUM INTO`, copia fuera del disco y restauración
  probada), en lugar de esperar al endurecimiento final: con pilotos reales no pueden esperar.
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
* **Definir las métricas de validación del negocio con los pilotos** (calidad de respuesta, uso real
  de los clientes finales, disposición a pagar, coste por conversación, estabilidad del canal). —
  *Tarea 1 de la etapa A-7, anterior a cualquier alta de piloto.*
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
