# HexCell Orchestrator

HexCell es un motor orquestador multi-célula (*multi-tenant*) de ultra alta eficiencia escrito en **Rust**, diseñado para desplegar y administrar bots automatizados de WhatsApp para microempresas locales. La arquitectura está optimizada estructuralmente para ejecutarse en servidores locales con severas restricciones de hardware (procesadores heredados de consumo doméstico y baja densidad de memoria RAM) sin comprometer la estabilidad, el aislamiento de datos ni el presupuesto financiero de las APIs de lenguaje natural.

La unidad desplegable por cliente se denomina **célula**. En la CLI y en el código el sustantivo es `cell`.

> **Estado del proyecto:** fase de diseño. Ver [docs/PRD.md](docs/PRD.md) (requisitos) y [docs/STATUS.md](docs/STATUS.md) (decisiones definidas y pendientes).

---

## 🧭 Estrategia de dos canales que conviven

Las dos fases no son una secuencia: son **dos canales vivos a la vez**, y cada célula se despliega sobre el que le corresponde.

* **Fase A — Canal propio en producción.** Se usa la biblioteca **whatsmeow** (Go, protocolo WhatsApp Web) sobre un **websocket saliente**: sin webhook, sin IP pública, sin Caddy y sin TLS entrante. Es el **canal por defecto y permanente**, con clientes de pago reales encima; no tiene límite de dos pilotos ni fecha de caducidad. `piloto-01` y `piloto-02` son las dos primeras células, no el alcance total. Docker desde el primer día. Los riesgos —baneo del número (**estructural**: Meta detecta la biblioteca por su huella de protocolo, y ninguna medida de comportamiento lo elimina), roturas de protocolo, mantenimiento con bus factor 1 y violación de los ToS de WhatsApp— se asumen de forma consciente y permanente, y están documentados en el PRD.
* **Compuertas de riesgo.** La compuerta del tercer cliente **queda derogada** (28 de julio de 2026), igual que la regla de que no se comercializa sobre canal no oficial. Lo que disciplina el crecimiento es un **techo duro de cartera** mientras el canal propio sea el único y un **umbral de incidentes que congela altas**; ambos valores son decisiones de negocio pendientes.
* **Fase B — Canal oficial adicional.** Meta Cloud API con webhooks, para las células que lo requieran. Se activa **cuando aparece un cliente que lo justifique** —típicamente una empresa medianamente grande que pueda asumir el alta y el coste—, no en una fecha ni con un número de clientes. **Se suma al canal propio; no lo sustituye ni retira ningún sidecar.** Aquí se descongelan Caddy, los subdominios, el On-Demand TLS y el Embedded Signup. La entrada pública está **pendiente de ADR**: Cloudflare Tunnel en capa gratuita (TLS terminado en el edge, sin necesidad del handshake anti-Hairpin) o VPS de ~3 USD/mes con WireGuard (TLS terminado en el propio Caddy, conservando la arquitectura original).

La pieza que hace posible que ambos canales convivan sin reescribir el producto es el **puerto de canal** (`ChannelAdapter`, FR-12): un trait del núcleo Rust que normaliza eventos entrantes, envío, identidad de conversación y acuses, de modo que sumar un canal sea sumar un adaptador.

Detalle completo en [docs/PRD.md](docs/PRD.md) (sección "Estrategia de Canal por Fases") y en el [plan de implementación](docs/plan/README.md).

---

## 🛡️ Pilares de la Arquitectura de Software

### 1. Inferencia Externa y Hardware Local Protegido
El hardware local no procesa modelos de lenguaje grande (LLMs). Toda la inferencia semántica y generativa se delega mediante conexiones HTTPS salientes hacia infraestructuras externas de bajo costo (Gemini Flash, Groq u OpenRouter). El motor nativo en Rust limita su consumo a la lógica de control, enrutamiento, consumo de API y consultas vectoriales locales, con un **presupuesto de línea base de ≤ 80 MB de RAM por célula sobre canal propio** (núcleo Rust más el sidecar Go de whatsmeow, que es permanente) y **< 50 MB en una célula sobre canal oficial**, que no lleva sidecar. Esa cifra **no está validada bajo carga sostenida**: es una estimación de diseño que debe convertirse en un objetivo medido con límites de `cgroup` y una prueba de carga, y hasta entonces **el techo real de células por servidor es desconocido** (ver la nota de NFR-01 en el PRD).

