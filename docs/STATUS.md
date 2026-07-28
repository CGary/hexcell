# Estado del Proyecto

> Registro vivo del avance. Última actualización: 2026-07-28.

## Fase actual
**Canal propio en producción — diseño.** No existe todavía código fuente ni scaffold Rust.

El proyecto opera sobre **dos canales que conviven**, no sobre dos fases que se suceden. El **canal
propio** (whatsmeow, sidecar Go) es el canal por defecto y permanente, con clientes de pago reales.
El **canal oficial** (Meta Cloud API) queda pospuesto a una segunda etapa y se incorporará como canal
adicional cuando aparezca un cliente que lo justifique. Ver [plan/README.md](plan/README.md) y
[adr/README.md](adr/README.md).

> Lo que se estudió y **no** se hizo, con su motivo y sus condiciones de reapertura, vive en
> [bitacora-de-descartes.md](bitacora-de-descartes.md). Consúltala antes de reabrir un debate: si la
> idea ya está allí, no se discute desde cero.

## Definido
* **Canal propio permanente y canal oficial aditivo por demanda** (2026-07-28, `adr-0014`, que
  supersede a `adr-0008`). whatsmeow deja de ser un canal temporal de validación y pasa a ser el
  canal de producción por defecto, con clientes de pago. La Cloud API se pospone a una segunda etapa
  y se incorporará como canal que **convive**, no que sustituye. Quedan **derogadas** la regla "no se
  comercializa sobre canal no oficial" y la compuerta del tercer cliente. Motivos registrados: el
  coste de gestión comercial por cliente —que recae sobre el tiempo del fundador, el recurso más
  escaso, y no aparece en ningún diagrama técnico— y el cobro de mensajes de servicio anunciado por
  Meta para el 1 de octubre de 2026.
* **El riesgo de baneo es estructural y el baneo se trata como evento esperado, no como fallo**
  (2026-07-28, `adr-0015`). Meta detecta la biblioteca por su huella de protocolo: los issues #810 y
  #807 (mayo de 2025) y #989 (noviembre de 2025) de `tulir/whatsmeow` documentan baneos y avisos de
  *"unauthorized tools"* sobre cuentas de **bajo volumen y solo-respuesta**, sin patrón accionable y
  cerrados como *not planned*. Ninguna medida de comportamiento lo elimina. La política se organiza
  en cuatro capas —reducir la probabilidad, detectar pronto, contener el daño y recuperar— con cada
  medida marcada como [causa documentada] o [precautorio], y con una lista explícita de lo que **no**
  se hace (proxies o rotación de IP, camuflar la huella de la biblioteca, números virtuales, mensajes
  proactivos, reconexión agresiva tras un baneo temporal).
* **Emitir el indicador de "escribiendo" es higiene documentada de coste cero, no una defensa.** El
  whitepaper oficial de WhatsApp de febrero de 2019 lo nombra como señal de abuso cuando una cuenta
  envía continuamente sin dispararlo, pero el documento es anterior a la arquitectura
  multi-dispositivo, no hay evidencia pública de eficacia y falsificarlo cuesta una línea de código.
  El jitter y los protocolos de "calentamiento" que se venden alrededor quedan **excluidos** por
  folclore.
* **Higiene del número:** un número dedicado en exclusiva al bot, sobre **SIM física con antigüedad y
  uso previo**, a nombre del cliente; nunca virtual, VoIP ni recién activada, con perfil de negocio
  completo. El teléfono primario del dueño debe seguir en uso humano real. **El cliente es siempre el
  titular del número y de la SIM; HexCell nunca**, porque el titular es quien puede apelar y así el
  baneo no cruza a la identidad del proveedor.
* **Riesgo de mantenimiento asumido:** whatsmeow tiene **bus factor 1** y su patrón de rotura
  recurrente es `Client outdated (405)` cuando WhatsApp sube la versión mínima de cliente. No se
  compromete ningún tiempo de recuperación que dependa de un mantenedor voluntario.
* **Puerto de canal (`ChannelAdapter`, FR-12)** como frontera entre el núcleo y el transporte, con
  **dos adaptadores vivos a la vez** en células distintas. Se mantiene la abstracción hacia el caso
  más restrictivo con esta distinción: el **tipo** admite el resultado restrictivo, la **política** de
  cada adaptador decide si lo produce; el adaptador del canal propio no impone una ventana de 24 h
  artificial.
