# Documento de Requisitos del Producto (PRD)
## Proyecto: Orquestador Multi-Célula HexCell (v1.0.0)

### 1. Control de Versiones y Estado
* **Estado:** Aprobado para Desarrollo.
* **Rol de Autoría:** Consultor de Producto Senior & Arquitecto de Soluciones.
* **Pila Tecnológica Núcleo:** Rust (Backend Nativo), Docker (Aislamiento), SQLite (Persistencia Dual), whatsmeow como adaptador del canal propio (Fase A, permanente) y Meta Cloud API + Caddy (Proxy Inverso) como adaptador del canal oficial (Fase B, adicional).

---

### 2. Descripción General y Objetivos Comerciales
HexCell es una plataforma de software multi-célula (*multi-tenant*) de alta eficiencia diseñada para ejecutarse en entornos de hardware locales restringidos (servidor Intel i7 de hace 10 años, 8 GB de memoria RAM, almacenamiento SSD). El producto permite empaquetar, desplegar y operar de forma masiva bots automatizados para WhatsApp dirigidos a microempresas locales, cubriendo los casos de uso de atención al cliente, respuestas a preguntas frecuentes, catálogo/venta de productos y agendamiento de servicios.

El objetivo central es minimizar el costo operativo por célula mediante una ejecución nativa sin sobrecarga de memoria.

La unidad desplegable por cliente se denomina **célula**: un contenedor del núcleo Rust (más su sidecar de canal cuando el canal lo exige), un volumen de datos propio y un par de bases SQLite independientes. En la CLI y en los identificadores de código, el sustantivo es `cell` (`hexcell-admin cell pause`, `--id <cell_id>`, binario `hexcell`).

---

### 2 bis. Estrategia de Canal por Fases

El producto no ataca de golpe la infraestructura completa, pero las dos fases **ya no son una secuencia con una compuerta que cierra la primera**. Son **dos canales que conviven**: cada célula se despliega sobre el canal que le corresponde y ambos permanecen vivos a la vez. La Fase A es el **canal propio en producción**; la Fase B es el **canal oficial adicional**, que se incorpora cuando aparece un cliente que lo justifique.

Este rumbo se fijó el 28 de julio de 2026 e **invierte deliberadamente** dos decisiones anteriores de este mismo documento:

* Queda **derogada la regla "no se comercializa sobre canal no oficial"**. El canal propio sostiene clientes de pago reales, sin límite de dos pilotos y sin fecha de caducidad.
* Queda **derogada la compuerta del tercer cliente**. El tercer cliente ya no cierra nada; lo que disciplina el crecimiento son las compuertas de riesgo (techo duro de cartera y umbral de incidentes que congela altas).

No es un matiz de redacción sino una inversión de postura, y se registra como tal. Los motivos completos —coste de gestión comercial por cliente, coste de transporte sobrevenido tras el anuncio de Meta del 1 de julio de 2026 sobre el cobro de los mensajes de servicio desde el 1 de octubre de 2026, y la pérdida de la bandeja del móvil aceptada como pendiente conocido— están en **`adr-0014`** (canal propio permanente), que supersede a `adr-0008` y a las decisiones previas sobre esta materia.

#### Fase A — Canal propio en producción

Se emplea la biblioteca **whatsmeow** (Go), que implementa el protocolo no oficial de WhatsApp Web. La conexión es un **websocket saliente**: no hay webhook entrante, no hace falta IP pública, ni Caddy, ni terminación TLS entrante, ni handshake anti-Hairpin. El servidor local se conecta hacia fuera y recibe los mensajes por ese mismo canal.

Es el **canal por defecto del producto y su modo de producción permanente**. El sidecar Go que aloja la sesión whatsmeow no es andamiaje temporal: acompaña a toda célula sobre canal propio durante toda su vida.

Las dos primeras células siguen siendo `piloto-01` —negocio de prueba del propio dueño, que actúa como banco de pruebas técnico— y `piloto-02` —negocio ajeno—, pero ahora son **el comienzo de la cartera, no su totalidad**. El número máximo de células sobre canal propio es un **techo duro de cartera** cuyo valor concreto es una **decisión de negocio pendiente**.

Docker se emplea desde el primer día: la unidad de despliegue es la misma célula contenedorizada sea cual sea su adaptador de canal.

**Riesgos asumidos conscientemente en el canal propio:**