### 2. Persistencia Segregada en SQLite Dual y Aislamiento WAL
Para evitar la contención de escrituras concurrentes y el bloqueo de transacciones (`SQLITE_BUSY`) al interactuar con servicios de red de alta latencia, cada célula corre en un contenedor Docker aislado equipado con dos bases de datos físicas independientes:
* `sessions.db`: Almacena el historial y el estado conversacional. Modo lectura/escritura continua en caliente. Nunca guarda identificadores de transporte crudos: el puerto de canal los mapea a identificadores internos.
* `knowledge_live.db`: Contiene las reglas de negocio, catálogos y embeddings vectoriales para el motor de Recuperación Aumentada por Generación (RAG). Se opera en modo estrictamente de lectura durante producción.

### 3. Pipeline de Actualización Inmutable y Cambio Atómico (Shadow DB)
Las actualizaciones de conocimiento se gestionan en una base de datos en sombra (`knowledge_staging.db`) aislando las llamadas por lotes a APIs de embeddings. Una vez validada la integridad estructural y semántica del índice, se ejecuta la secuencia atómica por épocas:
1. Sellar y colapsar el WAL de staging vía `PRAGMA wal_checkpoint(TRUNCATE);`.
2. Renombrar el archivo a una época inmutable (`knowledge_epoch_N.db`).
3. Reasignar de forma atómica el enlace simbólico del sistema de archivos y actualizar el pool de conexiones en memoria empleando `ArcSwap`.
4. Ejecutar un drenaje controlado asíncrono (`Graceful Drain`) de las conexiones del pool obsoleto, erradicando corrupciones o bloqueos de descriptores de archivos (`-wal` y `-shm`).

### 4. Puerto de Canal: la Frontera de Coexistencia
El núcleo Rust no conoce ningún transporte de WhatsApp. Toda integración vive detrás del trait `ChannelAdapter`, que normaliza el evento entrante canónico (remitente, conversación, contenido, marca temporal e identificador de deduplicación), el envío `send(conversation_id, contenido)`, la identidad de conversación mapeada a un identificador interno, y los acuses (`sent`/`delivered`/`read`/`failed`). Un sub-trait opcional cubre el ciclo de vida de sesión —emparejamiento por QR o código y persistencia de credenciales— que solo implementan los adaptadores no oficiales.

El puerto no es la frontera de una migración: sostiene **dos adaptadores vivos a la vez** en células distintas del mismo servidor. Se abstrae hacia el caso más restrictivo (la Cloud API), con una distinción que importa: **el tipo admite el resultado restrictivo, pero la política de cada adaptador decide si lo produce**. El adaptador del canal propio nunca devuelve `FueraDeVentana` porque su transporte no impone ninguna ventana de 24 horas, y fabricarla sería degradar el producto sin motivo.

En una célula sobre canal propio, el adaptador whatsmeow corre como **sidecar Go** junto al núcleo Rust: cada célula son dos contenedores que comparten red local y volumen, comunicados por IPC sobre socket local. El sidecar añade unos 15-30 MB de RAM, y ese coste es **permanente**: no es andamiaje que desaparezca más adelante, sino parte de la línea base de toda célula sobre canal propio.

### 5. Defensa Perimetral y Control Presupuestario (GCRA)
El control de admisión **GCRA (Generic Cell Rate Algorithm)** se aplica sobre el **flujo normalizado del puerto de canal**, no sobre HTTP, de modo que el mecanismo sea idéntico en ambas fases:
* Intercepta los eventos que exceden el límite de tasa antes de alocar memoria en el heap.
* En la Fase B, responde además con un código **HTTP 200 OK sintético e inmediato** a Meta (patrón *Fast-Reject*), anulando las tormentas de reintentos automáticos generadas por la API Graph cuando recibe códigos de error estándar (429/503). En la Fase A no hay petición que contestar: el exceso simplemente se descarta y se registra.
* Garantiza el control presupuestario mediante un sistema de contabilidad de cuotas financieras en dos fases: Reserva Previa (*Pre-Execution Hold*) antes de invocar al LLM y Conciliación Exacta (*Post-Execution Reconcile*) posterior a la recepción de los metadatos de tokens.

---

## 🛠️ Flujo de Onboarding e Inyección de Red (Anti-Hairpin NAT) *(Fase B)*