* **El mapeo de identidad de conversación pertenece al adaptador, no al núcleo** (2026-07-28,
  `adr-0010`). El adaptador traduce el identificador de transporte —JID en whatsmeow, `wa_id` en la
  Cloud API— al identificador interno y **entrega ya traducido** lo que cruza el puerto; el núcleo
  trata ese identificador como **opaco** y no lo deriva, ni lo interpreta, ni lo invierte. Se elimina
  así la responsabilidad duplicada que la etapa A-2 asignaba al núcleo, que habría sido la función
  identidad. La regla del PRD conserva su alcance estrecho: lo que se prohíbe es que **`sessions.db`**
  almacene identificadores de transporte crudos, no que existan en ninguna parte —dentro del
  adaptador existen por necesidad—.
* **El mapeo persiste en un almacén propio del adaptador, separado del `sqlstore`** (2026-07-28,
  `adr-0010`), sobre el volumen de la célula. El motivo es la rama `LoggedOut` con `device_removed`:
  obliga a **descartar** el `sqlstore`, y el mapeo tiene que **sobrevivir** a ese re-emparejamiento
  para que cada contacto siga cayendo en su hilo. Guardarlo dentro del `sqlstore` lo destruiría justo
  en el único escenario en que hace falta. En ese mismo almacén vive la **lista de exclusión (STOP)**
  de la etapa A-3, por la misma razón. Ese almacén es la **cuarta base del respaldo**.
* **Arquitectura de célula sobre canal propio:** dos contenedores (núcleo Rust + sidecar Go de
  whatsmeow) compartiendo red local y volumen, comunicados por IPC sobre socket local. El sidecar es
  **permanente**, no transitorio.
* **Docker desde el día 1**, también en la fase de validación.
* **Nomenclatura:** la unidad desplegable por cliente se llama **célula**; en CLI e identificadores de
  código, `cell` (`hexcell-admin cell pause`, `--id <cell_id>`, binario `hexcell`).
* **Células piloto:** `piloto-01` (negocio de prueba del propio dueño) y `piloto-02` (un conocido).
  Son el **comienzo de la cartera**, no su alcance total: ya no existe el límite de dos células.
* **Respaldos adelantados a la etapa A-2**, en lugar de esperar al endurecimiento final: con pilotos
  reales no pueden esperar. Cubren **las cuatro bases** —`sessions.db`, `knowledge_live.db`, el
  almacén de identidad del adaptador y el `sqlstore` del sidecar—, este último copiado por el propio
  sidecar vía `VACUUM INTO` sobre orden IPC y con frecuencia alta (cada pocas horas), porque las
  credenciales del protocolo Signal evolucionan. El respaldo del `sqlstore` deja de ser transitorio:
  pasa a ser respaldo de **disponibilidad del canal**. **La restauración solo se da por buena si el
  bot reconecta y responde**; recuperar ficheros con la sesión muerta cuenta como fallo.
* **Reparto del respaldo entre A-2 y A-3** (2026-07-28). La etapa A-2 **diseña** el procedimiento
  completo de las cuatro bases, escribe el runbook con su bifurcación, implementa las copias que no
  necesitan sidecar y deja versionado el **contrato IPC** de la copia del `sqlstore` sin ejecutarlo;
  sus criterios de aceptación se cumplen contra el adaptador simulado. La etapa A-3 lo completa con
  la **copia ejecutada por el propio proceso del sidecar** y el **ensayo extremo a extremo** —célula
  restaurada que reconecta al canal y responde a un mensaje real, con las dos ramas de
  `device_removed` recorridas—. Elimina la dependencia circular que exigía a A-2 verificar contra un
  sidecar que solo existe en A-3.
* **Regla de restauración del `sqlstore`:** no se restaura **solo** si hubo `LoggedOut` con
  `device_removed` —whatsmeow ya borró la sesión y el dispositivo no existe en el servidor, de modo
  que restaurar es inútil, no inválido—; ante cualquier otra desconexión el respaldo sigue siendo
  válido, igual que ante corrupción o fallo de disco. El runbook debe separar ambos casos: si no lo
  hace, alguien intentará restaurar un `sqlstore` muerto en plena crisis.