| Riesgo | Naturaleza | Mitigación aceptada |
| :--- | :--- | :--- |
| **Baneo del número** por parte de WhatsApp. | **Estructural, no conductual.** Meta detecta la biblioteca por su huella de protocolo, y ninguna medida de comportamiento lo elimina. Los issues [#810](https://github.com/tulir/whatsmeow/issues/810) y [#807](https://github.com/tulir/whatsmeow/issues/807) (mayo de 2025, concentrados en Brasil) y [#989](https://github.com/tulir/whatsmeow/issues/989) (noviembre de 2025: suspensiones de 24 h con código de enforcement `BULK_MESSAGING` pese a enviar pocos mensajes con pausas de 5 s) documentan baneos y avisos de *"unauthorized tools"* sobre cuentas de **bajo volumen y solo-respuesta**. Ninguno identificó un patrón accionable y los tres se cerraron como *not planned*. Meta banea del orden de 2 millones de cuentas al mes, el 75 % por decisión automática, y puede hacerlo **sin aviso previo**. | El baneo se documenta como **evento esperado, no como fallo**. Las medidas que reducen la probabilidad actúan sobre el término secundario; las que más valor aportan son las que **reducen el daño**: el cliente es siempre el titular del número y de la SIM —nunca HexCell—, aislamiento estricto por célula, techo duro de cartera, umbral de incidentes que congela altas y contrato que declara el canal como propio y no oficial, sin garantía de disponibilidad y con modo degradado pactado. |
| **Roturas de protocolo** cuando WhatsApp cambia su implementación. | La biblioteca la mantiene una comunidad de voluntarios; una rotura deja el canal inoperativo hasta que alguien la arregle. | Precedente medido: [la rotura de abril de 2026 en whatsmeow](https://github.com/lharries/whatsapp-mcp/issues/216) se resolvió en días mediante un simple *bump* de versión de la dependencia; el [incidente equivalente en Baileys](https://github.com/WhiskeySockets/Baileys/issues/2488) sirve de contraste para la elección de biblioteca. Se mantiene la dependencia fácilmente actualizable y se pacta con el cliente la posibilidad de silencio prolongado. |
| **Mantenimiento con bus factor 1.** | Prácticamente la totalidad de los ~1.620 commits de whatsmeow son de un **único mantenedor**, con actividad casi diaria en junio y julio de 2026. El patrón de rotura recurrente es `Client outdated (405)` ([#415](https://github.com/tulir/whatsmeow/issues/415), [#1031](https://github.com/tulir/whatsmeow/issues/1031)) cuando WhatsApp sube la versión mínima de cliente; el arreglo es siempre actualizar. | **No se compromete ningún tiempo de recuperación que dependa de un tercero voluntario.** La dependencia se pinnea por commit con una ventana de actualización definida —correr atrasado deja de conectar y declara una versión de cliente atípica—, y la actualización se escalona: nunca toda la cartera el mismo día. |
| **Violación de los Términos de Servicio de WhatsApp.** | El uso de clientes no oficiales incumple los ToS de la plataforma. | Se acepta como **riesgo permanente y comercializable**, no como riesgo temporal de validación. Es la decisión invertida el 28 de julio de 2026: el canal oficial deja de existir para eliminar este riesgo y pasa a ser una opción adicional para quien la necesite. El riesgo se traslada de forma explícita al contrato con el cliente. |

#### Condición de activación de la Fase B

La Fase B **no la dispara un número de clientes ni una fecha**. Se activa cuando aparece un cliente que la justifique —típicamente una empresa medianamente grande que pueda asumir el alta y el coste del canal oficial—. Hasta entonces permanece congelada, y cuando se active **se suma** al canal propio: no lo sustituye, no lo cierra y no retira ningún sidecar.

#### Fase B — Canal oficial adicional

Se adopta la **Meta Cloud API** con recepción por webhooks, para las células que lo requieran. Aquí se descongela todo lo que el canal propio no necesita: Caddy, subdominios por cliente, On-Demand TLS, Embedded Signup, `override_callback_uri` y el plano de control completo. Las células sobre canal oficial y las células sobre canal propio conviven en el mismo servidor y bajo el mismo orquestador.

La **entrada pública queda pendiente de ADR**, entre dos opciones con implicaciones muy distintas:

* **Cloudflare Tunnel (capa gratuita).** El TLS termina en el edge de Cloudflare y el túnel es una conexión saliente desde el servidor local. Elimina la necesidad del handshake sintético anti-Hairpin (FR-04) y del On-Demand TLS de Caddy, porque no hay certificado que emitir ni puerto que abrir en el router doméstico.
* **VPS de ~3 USD/mes + WireGuard.** El TLS termina en el propio Caddy, que corre detrás del túnel WireGuard. Conserva íntegra la arquitectura original del PRD, incluido el handshake anti-Hairpin y la emisión de certificados bajo demanda, a cambio de un coste fijo mensual.

---

### 3. Requisitos

#### A. Requisitos Funcionales (FR)
* **FR-01: Recepción de Mensajes Entrantes según el Canal Configurado en la Célula.** Cada célula declara en su configuración sobre qué canal opera, y ese ajuste determina la vía de recepción. Ambas vías son de producción y pueden estar activas simultáneamente en células distintas del mismo servidor.
  * *Célula sobre canal propio (whatsmeow):* recepción de mensajes a través de la **sesión whatsmeow** que mantiene el sidecar Go sobre un websocket saliente. Cada evento entrante se normaliza y se entrega al núcleo Rust a través del puerto de canal (FR-12), con su identificador de deduplicación. No existe petición HTTP entrante que verificar ni firmar.
  * *Célula sobre canal oficial (Meta Cloud API):* recepción y verificación de los **webhooks de la Meta Graph API**: desafío de suscripción (`hub.mode`, `hub.verify_token`, `hub.challenge`), validación de la firma criptográfica de cada entrega (`X-Hub-Signature-256`, HMAC-SHA256 sobre el cuerpo exacto y sin reserializar) y política de respuesta `HTTP 200 OK` inmediata antes de procesar, para no activar la máquina de reintentos de la API Graph.
  * *Nota documental:* la redacción original de FR-01 se perdió por truncado del documento fuente. El texto anterior es la **reconstrucción aprobada** y sustituye definitivamente al marcador de TODO.
* **FR-02: Aislamiento Completo por Célula:** Cada microempresa debe operar dentro de un contenedor Docker dedicado e independiente basado en imágenes mínimas (Alpine/Scratch), con el consumo objetivo de RAM en reposo que fija NFR-01 para su canal.
* **FR-03: Gestión de Configuración Dinámica (Caddy) *(solo en células sobre canal oficial)*:** El sistema debe registrar subdominios únicos por cliente (`clienteX.midominio.com`) de manera programática en la API de administración de Caddy sin interrumpir el tráfico de terceros.
* **FR-04: Handshake Sintético de Red *(solo en células sobre canal oficial)*:** Antes de registrar cualquier URL en Meta, el orquestador local debe validar la validez del certificado TLS y el enrutamiento público inyectando el SNI y resolviendo el socket directamente a la interfaz local (`127.0.0.1:443`) para eludir restricciones de Hairpin NAT. Su vigencia depende de la decisión de entrada pública: solo aplica si el TLS termina en el propio Caddy (opción VPS + WireGuard).
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
* **FR-12: Puerto de Canal (`ChannelAdapter`):** El núcleo Rust no conoce ningún transporte de WhatsApp. Toda integración de canal se implementa detrás de un trait `ChannelAdapter` que actúa como **frontera de coexistencia**: no es el paso de un canal a otro, sino la garantía de que **dos adaptadores viven a la vez**, en células distintas del mismo servidor, sin que el núcleo sepa cuál está debajo. Añadir el canal oficial debe ser escribir un segundo adaptador, no reescribir el producto.

  El puerto se abstrae **hacia el caso más restrictivo**, que es la Cloud API, no hacia el más permisivo. La decisión se mantiene íntegra pese al cambio de rumbo: un puerto modelado sobre las libertades de whatsmeow —enviar lo que sea, a quien sea, cuando sea— no podría albergar después al adaptador oficial, que es exactamente lo que FR-12 existe para evitar.

  La distinción que hace viable la coexistencia es esta: **el TIPO admite el resultado restrictivo; la POLÍTICA de cada adaptador decide si lo produce.** Que `send()` pueda devolver `FueraDeVentana` obliga al núcleo a saber reaccionar, pero **no obliga al adaptador del canal propio a imponer una ventana de 24 horas artificial**: ese adaptador nunca produce ese resultado porque su transporte no lo impone, y fabricar la restricción sería degradar el producto para parecerse a un canal que la célula no usa. El adaptador de la Cloud API sí la implementa de verdad. El puerto normaliza siete elementos:
  1. **Evento entrante canónico:** remitente, conversación, contenido, marca temporal e identificador de deduplicación.
  2. **Envío tipado:** operación `send(conversation_id, mensaje)` donde el mensaje es `RespuestaLibre` o `Plantilla { id, parámetros }`. La distinción no es cosmética: fuera de la ventana de servicio, la Cloud API solo acepta plantillas previamente aprobadas.
  3. **Resultado tipado del envío:** `send()` no devuelve un booleano ni un error opaco, sino un resultado que enumera los fallos del caso restrictivo: `FueraDeVentana`, `PlantillaRequerida`, `LimiteDeTasa`, `DestinatarioInvalido`. El núcleo debe distinguirlos porque cada uno exige una reacción distinta, y ninguno de ellos es un fallo de programación.
  4. **Estado de la ventana de servicio:** el puerto expone, por conversación, si la ventana de 24 horas está abierta y cuándo expira. En whatsmeow la implementación es trivial —siempre abierta, porque el transporte no impone ninguna ventana—, pero el núcleo consulta el mismo contrato sea cual sea el canal.
  5. **Identidad de conversación:** el transporte expone identificadores propios (Meta usa `wa_id`, whatsmeow usa JID) que **el adaptador** —nunca el núcleo— mapea a un identificador interno del sistema. El núcleo recibe ese identificador ya traducido y lo trata como **opaco**: no lo deriva, no lo interpreta y no lo invierte. El mapeo y su almacén son propiedad del adaptador, y ese almacén vive en el volumen de la célula **separado de las credenciales de sesión del transporte**, porque una desvinculación que obliga a descartar las credenciales no debe llevarse por delante la continuidad del hilo. Ese almacén entra en el respaldo por célula. **`sessions.db` nunca almacena identificadores de transporte crudos.**
  6. **Acuses normalizados:** `sent`, `delivered`, `read`, `failed`, con la misma semántica sea cual sea el canal.
  7. **Ciclo de vida de sesión (sub-trait opcional):** emparejamiento por QR o por código y persistencia de credenciales. Solo lo implementan los adaptadores no oficiales; la Cloud API no lo necesita y no lo implementa.

  El núcleo define y documenta su **política ante `FueraDeVentana`** —encolar la respuesta hasta que el cliente vuelva a escribir, o escalar a un humano— antes de que exista ninguna célula sobre canal oficial, aunque sobre canal propio el caso no se dispare nunca. Una política escrita cuando el fallo no ocurre se diseña con calma; escrita el día que ocurre, se improvisa.

#### B. Requisitos No Funcionales (NFR)
| ID | Categoría | Requisito Técnico |
| :--- | :--- | :--- |
| **NFR-01** | Eficiencia | **Presupuesto de línea base: ≤ 80 MB de RAM por célula en reposo** sobre canal propio (núcleo Rust + sidecar Go, que añade unos 15-30 MB). Como el sidecar es permanente, los 80 MB dejan de ser un sobrecoste transitorio y pasan a ser la línea base del producto. Una célula sobre canal oficial no lleva sidecar y su objetivo sigue siendo **< 50 MB**. **La cifra no está validada bajo carga sostenida** (ver nota). |
| **NFR-02** | Disponibilidad *(solo en células sobre canal oficial)* | Tasa nula (0%) de errores HTTP 502/503 expuestos hacia la WAN de Meta durante suspensiones o reactivaciones. |
| **NFR-03** | Latencia | Conmutación interna de base de datos de conocimiento inferior a 10 milisegundos. |
| **NFR-04** | Seguridad *(solo en células sobre canal oficial)* | Cifrado forzoso HTTPS TLS v1.2/v1.3 gestionado automáticamente vía Caddy (On-Demand TLS), si la entrada pública elegida termina el TLS en el propio servidor. |
| **NFR-05** | Seguridad | Aislamiento estricto de almacenamiento: Un contenedor no puede mapear ni acceder al volumen de datos de otra célula. |

**Nota sobre NFR-01 — el presupuesto de memoria es hoy una estimación de diseño, no una medida.** Los 80 MB se han fijado por cálculo, sin ninguna observación bajo carga sostenida. La obligación pendiente es convertirlos en un **objetivo medido**: límites de `cgroup` declarados por contenedor de la célula (núcleo y sidecar) y una **prueba de carga sostenida** que hoy no figura entre los criterios de aceptación de este documento —la prueba de carga existente ejercita el control de admisión con una ráfaga, no el consumo a lo largo del tiempo—.

De ello se sigue que **el techo real de células por servidor es desconocido hasta medirlo**. Dividir 8 GB entre 80 MB es aritmética, no capacidad. Además, es probable que el cuello de botella no sea la memoria sino la **CPU y la E/S**: N websockets simultáneos con criptografía Signal, cada uno con su sidecar Go y su motor SQLite, sobre un i7 de diez años. Cualquier compromiso sobre el número de células admisibles queda como **decisión pendiente hasta que exista la medición**.

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
[Fase A — canal propio (whatsmeow), permanente]
WhatsApp <--websocket saliente--> [Sidecar Go: whatsmeow]
                                          |
                                    IPC / socket local
                                          |
                              [Núcleo Rust: ChannelAdapter]
                                          |
                           GCRA -> Presupuesto LLM -> RAG -> sessions.db

Una célula sobre canal propio = 2 contenedores (núcleo + sidecar) con red local y volumen
compartidos. El sidecar acompaña a la célula durante toda su vida.

[Fase B — canal oficial (Cloud API), adicional]
Meta Cloud API --webhook HTTPS--> [Entrada pública (ADR)] --> [Núcleo Rust: ChannelAdapter]
                                          |
                           GCRA -> Presupuesto LLM -> RAG -> sessions.db

Una célula sobre canal oficial = 1 contenedor (núcleo), sin sidecar. Ambos tipos de célula
conviven en el mismo servidor y bajo el mismo orquestador.
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
2. **Reconexión del canal:** en la Fase A, el sidecar restablece la sesión whatsmeow desde sus credenciales persistidas, sin necesidad de volver a escanear el QR, **antes** de que la readiness pueda confirmarse. En la Fase B, un **PATCH a la Caddy Admin API** conmuta de la respuesta estática al `reverse_proxy` solo tras la primera confirmación positiva de salud.
3. **Readiness Polling local:** La CLI interroga al endpoint interno `http://{IP_DOCKER}/health/ready` cada 100ms. El contenedor solo responde 200 OK tras comprobar que las conexiones SQLite (`sessions.db` y `knowledge_live.db`) están activas, las estructuras atómicas GCRA cargadas, el puerto de canal enlazado con su adaptador **y la sesión de canal reportada como activa por el sidecar**.

---

### 6. Criterios de Aceptación para QA
* **Prueba de Carga del Canal:** sometimiento de una célula a 100 eventos concurrentes por el puerto de canal (Fase A: inyectados en el stream normalizado; Fase B: peticiones simulando la API de Meta). El sistema debe activar el control de admisión GCRA, descartar el exceso —devolviendo HTTP 200 rápido cuando exista petición que contestar— y el uso de memoria RAM no debe incrementarse en más del 15% del consumo base.
* **Prueba de Resiliencia de Sesión (Fase A):** reiniciar los contenedores de una célula y verificar que el sidecar restablece la sesión whatsmeow desde las credenciales persistidas, sin re-emparejamiento manual. Tras un reinicio **desacompasado de ambos procesos, en cualquiera de los dos órdenes**: cero eventos perdidos y cero eventos procesados por duplicado, sostenido por el outbox durable del sidecar y la deduplicación del núcleo.
* **Prueba de Recuperación de Sesión (Fase A):** restaurar una célula desde sus respaldos —las **cuatro** bases: `sessions.db`, `knowledge_live.db`, el almacén de identidad del adaptador y el `sqlstore` del sidecar— sobre un entorno limpio. La prueba **solo se supera si el bot reconecta al canal y responde a un mensaje real**; recuperar los ficheros con la sesión muerta cuenta como fallo. La prueba exige sidecar y canal real, de modo que se ejecuta en la etapa A-3; la etapa A-2 entrega el procedimiento, el runbook con su bifurcación y el contrato IPC de la copia del `sqlstore`, verificados contra el adaptador simulado.
* **Prueba de Resiliencia del Enlace TLS (Fase B):** bloquear artificialmente el Hairpin NAT del router local. Si la entrada pública elegida termina el TLS en el propio Caddy, el script de orquestación debe completar el onboarding con éxito mediante la bandera `--resolve` forzada a nivel de socket. Si el TLS termina en el edge, este criterio queda sin objeto y se sustituye por la verificación del túnel.
* **Prueba de Consistencia en Modo WAL:** ejecutar un intercambio de conocimiento mientras se procesan 20 lecturas RAG simultáneas. El sistema no debe arrojar excepciones de tipo `SQLITE_BUSY` ni dejar huérfanos archivos `.db-wal` o `.db-shm`.