> Esta sección describe el alta sobre el canal oficial y **queda congelada hasta que aparezca un cliente que justifique el canal oficial** —típicamente una empresa medianamente grande que pueda asumir el alta y su coste—. Ya no la dispara ningún número de clientes: la compuerta del tercer cliente está derogada. El alta de una célula sobre canal propio no usa nada de lo que sigue: se resuelve con un emparejamiento por QR o código contra la sesión whatsmeow del sidecar.
>
> **Opción preferente a evaluar cuando llegue ese momento: el [modo coexistencia](https://developers.facebook.com/docs/whatsapp/embedded-signup/custom-flows/onboarding-business-app-users/) de Meta.** Permite que un mismo número funcione a la vez en la app de WhatsApp Business del móvil y en la Cloud API, sincronizando 180 días de historial y contactos, y el integrador recibe por webhook (`smb_message_echoes`) lo que el dueño responde a mano desde su app. Resuelve de un golpe la interfaz de intervención humana y desmonta el argumento de que el cliente pierde su bandeja del móvil. Limitaciones: exige Embedded Signup de un Solution Partner o Tech Provider (no hay ruta de Cloud API directa), 20 mensajes por segundo fijos, sin grupos, sin mensajes efímeros, sin vista única, sin ubicación en vivo, sin listas de difusión y sin catálogo ni pedidos por API.

El proceso de alta de una nueva microempresa utiliza el flujo **Meta Embedded Signup** bajo una única aplicación del proveedor para una experiencia de usuario sin fricción técnica. El aislamiento de red se logra mediante la propiedad `override_callback_uri` de la API Graph, enviando el tráfico de cada WABA directamente al subdominio de la célula (`https://clienteX.midominio.com/webhook`).

Para asegurar el apretón de manos síncrono inicial frente a Meta, el script de orquestación mitiga la ausencia de Hairpin NAT en enrutadores locales forzando la resolución del socket del cliente HTTP hacia la interfaz de loopback local, enviando explícitamente el SNI y el encabezado Host del dominio público:

```bash
# Handshake sintético ejecutado por el orquestador local para forzar el desafío ACME en Caddy
curl --resolve cliente1.midominio.com:443:127.0.0.1 \
  -v "https://cliente1.midominio.com/webhook?hub.mode=subscribe&hub.verify_token=CRYPTO_TOKEN&hub.challenge=handshake_test"
```

Este método garantiza de manera matemática que la Autoridad Certificadora (Let's Encrypt/ZeroSSL) validó externamente el entorno WAN del servidor local antes de autorizar la suscripción definitiva en la API Graph de Meta.

**Este mecanismo solo aplica si la entrada pública elegida termina el TLS en el propio servidor** (opción VPS + WireGuard). Con Cloudflare Tunnel, el TLS termina en el edge y el handshake sintético deja de ser necesario. La decisión está pendiente de ADR.

---

## 💻 Manual de Operación de la CLI de Administración

Estado (2026-09-14): la gramática de los seis subcomandos `cell` existe en `hexcell-admin` desde HEX-074-c (tarea 10 de A-6), con validación de argumentos y modo `--simular`; sin `--simular` cada subcomando devuelve todavía `NoImplementadoTodavia` (código 3), porque las operaciones reales contra Docker llegan con las tareas 11-15. Actualización 2026-09-21: `cell pause` y `cell unpause` son reales desde HEX-080 (tarea 11); `cell terminate`, `cell rebind`, `cell list` y `cell status` siguen devolviendo `NoImplementadoTodavia` sin `--simular` hasta las tareas 12-14.

La suite de administración central compila como un binario nativo que interactúa directamente con el socket Unix de Docker (`/var/run/docker.sock`). En la Fase B interactúa además con la API local de administración en memoria de Caddy (`http://localhost:2019`).

### 1. Suspender Temporalmente una Célula (Falta de pago / Pausa)

Garantiza la liberación inmediata de RAM y CPU en el hardware local sin inyectar códigos de error de enrutamiento hacia el canal.

```bash
./hexcell-admin cell pause --id <cell_id>
```

*Mecanismo Interno (Fase A):* detiene el sidecar, con lo que el websocket saliente se cierra y la entrada de mensajes cesa por construcción; a continuación envía una señal `SIGTERM` al contenedor del núcleo con un margen de 30 segundos para drenar lecturas RAG en vuelo y hacer flush del WAL a disco. No interviene Caddy. Desde HEX-080 (2026-09-21) la CLI pide la parada sin plazo propio: el margen de 30 segundos lo fija el `stop_grace_period` de `deploy/cell.compose.yml`, única fuente de verdad de la gracia.

*Mecanismo Interno (Fase B):* aplica un parche en Caddy para sustituir el `reverse_proxy` por un `static_response_handler` (HTTP 200 instantáneo) y solo después emite el `SIGTERM`, evitando cualquier 502 hacia Meta. Requiere el parámetro `--domain cliente1.midominio.com`.

### 2. Reactivar una Célula

Restaura la producción asegurando que el backend está completamente listo antes de admitir tráfico real de mensajería.

```bash
./hexcell-admin cell unpause --id <cell_id>
```

*Mecanismo Interno:* inicia los contenedores de la célula de forma aislada. En la Fase A, el sidecar reanuda primero la sesión whatsmeow desde sus credenciales persistidas, sin re-escanear el QR: esa reanudación es condición previa de la readiness, no su consecuencia. La CLI ejecuta un bucle de *Readiness Polling* local hacia el endpoint `GET /health/ready` del contenedor cada 100ms, que responde 200 OK solo tras comprobar de extremo a extremo la vitalidad de sus pools de persistencia SQLite, el enlace del puerto de canal **y la sesión de canal activa**. En la **Fase B**, Caddy conmuta el tráfico de la respuesta estática al proxy inverso únicamente tras la primera confirmación positiva de salud.

### 3. Eliminar Definitivamente una Célula

Remoción destructiva limpia y desvinculación perimetral.

```bash
./hexcell-admin cell terminate --id <cell_id>
```

*Mecanismo Interno (Fase A):* cierra la sesión whatsmeow (desvinculando el dispositivo del número), ejecuta el drenaje por `SIGTERM` de ambos contenedores y destruye los volúmenes de disco locales de manera física (`std::fs::remove_dir_all`), incluidas las credenciales de sesión.

*Mecanismo Interno (Fase B):* invoca además la desasociación del webhook en la API Graph de Meta y purga de forma atómica la regla de enrutamiento y la memoria caché de certificados en el servidor web Caddy. Requiere los parámetros `--domain` y `--waba`.

### 4. Sustituir el Número de una Célula *(Fase A)*

Re-empareja una célula existente con un número distinto conservando su historia. Es la salida técnica de un baneo permanente, y no un alta nueva: la célula, su conocimiento y la memoria del bot por contacto sobreviven a la sustitución.

```bash
./hexcell-admin cell rebind --id <cell_id> --motivo "<motivo>"
```

*Mecanismo Interno (Fase A):* exige **confirmación explícita** por tratarse de una operación destructiva sobre la identidad de canal de la célula, con la misma exigencia que `cell terminate`. Deja la célula en **pausa de envío** hasta que el emparejamiento con el número nuevo queda confirmado, de modo que no pueda intentar responder sin sesión. Descarta el `sqlstore` del sidecar —corresponde a un dispositivo que ya no existe en el servidor de WhatsApp, y restaurarlo desde respaldo es inútil— y **conserva intactos** `sessions.db`, `knowledge_live.db` y el almacén de identidad del adaptador, que es donde viven la identidad de conversación y la lista de exclusión (STOP): por eso el mismo contacto sigue cayendo en el mismo hilo tras la sustitución. Cierra anotando la sustitución de forma auditable, con el número anterior, la fecha absoluta y el motivo.

Este comando no existe en la Fase B: nace de la operación del canal propio y no interviene Caddy ni la API Graph de Meta. Cuándo **procede** sustituir el número —y cuándo no— lo decide el runbook de baneo, no la CLI.

### 5. Configuración por célula como archivos

Entregado el 2026-09-21 con HEX-081 (tarea 22 de A-6; `adr-0038` fechado el 2026-09-19).

La configuración de cada célula vive en **archivos versionables en git**: `deploy/celula.defecto.env.ejemplo` contiene los valores compartidos y `deploy/celula.superposicion.env.ejemplo` muestra un *overlay* por célula. Se renderizan con:

```bash
hexcell-admin config render --defecto deploy/celula.defecto.env.ejemplo \
  --superposicion deploy/celula.superposicion.env.ejemplo --salida celula.env
```

Los archivos contienen **solo parámetros no secretos**; todo secreto sigue viajando por variables de entorno. Una clave desconocida o un valor inválido falla cerrado y no crea ni modifica la salida. Los overlays con valores reales son datos del cliente y se versionan únicamente en el repositorio privado del operador; este repositorio contiene solo ejemplos con marcadores. `hexcell-admin` renderiza el entorno de la plantilla de arranque: el binario de la célula **no gana un segundo lector de configuración**. `--simular` valida y muestra el número de claves sin escribir.

### 6. Composición de la célula

La célula se materializa como dos contenedores —núcleo y sidecar— descritos en `deploy/cell.compose.yml`, parametrizada por célula con las variables de `deploy/celula.env.ejemplo` (nota de uso en `docs/plantilla-celula.md`). Las banderas de endurecimiento —`read_only`, `cap_drop: [ALL]`, `no-new-privileges` y el `tmpfs` de la ruta de escritura temporal— están impuestas en esa plantilla y se verifican mecánicamente con la guarda `deploy/verificar_endurecimiento.sh` (HEX-070, 2026-09-11). La propagación ordenada de `docker stop` con margen de 30 s —`STOPSIGNAL SIGTERM` en ambos Dockerfiles y `stop_grace_period` en ambos servicios— se verifica mecánicamente con `deploy/verificar_senales.sh` (probada por mutación, en CI) y en vivo, con contenedores reales, con `deploy/verificar_apagado_ordenado.sh` (manual, HEX-075, 2026-09-13). El aislamiento entre células —red y volumen propios, sin cruce de volumen ni de red, sin socket IPC ajeno y sin puertos publicados al host— se verifica mecánicamente con `deploy/verificar_aislamiento_estatica.sh` (probada por mutación, en CI) y en vivo, levantando dos células reales, con `deploy/verificar_aislamiento.sh` (manual, HEX-076, 2026-09-13). Los límites de recursos por contenedor —memoria, CPU y descriptores de archivo, parametrizados por célula con los valores de `deploy/celula.env.ejemplo`— se verifican mecánicamente sobre el YAML resuelto con `deploy/verificar_limites.sh` (probada por mutación, en CI; valores provisionales pendientes de la medición de la tarea 16 del plan de la etapa A-6). Esa medición —memoria agregada de la célula compuesta desde cgroup v2 en reposo y bajo un generador declarado, más el peso de ambas imágenes— se instrumenta en vivo con `deploy/medir_memoria_y_imagenes.sh` (manual, HEX-079, 2026-09-19); los valores de referencia se registran en `docs/plantilla-celula.md`.

### 7. Estado y listado de células

Entregado el 2026-09-22 con HEX-083 (tarea 14 de A-6; `adr-0039` fechado el 2026-09-22).

El almacén del plano de control persiste el estado de cada célula en SQLite, con la ruta configurable mediante la variable de entorno `HEXCELL_ADMIN_ALMACEN` (valor por omisión `/var/lib/hexcell-admin/plano_de_control.db`). Si el directorio padre no existe, el comando falla con un diagnóstico claro; `hexcell-admin` nunca crea ese directorio.

```bash
hexcell-admin cell status --id <cell_id>
```

`cell status` cruza las tres fuentes (almacén, Docker, sonda de salud) y reporta el estado almacenado, el estado Docker de núcleo y sidecar, la salud (`listo`, `no_listo` o `inalcanzable`), el historial de sustituciones y los códigos de discrepancia detectados:

* **DISC-01:** el almacén indica `en_ejecucion` pero un contenedor no está corriendo.
* **DISC-02:** el almacén indica `suspendida` pero un contenedor está corriendo.
* **DISC-03:** los contenedores corren pero `/health/ready` no confirma disponibilidad.
* **DISC-04:** el almacén tiene una fila pero los contenedores no existen en Docker.
* **DISC-05:** los contenedores existen en Docker pero el almacén no tiene fila.

El comando sale con código 0 si no hay discrepancias, o código 1 si hay al menos una. No reporta ratio de acuses ni ventana de silencio (eso es la tarea 20).

```bash
hexcell-admin cell list
```

`cell list` imprime la unión de células del almacén y de Docker, con el estado almacenado (o `sin_fila` si la célula sólo existe en Docker) y el estado Docker de núcleo y sidecar. No sondea la salud. Sale con código 0 una vez producida la lista.

Ambos comandos son de sólo lectura **por construcción**: abren la base con el descriptor de sólo lectura de SQLite, no aplican migraciones y no crean el archivo, así que un `HEXCELL_ADMIN_ALMACEN` que apunte a una ruta inexistente falla con un diagnóstico en vez de dejar una base nueva detrás de una consulta. Una discrepancia tampoco se repara: DISC-05 se reporta y la fila no se crea.

Una fuente que **falla** no es una discrepancia: si `docker inspect` devuelve un error que no sea «no encontrado» —500 del demonio, socket inalcanzable, respuesta malformada—, `cell status` nombra la fuente Docker y el contenedor en el diagnóstico y sale con código 1 sin emitir ningún `DISC-0N`, porque en ese caso no se sabe si los contenedores existen.