* **Re-emparejamiento por `PairPhone()` como procedimiento de recuperación de primera clase**
  (segunda capa, etapa A-3): código de ocho caracteres que el piloto teclea en su propio teléfono,
  sin necesidad de tenerlo en mano. Se **ensaya y cronometra en el alta de cada cliente**, porque
  exige al dueño con el teléfono delante: si no se ha practicado, el tiempo de recuperación lo fija
  su agenda, no el código.
* **Puerto de canal abstraído hacia el caso más restrictivo** (FR-12): envío tipado
  (`RespuestaLibre` | `Plantilla`), resultado tipado (`FueraDeVentana`, `PlantillaRequerida`,
  `LimiteDeTasa`, `DestinatarioInvalido`) y estado de la ventana de servicio de 24 h. El adaptador
  simulado de la etapa A-2 imita la semántica de la Cloud API, no la de whatsmeow, y los tests de
  contrato corren contra ese caso difícil.
* **Invariante solo-respuesta elevado al sistema de tipos** (etapa A-3): un envío solo es construible
  a partir de un identificador de evento entrante válido, de modo que violarlo no compila. El test y
  el contador de la alerta se conservan como segunda línea, no como única. Lo acompañan el **TTL
  absoluto en la cola de salida** —vector real de violación, porque un reintento tardío parece
  iniciación de conversación—, la latencia mínima de respuesta, el horario de atención y el drenaje
  sin envío al pausar o eliminar una célula.
* **Outbox durable en el sidecar** (etapa A-3): todo evento entrante se persiste con `fsync` como
  primera acción, antes de entregarlo al núcleo; entrega *at-least-once* con confirmación explícita y
  deduplicación en el núcleo. Limitación documentada: el acuse de protocolo hacia WhatsApp es
  automático y no se puede diferir, de modo que queda una ventana de pérdida de microsegundos.
* **Alertas push y dead-man's switch adelantados a la etapa A-6**: bot de Telegram ante **ocho**
  condiciones —sesión desvinculada, sidecar sin reconectar más de 5 minutos, bucle de reinicios,
  saldo agotado, descartes GCRA anómalos, descarte de envíos no solicitados (invariante anti-ban),
  **baneo temporal detectado** (máxima prioridad, por ser el único aviso previo que suele existir) y
  **caída anómala del ratio de acuses de entrega segmentado por contacto** (detección indirecta de
  bloqueos; el número de reportes no es observable de ninguna forma)—; más healthchecks.io con ping
  cada 5 minutos para que la caída total del servidor se notifique desde fuera. Descongela
  deliberadamente un mínimo de la observabilidad de la etapa B-3, porque hay usuarios reales desde el
  primer día. **La observabilidad acorta el tiempo de reacción, no evita el baneo:** el baneo
  permanente suele llegar sin aviso.
* **Canary de biblioteca** (etapa A-6): una célula centinela propia, con número propio, corre la
  versión candidata de whatsmeow durante 72 horas antes de escalonar la actualización al resto de la
  cartera. Nunca se actualizan todas las células el mismo día.
* **Endurecimiento contra el patrón "compila ≠ correcto"** (2026-07-27), aplicado transversalmente:
  validación semántica del puerto en A-1 (`match` exhaustivo y cotejo contra la documentación
  oficial de la Cloud API), `hexcell-meta` vacío hasta resolver el ADR-0013, CI de A-1 con alcance
  declarado, `/health/ready` condicionado a sesión de canal activa (A-2/A-3/A-6, README y PRD
  alineados), ventana de deduplicación dimensionada frente al horizonte de reentrega (A-2),
  invariante continuo anti-envíos-no-solicitados con alerta (A-3/A-6), criterio de no-falso-positivo
  en GCRA (A-4), reversión de épocas con la misma validación semántica que la promoción (A-5), y
  eliminación de la vía de escape del criterio del núcleo intacto en B-1 (ahora bloquea la
  aceptación y exige revisar el ADR-0010).
