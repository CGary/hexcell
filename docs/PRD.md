# Documento de Requisitos del Producto (PRD)
## Proyecto: Orquestador Multi-Célula HexCell (v1.0.0)

### 1. Control de Versiones y Estado
* **Estado:** Aprobado para Desarrollo.
* **Rol de Autoría:** Consultor de Producto Senior & Arquitecto de Soluciones.
* **Pila Tecnológica Núcleo:** Rust (Backend Nativo), Docker (Aislamiento), SQLite (Persistencia Dual), whatsmeow como adaptador de canal en la Fase A y Meta Cloud API + Caddy (Proxy Inverso) en la Fase B.

---

### 2. Descripción General y Objetivos Comerciales
HexCell es una plataforma de software multi-célula (*multi-tenant*) de alta eficiencia diseñada para ejecutarse en entornos de hardware locales restringidos (servidor Intel i7 de hace 10 años, 8 GB de memoria RAM, almacenamiento SSD). El producto permite empaquetar, desplegar y operar de forma masiva bots automatizados para WhatsApp dirigidos a microempresas locales, cubriendo los casos de uso de atención al cliente, respuestas a preguntas frecuentes, catálogo/venta de productos y agendamiento de servicios.

El objetivo central es minimizar el costo operativo por célula mediante una ejecución nativa sin sobrecarga de memoria.

La unidad desplegable por cliente se denomina **célula**: un contenedor del núcleo Rust (más su sidecar de canal cuando el canal lo exige), un volumen de datos propio y un par de bases SQLite independientes. En la CLI y en los identificadores de código, el sustantivo es `cell` (`hexcell-admin cell pause`, `--id <cell_id>`, binario `hexcell-cell`).

---

### 2 bis. Estrategia de Canal por Fases

El producto no ataca de golpe la infraestructura completa. La inversión en canal oficial, dominio público, certificados y plano de control solo se justifica cuando el negocio esté validado, de modo que el desarrollo se divide en dos fases con una compuerta explícita entre ellas.

#### Fase A — MVP de validación (canal no oficial)

Se emplea la biblioteca **whatsmeow** (Go), que implementa el protocolo no oficial de WhatsApp Web. La conexión es un **websocket saliente**: no hay webhook entrante, no hace falta IP pública, ni Caddy, ni terminación TLS entrante, ni handshake anti-Hairpin. El servidor local se conecta hacia fuera y recibe los mensajes por ese mismo canal.

El alcance de la Fase A es deliberadamente estrecho: **exactamente dos células piloto**.

* **piloto-01** — negocio de prueba del propio dueño del proyecto, con un número de WhatsApp nuevo y dedicado.
* **piloto-02** — negocio de un conocido, también con un número nuevo y dedicado.

Docker se emplea desde el primer día: la unidad de despliegue es la misma célula contenedorizada que usará la Fase B, aunque su adaptador de canal sea distinto.

**Riesgos asumidos conscientemente en la Fase A:**