* **Riesgo de ecosistema del canal propio: asumido con vigilancia progresiva** (2026-07-27,
  reformulado el 2026-07-28). El endurecimiento de Meta contra clientes no oficiales se acepta como
  riesgo consciente y **permanente**, no como riesgo temporal de validación; las medidas concretas
  son las cuatro capas de `adr-0015`, y lo que disciplina el crecimiento son las compuertas de riesgo
  de la etapa A-7, no un límite temporal.
* **El canal oficial nace como canal solo-respuesta** (2026-07-27): se usará únicamente para
  responder consultas entrantes; no hay plan de mensajes salientes iniciados por el negocio. El bot
  queda por diseño fuera de la prohibición de chatbots de propósito general de Meta (enero de 2026), y
  la política ante `FueraDeVentana` queda decidida: esperar a que el cliente vuelva a escribir, con
  escalada a humano como excepción. El envío tipado `Plantilla` del puerto (FR-12) se conserva en el
  contrato, sin uso previsto en esta versión del producto.
  * **CORRECCIÓN (2026-07-28):** queda **invalidada** la parte de esta decisión que afirmaba que el
    transporte del canal oficial cuesta aproximadamente 0. Meta anunció el 1 de julio de 2026 que
    **desde el 1 de octubre de 2026 cobrará también los mensajes de servicio** (las respuestas dentro
    de la ventana de 24 h), con tarifas publicables hasta el 1 de septiembre de 2026. *Estado de la
    evidencia: confirmado por múltiples BSPs, todavía no reflejado en la página oficial de precios de
    Meta.* El coste por conversación sobre canal oficial debe recalcularse.
* **Modo coexistencia de Meta como opción preferente de la segunda etapa** (2026-07-28). Un mismo
  número puede funcionar a la vez en la app de WhatsApp Business del móvil y en la Cloud API,
  sincronizando 180 días de historial y contactos, y el integrador recibe por webhook
  (`smb_message_echoes`) lo que el dueño responde a mano desde su app —lo que resuelve el pendiente
  de la interfaz de intervención humana—. Requiere Embedded Signup de un Solution Partner o Tech
  Provider: no hay ruta de Cloud API directa. Limitaciones: 20 mensajes por segundo, sin grupos, sin
  mensajes efímeros, sin vista única, sin ubicación en vivo, sin listas de difusión, sin catálogo ni
  pedidos por API.
* **Compuerta pre-registrada y roles asimétricos de los pilotos** (etapa A-7): los umbrales numéricos
  y los **criterios de fracaso** se fijan por escrito antes del primer alta. Ya no deciden un cambio
  de canal, sino **si el producto sigue adelante y si se abren más altas**. **piloto-01 es banco de
  pruebas técnico y sus datos no cuentan para la validación de negocio** (el dueño no puede ser su
  propio cliente); **piloto-02 paga un importe simbólico pero real desde el segundo mes**, porque el
  acto de pagar es la métrica y "sí pagaría" no es evidencia.
* La pila tecnológica: Rust (backend nativo), Docker (aislamiento por célula), SQLite dual
  (persistencia); Caddy (proxy inverso + SSL) solo en células sobre canal oficial.
* El modelo de despliegue por contenedores aislados (imágenes Alpine/Scratch), con presupuesto de
  memoria por canal: **≤ 80 MB por célula sobre canal propio** (núcleo + sidecar, permanente) y
  < 50 MB sobre canal oficial, sin sidecar. **Ninguna de las dos cifras se ha validado bajo carga
  sostenida**, el techo de células por servidor es desconocido hasta medirlo, y el cuello probable no
  es la memoria sino la CPU y la E/S.
* La viabilidad técnica del hardware (Intel i7 de 10 años, 8 GB RAM, SSD).
* Requisitos funcionales y no funcionales: ver [PRD.md](PRD.md).
* **FR-01 reconstruido y aprobado**, ahora redactado por canal configurado en la célula.
* **Plan de implementación en 7 etapas de canal propio + 3 de canal oficial: ver
  [plan/README.md](plan/README.md).** Cubre FR-01..FR-12 y NFR-01..NFR-05, y sitúa los pendientes de
  producto de más abajo como bloqueos declarados en las etapas que los necesitan.