| Riesgo | Naturaleza | Mitigación aceptada |
| :--- | :--- | :--- |
| **Ban permanente del número** por parte de WhatsApp. | El canal no es oficial; la plataforma puede desactivar el número sin previo aviso ni derecho a apelación. | Cada célula piloto usa un **número nuevo y dedicado**, nunca el número principal del negocio. Un ban cuesta un número desechable, no la línea comercial del cliente. Se suma calentamiento de cuenta: volumen gradual, retardos humanizados y política de solo responder — el bot nunca inicia conversaciones con desconocidos. |
| **Roturas de protocolo** cuando WhatsApp cambia su implementación. | La biblioteca la mantiene una comunidad de voluntarios; una rotura deja el canal inoperativo hasta que alguien la arregle. | Precedente medido: [la rotura de abril de 2026 en whatsmeow](https://github.com/lharries/whatsapp-mcp/issues/216) se resolvió en días mediante un simple *bump* de versión de la dependencia; el [incidente equivalente en Baileys](https://github.com/WhiskeySockets/Baileys/issues/2488) sirve de contraste para la elección de biblioteca. Se pacta con los pilotos la posibilidad de semanas de silencio y se mantiene la dependencia fácilmente actualizable. |
| **Violación de los Términos de Servicio de WhatsApp.** | El uso de clientes no oficiales incumple los ToS de la plataforma. | Se acepta **temporalmente y solo como riesgo de validación**, con dos pilotos controlados y sin comercialización. La Fase B existe precisamente para eliminar este riesgo antes de vender. |

#### Compuerta de salida

Cuando las dos células piloto operen de forma estable y el negocio quede validado, **el tercer cliente dispara la Fase B**. No se comercializa sobre canal no oficial.

#### Fase B — Comercial (canal oficial)

Se adopta la **Meta Cloud API** con recepción por webhooks. Aquí se descongela todo lo que la Fase A mantiene en reserva: Caddy, subdominios por cliente, On-Demand TLS, Embedded Signup, `override_callback_uri` y el plano de control completo.

La **entrada pública queda pendiente de ADR**, entre dos opciones con implicaciones muy distintas:

* **Cloudflare Tunnel (capa gratuita).** El TLS termina en el edge de Cloudflare y el túnel es una conexión saliente desde el servidor local. Elimina la necesidad del handshake sintético anti-Hairpin (FR-04) y del On-Demand TLS de Caddy, porque no hay certificado que emitir ni puerto que abrir en el router doméstico.
* **VPS de ~3 USD/mes + WireGuard.** El TLS termina en el propio Caddy, que corre detrás del túnel WireGuard. Conserva íntegra la arquitectura original del PRD, incluido el handshake anti-Hairpin y la emisión de certificados bajo demanda, a cambio de un coste fijo mensual.

---

### 3. Requisitos

#### A. Requisitos Funcionales (FR)
* **FR-01: Recepción de Mensajes Entrantes por Fase.**
  * *Fase A:* recepción de mensajes a través de la **sesión whatsmeow** que mantiene el sidecar Go sobre un websocket saliente. Cada evento entrante se normaliza y se entrega al núcleo Rust a través del puerto de canal (FR-12), con su identificador de deduplicación. No existe petición HTTP entrante que verificar ni firmar.
  * *Fase B:* recepción y verificación de los **webhooks de la Meta Graph API**: desafío de suscripción (`hub.mode`, `hub.verify_token`, `hub.challenge`), validación de la firma criptográfica de cada entrega (`X-Hub-Signature-256`, HMAC-SHA256 sobre el cuerpo exacto y sin reserializar) y política de respuesta `HTTP 200 OK` inmediata antes de procesar, para no activar la máquina de reintentos de la API Graph.
  * *Nota documental:* la redacción original de FR-01 se perdió por truncado del documento fuente. El texto anterior es la **reconstrucción aprobada** y sustituye definitivamente al marcador de TODO.
* **FR-02: Aislamiento Completo por Célula:** Cada microempresa debe operar dentro de un contenedor Docker dedicado e independiente basado en imágenes mínimas (Alpine/Scratch), con el consumo objetivo de RAM en reposo que fija NFR-01 para la fase correspondiente.
* **FR-03: Gestión de Configuración Dinámica (Caddy) *(solo Fase B)*:** El sistema debe registrar subdominios únicos por cliente (`clienteX.midominio.com`) de manera programática en la API de administración de Caddy sin interrumpir el tráfico de terceros.
* **FR-04: Handshake Sintético de Red *(solo Fase B)*:** Antes de registrar cualquier URL en Meta, el orquestador local debe validar la validez del certificado TLS y el enrutamiento público inyectando el SNI y resolviendo el socket directamente a la interfaz local (`127.0.0.1:443`) para eludir restricciones de Hairpin NAT. Su vigencia depende de la decisión de entrada pública: solo aplica si el TLS termina en el propio Caddy (opción VPS + WireGuard).
* **FR-05: Arquitectura de Persistencia Dual (Dual-DB):** Cada contenedor debe desacoplar el estado transaccional del conocimiento de negocio mediante dos bases de datos SQLite físicas independientes: `sessions.db` (Lectura/Escritura continua) y `knowledge_live.db` (Lectura intensiva de RAG).
* **FR-06: Indexación en Sombra (Shadow DB):** Las actualizaciones de catálogo o embeddings de IA no deben bloquear la producción. Deben compilarse asíncronamente en un archivo `knowledge_staging.db` mediante llamadas por lotes a APIs externas.
* **FR-07: Conmutación Atómica por Épocas:** La promoción de nuevos conocimientos en el bot debe ocurrir en microsegundos usando renombrado de archivos por épocas (`knowledge_epoch_N.db`), manipulación de enlaces simbólicos y reemplazo atómico de punteros en memoria (`ArcSwap`), seguido de un drenaje asíncrono controlado (`Graceful Drain`) del pool antiguo para evitar corrupciones en el modo WAL de SQLite.
* **FR-08: Control de Admisión Anti-Spam (GCRA):** Control de admisión basado en el algoritmo *Generic Cell Rate Algorithm* (GCRA) sin cerrojos de memoria, aplicado **sobre el flujo normalizado del puerto de canal** (FR-12) y no sobre la capa HTTP, de modo que el mecanismo sea idéntico en ambas fases.
  * *Fase A:* el GCRA se interpone en el stream de eventos que llega por el websocket, descartando el exceso antes de alocar memoria de procesamiento. No hay respuesta que devolver a nadie: el mensaje simplemente no se procesa y el descarte queda registrado.
  * *Fase B:* además del descarte, se conserva el patrón *Fast-Reject* con `HTTP 200 OK` inmediato hacia Meta, para anular las tormentas de reintentos que la API Graph dispara ante códigos 429/503.
* **FR-09: Semáforo de Concurrencia de CPU:** Límite estricto de tareas Tokio en vuelo simultáneas por contenedor para mitigar la degradación por cambio de contexto en el procesador.
* **FR-10: Contabilidad Financiera de Dos Fases:** Control atómico previo a la llamada del LLM (*Pre-Execution Hold*) basado en la longitud estimada del prompt y conciliación posterior (*Post-Execution Reconcile*) según los tokens reales devueltos por la API (Gemini/Groq), conmutando a un modo degradado de reglas fijas locales al agotarse el saldo. Opera sobre el flujo normalizado del puerto de canal, con independencia del transporte.
* **FR-11: Operaciones CLI de Tráfico Amortiguado (Traffic Shedding):** Herramienta de línea de comandos capaz de suspender clientes sin generar errores hacia el canal.
  * *Fase A:* detener los contenedores de la célula (núcleo y sidecar). No interviene Caddy: al cerrarse el websocket saliente, el tráfico entrante cesa por construcción y no queda ninguna petición sin contestar.
  * *Fase B:* *blackholing* en Caddy (HTTP 200 inmediato estático) **antes** de emitir el SIGTERM de Docker, asegurando que no se generen respuestas HTTP 502 hacia Meta.
* **FR-12: Puerto de Canal (`ChannelAdapter`):** El núcleo Rust no conoce ningún transporte de WhatsApp. Toda integración de canal se implementa detrás de un trait `ChannelAdapter` que actúa como **frontera de migración**: el salto de la Fase A a la Fase B debe ser un cambio de adaptador, no una reescritura.

  El puerto se abstrae **hacia el caso más restrictivo**, que es la Cloud API, no hacia el más permisivo. Un puerto modelado sobre las libertades de whatsmeow —enviar lo que sea, a quien sea, cuando sea— obligaría a rediseñarlo en la Fase B, que es exactamente lo que FR-12 existe para evitar. El adaptador de whatsmeow implementa la semántica restrictiva de forma trivial (ventana siempre abierta, plantillas nunca requeridas); el de la Cloud API la implementa de verdad. El puerto normaliza siete elementos:
  1. **Evento entrante canónico:** remitente, conversación, contenido, marca temporal e identificador de deduplicación.
  2. **Envío tipado:** operación `send(conversation_id, mensaje)` donde el mensaje es `RespuestaLibre` o `Plantilla { id, parámetros }`. La distinción no es cosmética: fuera de la ventana de servicio, la Cloud API solo acepta plantillas previamente aprobadas.
  3. **Resultado tipado del envío:** `send()` no devuelve un booleano ni un error opaco, sino un resultado que enumera los fallos del caso restrictivo: `FueraDeVentana`, `PlantillaRequerida`, `LimiteDeTasa`, `DestinatarioInvalido`. El núcleo debe distinguirlos porque cada uno exige una reacción distinta, y ninguno de ellos es un fallo de programación.
  4. **Estado de la ventana de servicio:** el puerto expone, por conversación, si la ventana de 24 horas está abierta y cuándo expira. En whatsmeow la implementación es trivial —siempre abierta—, pero el núcleo consulta el mismo contrato en ambas fases.
  5. **Identidad de conversación:** el transporte expone identificadores propios (Meta usa `wa_id`, whatsmeow usa JID) que el adaptador mapea a un identificador interno del sistema. **`sessions.db` nunca almacena identificadores de transporte crudos.**
  6. **Acuses normalizados:** `sent`, `delivered`, `read`, `failed`, con la misma semántica sea cual sea el canal.
  7. **Ciclo de vida de sesión (sub-trait opcional):** emparejamiento por QR o por código y persistencia de credenciales. Solo lo implementan los adaptadores no oficiales; la Cloud API no lo necesita y no lo implementa.

  El núcleo define y documenta su **política ante `FueraDeVentana`** —encolar la respuesta hasta que el cliente vuelva a escribir, o escalar a un humano— desde la Fase A, aunque en ella el caso no se dispare nunca. Una política escrita cuando el fallo no ocurre se diseña con calma; escrita el día que ocurre, se improvisa.

#### B. Requisitos No Funcionales (NFR)
| ID | Categoría | Requisito Técnico |
| :--- | :--- | :--- |
| **NFR-01** | Eficiencia | Consumo máximo por célula en reposo, por fase: **Fase A ≤ 80 MB** (núcleo Rust + sidecar Go, que añade unos 15-30 MB); **Fase B < 50 MB** (objetivo original con Cloud API, sin sidecar). |
| **NFR-02** | Disponibilidad *(solo Fase B)* | Tasa nula (0%) de errores HTTP 502/503 expuestos hacia la WAN de Meta durante suspensiones o reactivaciones. |
| **NFR-03** | Latencia | Conmutación interna de base de datos de conocimiento inferior a 10 milisegundos. |
| **NFR-04** | Seguridad *(solo Fase B)* | Cifrado forzoso HTTPS TLS v1.2/v1.3 gestionado automáticamente vía Caddy (On-Demand TLS), si la entrada pública elegida termina el TLS en el propio servidor. |
| **NFR-05** | Seguridad | Aislamiento estricto de almacenamiento: Un contenedor no puede mapear ni acceder al volumen de datos de otra célula. |

---

### 4. Arquitectura y Ciclo de Vida de los Datos

#### Patrón Shadow DB e Inmutabilidad de Épocas

```
[Flujo de Actualización de Conocimiento]
Panel Admin -> Payload JSON -> Contenedor Rust
|
(Crea) knowledge_staging.db
| -> Ingesta de Embeddings (API externa)
(Sella) PRAGMA wal_checkpoint(TRUNCATE);
|
(Renombra) knowledge_epoch_2.db
| -> Cambia enlace simbólico atómico
(Memoria) ArcSwap::store(Nuevo Pool)
|
[Mensajes de WhatsApp consumen Epoch 2]
|
(Drena) old_pool.close().await
| -> Libera FDs de Epoch 1 sin corrupción WAL
```

#### Puerto de canal y despliegue de la célula

```
[Fase A — canal no oficial]
WhatsApp <--websocket saliente--> [Sidecar Go: whatsmeow]
                                          |
                                    IPC / socket local
                                          |
                              [Núcleo Rust: ChannelAdapter]
                                          |
                           GCRA -> Presupuesto LLM -> RAG -> sessions.db

Una célula = 2 contenedores (núcleo + sidecar) con red local y volumen compartidos.

[Fase B — canal oficial]
Meta Cloud API --webhook HTTPS--> [Entrada pública (ADR)] --> [Núcleo Rust: ChannelAdapter]
                                          |
                           GCRA -> Presupuesto LLM -> RAG -> sessions.db

Una célula = 1 contenedor (núcleo). El sidecar desaparece.
```

---

### 5. Matrices de Ciclo de Vida de Administración

#### Secuencia de Suspensión — Fase A (CLI Central)
1. **Detener el sidecar:** cierre ordenado de la sesión whatsmeow. Al caer el websocket saliente, cesa la entrada de mensajes sin dejar peticiones sin respuesta.
2. **SIGTERM al contenedor del núcleo:** con un tiempo de gracia de 30 segundos (`t=30`). El binario en Rust intercepta la señal, deja de aceptar eventos del puerto, drena las peticiones RAG activas, ejecuta un checkpoint de SQLite y finaliza limpiamente (`Exit 0`).
3. **Liberación de Memoria:** el kernel remueve ambos procesos de la memoria RAM del servidor local.

#### Secuencia de Suspensión — Fase B (CLI Central)
1. **PATCH Caddy Admin API:** Sustituir la ruta de `reverse_proxy` por un `static_response_handler` que devuelva HTTP 200 OK con `{}` a Meta de forma inmediata.
2. **SIGTERM Docker Container:** Detener el contenedor del cliente con un tiempo de gracia de 30 segundos (`t=30`), con el mismo apagado ordenado descrito arriba.
3. **Liberación de Memoria:** El kernel remueve el proceso de la memoria RAM del servidor local.

#### Secuencia de Reactivación (CLI Central)
1. **POST Docker API:** Iniciar los contenedores de la célula. En la Fase B, Caddy mantiene el comportamiento estático activo absorbiendo webhooks en paralelo; en la Fase A no hay nada que absorber, porque el canal permanece desconectado hasta que el sidecar reanuda la sesión.
2. **Readiness Polling local:** La CLI interroga al endpoint interno `http://{IP_DOCKER}/health/ready` cada 100ms. El contenedor solo responde 200 OK tras comprobar que las conexiones SQLite (`sessions.db` y `knowledge_live.db`) están activas, las estructuras atómicas GCRA cargadas y el puerto de canal enlazado con su adaptador.
3. **Reconexión del canal:** en la Fase A, el sidecar restablece la sesión whatsmeow desde sus credenciales persistidas, sin necesidad de volver a escanear el QR. En la Fase B, un **PATCH a la Caddy Admin API** conmuta de la respuesta estática al `reverse_proxy` solo tras la primera confirmación positiva de salud.

---

### 6. Criterios de Aceptación para QA
* **Prueba de Carga del Canal:** sometimiento de una célula a 100 eventos concurrentes por el puerto de canal (Fase A: inyectados en el stream normalizado; Fase B: peticiones simulando la API de Meta). El sistema debe activar el control de admisión GCRA, descartar el exceso —devolviendo HTTP 200 rápido cuando exista petición que contestar— y el uso de memoria RAM no debe incrementarse en más del 15% del consumo base.
* **Prueba de Resiliencia de Sesión (Fase A):** reiniciar los contenedores de una célula y verificar que el sidecar restablece la sesión whatsmeow desde las credenciales persistidas, sin re-emparejamiento manual. Tras un reinicio **desacompasado de ambos procesos, en cualquiera de los dos órdenes**: cero eventos perdidos y cero eventos procesados por duplicado, sostenido por el outbox durable del sidecar y la deduplicación del núcleo.
* **Prueba de Recuperación de Sesión (Fase A):** restaurar una célula desde sus respaldos —las tres bases, incluido el `sqlstore` del sidecar— sobre un entorno limpio. La prueba **solo se supera si el bot reconecta al canal y responde a un mensaje real**; recuperar los ficheros con la sesión muerta cuenta como fallo.
* **Prueba de Resiliencia del Enlace TLS (Fase B):** bloquear artificialmente el Hairpin NAT del router local. Si la entrada pública elegida termina el TLS en el propio Caddy, el script de orquestación debe completar el onboarding con éxito mediante la bandera `--resolve` forzada a nivel de socket. Si el TLS termina en el edge, este criterio queda sin objeto y se sustituye por la verificación del túnel.
* **Prueba de Consistencia en Modo WAL:** ejecutar un intercambio de conocimiento mientras se procesan 20 lecturas RAG simultáneas. El sistema no debe arrojar excepciones de tipo `SQLITE_BUSY` ni dejar huérfanos archivos `.db-wal` o `.db-shm`.