## Pendiente
* **Valores numéricos de las compuertas de riesgo de cartera**: el **techo duro de células vivas**
  mientras el canal propio sea el único, y el **umbral de incidentes de baneo** (cuántos, en qué
  ventana) que congela todas las altas hasta analizar. Sustituyen a la compuerta del tercer cliente y
  son decisión de negocio. — *Tarea 1 de la etapa A-7, bloqueante y anterior a cualquier alta.*
* **Revisión legal local del contrato del canal propio.** El contrato declara el canal como no
  oficial, con el riesgo de baneo explícito, sin garantía de disponibilidad y con modo degradado
  pactado. En varias jurisdicciones las exoneraciones totales frente a microempresas no son
  oponibles, y una cláusula inejecutable es **peor que ninguna** porque genera falsa seguridad. —
  *Bloquea el primer cliente de pago.*
* **Fijar los valores numéricos de la compuerta pre-registrada**: umbrales de éxito (conversaciones
  semanales sostenidas, porcentaje de resolución sin intervención, retención de clientes finales,
  coste máximo por conversación, disponibilidad mínima), **importe del cobro simbólico a piloto-02**
  y techos de los criterios de fracaso. El plan fija la estructura; los números son decisión de
  negocio. — *Tarea 1 de la etapa A-7, bloqueante y anterior a cualquier alta de piloto.*
* **Calibrar los parámetros anti-baneo de la etapa A-3**: TTL absoluto de la cola de salida, latencia
  mínima de respuesta y horario de atención por defecto. El plan fija el mecanismo; los valores se
  calibran con tráfico real. — *Etapa A-3.*
* **Prueba de carga sostenida y techo de células por servidor** (NFR-01): convertir los 80 MB en un
  objetivo medido con límites de cgroup, y descubrir si el cuello real es la memoria o la CPU y la
  E/S. — *Bloquea escalar la cartera más allá de las primeras células.*
* **Resultado del experimento con Meta Verified en piloto-01.** Varios usuarios del issue #810
  reportaron que activarlo detuvo los avisos de *"unauthorized tools"*; es correlación anecdótica sin
  confirmación de Meta y se ensaya como experimento, nunca como medida probada. — *Etapa A-7.*
* **Tarifa de los mensajes de servicio de Meta** una vez publicada (hasta el 1 de septiembre de
  2026), y recálculo del coste por conversación sobre canal oficial. — *Condiciona la viabilidad
  económica de la segunda etapa.*
* **ADR de entrada pública del canal oficial: Cloudflare Tunnel (capa gratuita) frente a VPS ~3
  USD/mes + WireGuard.** Condiciona la vigencia de FR-04 y NFR-04. — *Primera tarea de la etapa B-1;
  determina la mitad del alcance de la etapa B-2.*
* Interfaz de intervención humana para las células sobre canal oficial: si se adopta el modo
  coexistencia, el dueño conserva su app y el problema desaparece; si no, la escalada a humano
  necesita una interfaz provista por HexCell. — *Alcance a declarar en las etapas B-1/B-2.*
* Lógica de negocio específica. — *Bloquea el alcance funcional de la etapa A-2 y se descubre en la
  etapa A-7 con los pilotos reales.*
* Flujos de usuario finales. — *Bloquean la superficie de carga de catálogo de la etapa A-5 y el alta
  comercial automatizada de la etapa B-2.*
* Manejo de excepciones comerciales. — *Condiciona el modo degradado (etapa A-4) y las alertas
  (etapa B-3).*
* Modelo de monetización **sobre el canal propio**, ahora que hay clientes de pago encima de él. —
  *Bloquea la calibración de saldos (etapa A-4) y la suspensión por impago. La etapa A-7 le aporta su
  primera entrada empírica.*
* Proceso exacto de alta (onboarding) comercial de una nueva microempresa. — *El alta operada
  manualmente de los dos pilotos se resuelve en la etapa A-7; su automatización, en la etapa B-2.*
* Decidir licencia (`LICENSE`). — *Etapa A-1, tarea 3. El repositorio git ya está inicializado.*
* Scaffold del workspace Rust (`Cargo.toml`, `src/`) y del módulo Go del sidecar cuando arranque la
  fase de implementación. — *Etapa A-1, tareas 5 y 6.*
