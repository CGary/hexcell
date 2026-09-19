# Bitácora de descartes

> Registro de lo que se consideró y **no** se hizo. Última actualización: 2026-09-19 (D-55).

## Para qué sirve este documento

Los ADR registran lo que se decidió. Este documento registra lo contrario: **las opciones que se
estudiaron y se descartaron, y por qué**. Existe porque las ideas muertas vuelven. Alguien —el propio
dueño dentro de seis meses, o una instancia nueva de Claude Code— propone algo que suena razonable
sin saber que ya se evaluó, se rechazó y hay evidencia de por qué. Sin este registro, ese debate se
repite entero cada vez.

**Antes de proponer un cambio de rumbo, un atajo o una técnica nueva, búscala aquí.**

Cada entrada declara además **qué tendría que cambiar para reabrirla**, y ese campo es el que impide
que la bitácora se convierta en dogma. Un descarte que se apoya en un hecho externo —un precio, la
política de un tercero, una limitación técnica— **caduca cuando ese hecho cambia**. Un descarte que
se apoya en un principio de diseño, no.

### Reglas de uso

1. **Una entrada por descarte, con identificador correlativo `D-NN`.** La numeración es fuente de
   verdad: nunca se reutiliza ni se reordena.
2. **Las entradas no se editan ni se borran.** Si un descarte se reabre, se añade una línea
   **`REABIERTO`** al final de su entrada, con la fecha y el ADR que lo justifica. La historia se
   conserva íntegra: un descarte revertido enseña más que un descarte desaparecido.
3. **Este documento no decide nada.** La decisión vive en el ADR o en el PRD; aquí se registra el
   rastro. Ante contradicción, manda la jerarquía documental de `CLAUDE.md`.
4. **Un descarte sin motivo escrito es un descarte perdido.** Si la razón no se puede reconstruir, se
   escribe *"sin motivo registrado"* en vez de inventarlo — es información honesta y señala una
   deuda.

### Índice por idea

| ID | Idea descartada | Estado |
| :--- | :--- | :--- |
| [D-01](#d-01) | Estrategia de dos fases con compuerta en el tercer cliente | Reabrible si cambia un hecho externo |
| [D-02](#d-02) | Migrar al canal oficial desde el cliente cero | Mecanismo previsto, no reabrir |
| [D-03](#d-03) | Plan mono-canal: Cloud API y webhooks desde el día 1 | A determinar |
| [D-04](#d-04) | Supuesto: "el transporte del canal oficial cuesta ≈ 0" | Reabrible si cambia un hecho externo |
| [D-05](#d-05) | Supuesto: "el canal oficial obliga a perder la bandeja del móvil" | Incorporado, no reabrir |
| [D-06](#d-06) | Supuesto: "el indicador de 'escribiendo' es folclore" | Corregido, no reabrir |
| [D-07](#d-07) | Baileys como biblioteca del canal propio | Reabrible si cambia un hecho externo |
| [D-08](#d-08) | Prácticas anti-baneo rechazadas en bloque | Principio de diseño, no reabrir |
| [D-09](#d-09) | Firma anticipada del adaptador de Cloud API en la etapa A-1 | Principio de diseño, no reabrir |
| [D-10](#d-10) | Vía de escape "excepción documentada como deuda" en B-1 | Principio de diseño, no reabrir |
| [D-11](#d-11) | Respaldos aplazados al endurecimiento final | Principio de diseño, no reabrir |
| [D-12](#d-12) | Devolver 429/503 a Meta bajo sobrecarga | Reabrible si cambia un hecho externo |
| [D-13](#d-13) | Encolar mensajes ante `FueraDeVentana` | A determinar |
| [D-14](#d-14) | Nombres anteriores: ZeroClaw, `hexcell-cell`, "inquilino" | Cerrado |
| [D-15](#d-15) | Guardar el mapeo de identidad dentro del `sqlstore` del sidecar | Principio de diseño, no reabrir |
| [D-16](#d-16) | Guardar el identificador de transporte en `sessions.db` | Principio de diseño, no reabrir |
| [D-17](#d-17) | `tracing` + `tracing-subscriber` con capa JSON para el registro estructurado | Principio de diseño, no reabrir |
| [D-18](#d-18) | `tokio-util::CancellationToken` para el apagado ordenado | Principio de diseño, no reabrir |
| [D-19](#d-19) | API de respaldo en línea de `rusqlite` (`Connection::backup`) frente a `VACUUM INTO` | Principio de diseño, no reabrir |
| [D-20](#d-20) | Planificador de respaldo dentro del propio proceso de la célula | Principio de diseño, no reabrir |
| [D-21](#d-21) | Usar trybuild como mecanismo de prueba compile-failure | Reabrible si cambia semántica de rustc |
| [D-22](#d-22) | Respaldo concurrente sin pausa previa (steal-and-exit con reconexión automática) | Principio de diseño, no reabrir |
| [D-23](#d-23) | Disparador de respaldo en el propio proceso del núcleo por señales/env | Principio de diseño, no reabrir |
| [D-24](#d-24) | Generalizar la orden de respaldo del `sqlstore` con un discriminador de almacén para `identidad.db` | Principio de diseño, no reabrir |
| [D-25](#d-25) | Centralizar las bases de datos operativas (un RDBMS único multi-inquilino para el camino caliente) | Principio de diseño, no reabrir |
| [D-26](#d-26) | rqlite / libSQL sqld en el camino caliente (los almacenes operativos del bot por HTTP) | Principio de diseño, no reabrir |
| [D-27](#d-27) | Alternativas descartadas para la inferencia HTTPS (reqwest, aws-lc-rs, backoff exponencial, reintentar 429, noveno crate) | Principio de diseño, no reabrir |
| [D-28](#d-28) | Alternativas descartadas para el puerto de embeddings y adaptador OpenRouter (compartir parser de chat, zipping posicional, reserva por fragmento/ingesta, elevar timeout, base64, pseudo-conversación) | Principio de diseño, no reabrir |
| [D-29](#d-29) | Alternativas descartadas para la conmutación atómica de épocas (cerrojo en pool, unlink+symlink, copia en caliente, reinicio de proceso) | Principio de diseño, no reabrir |
| [D-30](#d-30) | Alternativas descartadas para el drenaje de la época superseída (notificación por Condvar, cierre forzado, remediación por borrado, sobrecarga de variable de apagado) | Principio de diseño, no reabrir |
| [D-31](#d-31) | Alternativas descartadas para la reversión de épocas y guardas de fallo silencioso (re-acuñación de épocas, comodín en partición semántica, guarda de enlace colgante en solo lectura, fallback silencioso de ruta canónica) | Principio de diseño, no reabrir |
| [D-32](#d-32) | Escribir la marca de sospechosa después de reasignar el enlace simbólico | Principio de diseño, no reabrir |
| [D-33](#d-33) | Serializar el binario de tests con `--test-threads=1` para tapar la carrera del entorno del proceso | Principio de diseño, no reabrir |
| [D-34](#d-34) | Mover los tests que mutan el entorno a un binario de integración aparte | Principio de diseño, no reabrir |
| [D-35](#d-35) | Alternativas descartadas al escribir la prueba de estrés de conmutación de época (anchura de pool por omisión, correr dentro de la batería por defecto, contrastar NFR-03 contra el intervalo ancho, tolerancia en la aserción de descriptores) | Principio de diseño, no reabrir |
| [D-36](#d-36) | Medir la simultaneidad de las lecturas con un medidor de pico de hilos alrededor de `recuperar_contexto` | Reabrible si cambia un hecho del árbol |
| [D-37](#d-37) | Afirmar el muro estricto de NFR-03 (< 10 ms) sobre `duracion_de_conmutacion_ms` dentro de la prueba de estrés | Reabrible si cambia un hecho del árbol |
| [D-38](#d-38) | Añadir exclusión mutua real entre `respaldar_en` y `iniciar_promocion`/`promover_epoca` (cerrojo o bandera compartida de promoción consultada desde el respaldo) | Principio de diseño, no reabrir |
| [D-39](#d-39) | Serde / Serialize / Deserialize en `hexcell_storage::DocumentoDeIngesta` | Principio de diseño, no reabrir |
| [D-40](#d-40) | `spawn_blocking` para ejecutar `ejecutar_ingesta` desde el listener administrativo | Reabrible si cambia un hecho del árbol |
| [D-41](#d-41) | `ArcSwap` o `tokio::sync::Mutex` para la compuerta del estado administrativo de ingesta (`EstadoDeAdmin`) | Principio de diseño, no reabrir |
| [D-42](#d-42) | Variables de entorno adicionales para el texto de la sonda semántica y parámetros de fragmentación de ingesta | Reabrible si cambia un hecho del proyecto |
| [D-43](#d-43) | Extraer a `abrirRecursosDeArranque` el cableado de `main()` posterior al buzón | Reabrible si cambia un hecho del árbol |
| [D-44](#d-44) | Liberar los recursos ya abiertos cuando el arranque falla a mitad de camino | Reabrible si cambia un hecho del proyecto |
| [D-45](#d-45) | Convertir la clasificación del acuse de entrega/lectura en un mensaje IPC (`acuse_envio` u otro) | Reabrible si aparece un consumidor fuera del proceso |
| [D-46](#d-46) | Lista de LRU real (`container/list`) para el desalojo de `contactos` y `correlaciones` de `sidecar/internal/metricas` | Principio de diseño, no reabrir |
| [D-47](#d-47) | Declarar la señal de parada con la clave `stop_signal:` de `deploy/cell.compose.yml` en lugar de la directiva `STOPSIGNAL` de cada Dockerfile | Reabrible si aparece un caso donde la señal de parada deba variar por célula |
| [D-48](#d-48) | Ejercer los vectores de cruce de red y de socket IPC del script en vivo de aislamiento con `docker exec` directo dentro de los contenedores reales | Reabrible si se reintroduce un intérprete en las imágenes finales |
| [D-49](#d-49) | Probar que el socket IPC de una célula no es alcanzable desde otra comparando únicamente el dispositivo de archivos (`stat -c %d`) | Reabrible solo si los volúmenes pasaran a sistemas de archivos separados |
| [D-50](#d-50) | Promedio móvil de latencias a través de múltiples acuses en `sidecar/internal/metricas`, en lugar de la última observada por acuse | Principio de diseño, no reabrir |
| [D-51](#d-51) | Vigilancia externa: subcomando de hexcell-admin, binario/crate nuevo, ping gateado a la salud de la célula, y reintento/backoff local | Principio de diseño, no reabrir |
| [D-52](#d-52) | Forma explícita `soft`/`hard` de `ulimits.nofile` y comprobación de "campo presente" en el guardia de límites de recursos (HEX-078) | Principio de diseño, no reabrir |
| [D-53](#d-53) | Bibliotecas externas de análisis de argumentos para `hexcell-admin` (`clap`, `argh`, `pico-args`, `structopt`) | Principio de diseño, no reabrir |
| [D-54](#d-54) | Alerta de bucle de reinicios de contenedores dentro de HEX-077-b, sin productor de señal que la alimente | Reabrir si se construye un observador de reinicios |
| [D-55](#d-55) | Reutilizar `crates/hexcell/tests/carga.rs` como generador externo de carga contra la célula compuesta en vivo (HEX-079) | Reabrible si cambia un hecho del árbol |
| D-55 | Parser externo para la configuración de células | Principio de diseño, no reabrir |

---

## Descartes estructurales

### D-01
**Estrategia de dos fases con compuerta en el tercer cliente, y regla "no se comercializa sobre canal
no oficial".**

* **Decidido:** 2026-07-26 (`adr-0008`). **Derogado:** 2026-07-28 (`adr-0014`).
* **Por qué se descartó:** cayó su premisa económica. Primero, llevar cada microempresa al canal
  oficial exige convencerla de montar una WABA y hacerle las gestiones: un coste que recae sobre el
  tiempo del fundador, el recurso más escaso del proyecto, y que **no aparece en ningún diagrama
  técnico**, razón por la que se había subestimado. Segundo, Meta anunció el 1 de julio de 2026 que
  **desde el 1 de octubre de 2026 cobrará también los mensajes de servicio** — justo el tráfico
  solo-respuesta que se daba por gratuito.
* **Registro normativo:** `docs/adr/adr-0014-canal-propio-permanente.md`, `docs/PRD.md` (sección de
  estrategia de canal), `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** *hecho externo mutable, pero solo en parte.* Si Meta
  desmiente o revierte el cobro de mensajes de servicio, decae el segundo motivo. **El primero se
  sostiene solo**: para reabrir la compuerta habría que demostrar que el alta en el canal oficial deja
  de consumir tiempo del fundador por cliente.

### D-02
**Migrar al canal oficial desde el cliente cero, sin etapa de canal propio.**

* **Descartado:** 2026-07-28 (`adr-0014`, alternativa evaluada).
* **Por qué se descartó:** los mismos dos costes de D-01, agravados por pagarse **antes** de tener
  evidencia de que el producto se vende. Durante la evaluación se encontró el **modo coexistencia** de
  Meta, que permite el mismo número en la app del móvil y en la Cloud API a la vez; desmonta el
  argumento de comodidad (ver D-05) pero no los dos motivos económicos, así que no cambió la decisión.
  La coexistencia quedó mandatada como **opción preferente de la segunda etapa**.
* **Registro normativo:** `docs/adr/adr-0014-canal-propio-permanente.md` (sección de alternativas),
  `docs/plan/fase-b-1-canal-oficial.md`.
* **Qué tendría que cambiar para reabrirlo:** *no hace falta reabrirlo.* El mecanismo ya existe: la
  aparición de un cliente que justifique el canal oficial activa la segunda etapa sin revertir nada.

### D-03
**Plan de implementación mono-canal: Cloud API con webhooks, Caddy y TLS entrante desde el día 1, en
ocho etapas, sin sidecar, con presupuesto de menos de 50 MB por "inquilino".**

* **Creado:** 2026-07-26 (commit `6d647d7`). **Descartado:** el mismo día (commit `fa7ef4d`, que
  eliminó **siete** de sus ocho etapas).
* **Por qué se descartó:** **sin motivo registrado.** El commit no lleva cuerpo y ningún documento
  describe qué contenía aquel plan ni qué lo tumbó. La razón reconstruible es validar el negocio sin
  asumir por adelantado los trámites y costes de Meta, pero **es una deducción, no un registro**.
  `docs/plan/fase-a-6-empaquetado-cli.md` alude a "el diseño original" sin describirlo.
* **Registro normativo:** ninguno. **Vive en el historial de git**, en el rango
  `6d647d7..fa7ef4d`. Única excepción: la etapa 4 (conocimiento y Shadow DB) **no se eliminó, se
  renombró** a `docs/plan/fase-a-5-conocimiento-shadow-db.md` — es el único fragmento de aquel plan
  que sobrevive en el árbol actual.
* **Qué tendría que cambiar para reabrirlo:** *a determinar.* El principio que lo sustituyó —validar
  antes de invertir en infraestructura de terceros— se ha reafirmado dos veces (D-01 lo mantuvo
  incluso al invertir el rumbo del canal), pero sin el motivo original escrito no se puede evaluar con
  rigor. **Esta entrada es el mejor argumento para que esta bitácora exista.**

---

## Supuestos invalidados

Un supuesto invalidado es más peligroso que una alternativa descartada: nadie lo debatió, se dio por
cierto y se construyó encima.

### D-04
**Supuesto: "el transporte del canal oficial cuesta aproximadamente 0, porque el bot solo responde y
las respuestas dentro de la ventana de 24 h son gratuitas".**

* **Afirmado:** 2026-07-27. **Invalidado:** 2026-07-28.
* **Por qué se invalidó:** el anuncio de Meta del 1 de julio de 2026 sobre el cobro de mensajes de
  servicio desde el 1 de octubre de 2026, con tarifas publicables hasta el 1 de septiembre de 2026.
  *Estado de la evidencia: confirmado por múltiples BSPs, todavía no reflejado en la página oficial de
  precios de Meta.*
* **Registro normativo:** `docs/STATUS.md` (bloque de corrección fechado), `adr-0014`,
  `docs/plan/fase-b-1-canal-oficial.md`.
* **Qué tendría que cambiar para reabrirlo:** *hecho externo mutable con fecha de comprobación.* Si
  Meta no publica la tarifa antes del 1 de septiembre de 2026, o la desmiente, el supuesto vuelve a
  ser válido. **Es la entrada de esta bitácora con la caducidad más próxima: revísala.**

### D-05
**Supuesto: "adoptar el canal oficial obliga al cliente a perder la bandeja de entrada de la app de
WhatsApp Business en su móvil".**

* **Desmontado:** 2026-07-28.
* **Por qué se invalidó:** existe el **modo coexistencia** oficial de Meta: el mismo número funciona a
  la vez en la app del móvil y en la Cloud API, sincroniza 180 días de historial y contactos, y el
  integrador recibe por webhook (`smb_message_echoes`) lo que el dueño responde a mano desde su app.
  Requiere Embedded Signup de un Solution Partner o Tech Provider. Limitaciones: 20 mensajes por
  segundo, sin grupos, sin mensajes efímeros, sin vista única, sin ubicación en vivo, sin listas de
  difusión, sin catálogo ni pedidos por API.
* **Registro normativo:** `adr-0014` (alternativa B), `docs/STATUS.md`,
  `docs/plan/fase-b-1-canal-oficial.md`.
* **Qué tendría que cambiar para reabrirlo:** *no aplica.* El hallazgo ya está incorporado como
  mandato de evaluación para la segunda etapa, y **resuelve de paso el pendiente de la interfaz de
  intervención humana**.

### D-06
**Supuesto: "emular el indicador de 'escribiendo' es folclore de vendedores de envíos masivos, sin
respaldo documental".**

* **Afirmado y corregido el mismo día:** 2026-07-28.
* **Por qué se invalidó:** el whitepaper oficial de WhatsApp *"Stopping Abuse: How WhatsApp Fights
  Bulk Messaging and Automated Behavior"* (6 de febrero de 2019), sección *While Messaging*, dice
  literalmente que *"si una cuenta envía mensajes continuamente sin disparar el indicador de
  escritura, puede ser señal de abuso, y banearemos la cuenta"*, en un párrafo propio sobre mecanismos
  que apuntan directamente a la automatización.
* **Matiz que sobrevive y es obligatorio en la redacción:** se documenta como **higiene de coste cero,
  nunca como defensa**. El documento tiene siete años, es anterior a la arquitectura multi-dispositivo,
  no hay evidencia pública de eficacia, y su propio razonamiento —que los emisores masivos "puede que
  no tengan capacidad técnica de falsificarlo"— se debilita cuando falsificarlo cuesta una línea de
  código. **Lo que sí sigue descartado es el paquete que se vende alrededor** (jitter, protocolos de
  "calentamiento"): ver D-08.
* **Registro normativo:** `docs/adr/adr-0015-politica-de-convivencia-con-el-baneo.md`,
  `docs/plan/fase-a-3-adaptador-whatsmeow.md`, `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** *no aplica.* La lección de método sí queda: **antes de
  descartar algo como mito hay que comprobar si existe documentación primaria**. Esta llevaba siete
  años publicada.

---

## Descartes técnicos

### D-07
**Baileys como biblioteca del canal propio, en lugar de whatsmeow.**

* **Descartado:** sin fecha en documento; la decisión entra en el repositorio el 2026-07-26
  (`adr-0009`).
* **Por qué se descartó:** whatsmeow gana por binario Go liviano —determinante para el presupuesto de
  memoria por célula— y por recuperación rápida ante roturas de protocolo.
* **Registro normativo:** `docs/adr/README.md`, fila `adr-0009` (el archivo del ADR está por escribir).
* **Qué tendría que cambiar para reabrirlo:** *hecho externo mutable.* whatsmeow tiene **bus factor
  1**: prácticamente todos sus commits son de un único mantenedor. Si lo pierde, esta decisión se
  reabre de inmediato — y conviene tener la evaluación hecha **antes** de necesitarla.

### D-08
**Prácticas anti-baneo rechazadas en bloque:** proxies, VPN o rotación de IP; parchear whatsmeow para
camuflar su huella de protocolo; números virtuales o SIM recién activada; mensajes proactivos "útiles"
(recordatorios, seguimientos, encuestas, "¿sigues ahí?"); reconexión agresiva tras un baneo temporal;
número maestro compartido entre clientes o a nombre de HexCell; reactivación automática de una célula
baneada sin decisión humana; prometer disponibilidad sobre el canal propio; y creer que la capa de
detección temprana evita baneos, cuando solo acorta el tiempo de reacción. Aparte, en la sección de
medidas del mismo ADR, quedan excluidos el **jitter** y los **protocolos de "calentamiento"** de
cuenta.

* **Descartadas:** 2026-07-28 (`adr-0015`).
* **Por qué se descartaron:** las direcciones IP de centro de datos son señal antispam directa, de
  modo que un proxy **empeora** el perfil. La detección de clientes no oficiales es multiseñal:
  camuflar la huella no funciona y además saca del flujo de actualizaciones de la biblioteca, que sí
  importa. Los mensajes proactivos atacan la causa de baneo documentada número uno. Reconectar durante
  un baneo temporal **escala el baneo a permanente** (`faq.whatsapp.com/1848531392146538`). El resto
  es folclore de proveedores de envío masivo, sin evidencia.
* **Registro normativo:** `docs/adr/adr-0015-politica-de-convivencia-con-el-baneo.md`, sección "lo que
  NO hay que hacer", escrita expresamente para que nadie lo reintroduzca como idea nueva.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño con causa documentada.* **No
  reabrir.** Si alguien vuelve con una de estas ideas, la respuesta está aquí y en `adr-0015`.

### D-09
**Escribir por adelantado la firma del adaptador de Cloud API durante la etapa A-1, como "mitigación
de compatibilidad".**

* **Retirado:** 2026-07-27.
* **Por qué se descartó:** patrón *"compila ≠ correcto"*. Una firma que compila no garantiza la
  semántica; la garantía real son los tests de contrato contra el caso más restrictivo. El crate
  `hexcell-meta` nace vacío hasta que se resuelva el `adr-0013`.
* **Registro normativo:** `docs/STATUS.md` (entrada de endurecimiento),
  `docs/plan/fase-b-1-canal-oficial.md` (tabla de riesgos).
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir.**

### D-10
**Vía de escape "excepción documentada como deuda de diseño" en el criterio de que el núcleo no se
toca para soportar el canal oficial (etapa B-1).**

* **Eliminada:** 2026-07-27.
* **Por qué se descartó:** convertía en negociable el criterio central de toda la estrategia de dos
  canales. Ahora, si el adaptador de Cloud API exige tocar el núcleo, la etapa **no se acepta**: el
  trabajo se detiene y el contrato del puerto se corrige mediante una revisión explícita del
  `adr-0010`.
* **Registro normativo:** `docs/plan/fase-b-1-canal-oficial.md` (criterios de aceptación),
  `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir.**

### D-11
**Dejar los respaldos para la etapa de endurecimiento final.**

* **Descartado:** 2026-07-26, adelantándolos a la etapa A-2.
* **Por qué se descartó:** con pilotos reales desde el principio, los respaldos no pueden esperar.
  Cubren **tres** bases: `sessions.db`, `knowledge_live.db` y el `sqlstore` del sidecar.
* **Registro normativo:** `docs/STATUS.md`, `docs/plan/fase-a-2-nucleo-persistencia.md`.
* **Qué tendría que cambiar para reabrirlo:** *no aplica.*

### D-12
**Devolver códigos 429 o 503 a Meta bajo sobrecarga.**

* **Descartado:** sin fecha en documento; la decisión entra en el repositorio el 2026-07-26
  (`adr-0004`).
* **Por qué se descartó:** dispara las tormentas de reintentos automáticos de la API Graph. Se
  sustituye por el patrón *Fast-Reject*: `HTTP 200 OK` sintético e inmediato.
* **Registro normativo:** `docs/PRD.md` (FR-08), `docs/adr/README.md` fila `adr-0004`.
* **Qué tendría que cambiar para reabrirlo:** *hecho externo mutable* — si Meta cambia el
  comportamiento de reintentos de la API Graph.

### D-15
**Guardar el mapeo de identidad de conversación —y con él la lista de exclusión (STOP)— dentro del
`sqlstore` del sidecar, en lugar de en un almacén propio del adaptador.**

* **Descartado:** 2026-07-28 (`adr-0010`).
* **Por qué se descartó:** es el sitio que parece natural, porque "todo lo de whatsmeow vive ahí", y
  por eso mismo hay que dejarlo escrito. La rama `LoggedOut` con `device_removed` **obliga a descartar
  el `sqlstore`**: whatsmeow ya ha borrado la sesión, el dispositivo no existe en el servidor de
  WhatsApp y la única salida es el re-emparejamiento. Un mapeo alojado dentro del `sqlstore` se
  destruiría **justo en el único escenario en el que se necesita que sobreviva**, y tras el
  re-emparejamiento cada contacto abriría un hilo nuevo: el cliente percibiría amnesia inmediatamente
  después de una incidencia, que es el peor momento posible. Con la lista STOP dentro, el daño es
  peor: un contacto que pidió la baja volvería a recibir mensajes. El mapeo vive por tanto en un
  almacén propio del adaptador sobre el volumen de la célula, separado del `sqlstore`, y pasa a ser la
  **cuarta base del respaldo**.
* **Registro normativo:** `docs/adr/adr-0010-puerto-de-canal.md` (decisión 6 y alternativa C),
  `docs/plan/fase-a-3-adaptador-whatsmeow.md` (tareas 9 y 13, y su tabla de riesgos),
  `docs/plan/fase-a-2-nucleo-persistencia.md` (respaldo de las cuatro bases), `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir.** Solo decaería si
  whatsmeow dejara de borrar la sesión ante `device_removed`, que es precisamente el comportamiento
  del que depende toda la regla de restauración.

### D-16
**Guardar el identificador de transporte crudo —el JID de whatsmeow o el `wa_id` de Meta— en
`sessions.db`, por comodidad de consulta y de depuración.**

* **Descartado:** 2026-07-28 (`adr-0010`); la regla ya estaba en el PRD (FR-12) desde el 2026-07-26.
* **Por qué se descartó:** contamina datos históricos de clientes de pago y convierte cualquier
  cambio de canal en una migración de datos, que es exactamente lo que FR-12 existe para evitar. El
  alcance de la prohibición es **estrecho y hay que citarlo como tal**: lo que se prohíbe es que
  **`sessions.db`** almacene esos identificadores, no que existan en el sistema. Dentro del adaptador
  existen por necesidad —alguien tiene que traducir— y ahí es donde se quedan, en el almacén de
  identidad del adaptador. Enunciar la regla como "en ningún sitio" sería falso y volvería a abrir el
  debate cada vez que alguien encuentre un JID en el proceso del sidecar.
* **Registro normativo:** `docs/PRD.md` (FR-12, punto 5),
  `docs/adr/adr-0010-puerto-de-canal.md` (decisiones 4 y 5, alternativa D),
  `docs/plan/fase-a-2-nucleo-persistencia.md` (criterio de aceptación con inspección del esquema),
  `docs/plan/fase-a-3-adaptador-whatsmeow.md` (criterio de aceptación del JID).
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir.** Decaería solo si
  se abandonara la estrategia de dos canales convivientes, que es el pilar de `adr-0014`.

---

## Descartes menores

### D-13
**Encolar los mensajes que caen fuera de la ventana de servicio de 24 h, hasta que el cliente vuelva a
escribir.**

* **Descartado:** 2026-07-27, en favor de esperar a que el cliente escriba de nuevo, con escalada a
  humano como excepción.
* **Por qué se descartó:** motivo no registrado en ningún documento; **la alternativa descartada solo
  se ve en el diff del commit `ecc7598`**.
* **Registro normativo:** la decisión adoptada está en `docs/STATUS.md`; la alternativa, en ninguno.
* **Qué tendría que cambiar para reabrirlo:** *a determinar.*

### D-14
**Nombres anteriores del proyecto y de sus piezas:** "ZeroClaw" como nombre del producto (renombrado a
HexCell el 2026-07-27), `hexcell-cell` como nombre del binario de la célula (simplificado a `hexcell`)
e "inquilino" como término para la unidad desplegable por cliente (sustituido por "célula").

* **Por qué se descartaron:** sin motivo registrado; renombres de criterio del dueño.
* **Registro normativo:** solo el historial de git (`e290e40`, `e1876a6`, `fa7ef4d`).
* **Qué tendría que cambiar para reabrirlo:** *cerrado.* Se registran para que nadie confunda una
  mención antigua con un componente distinto.

### D-17
**`tracing` + `tracing-subscriber` con una capa de serialización JSON para el registro
estructurado del motor de mensajería, en lugar de escribirlo a mano.**

* **Descartado:** 2026-07-30 (HEX-007).
* **Por qué se descartó:** arrastra un serializador y alrededor de una docena de crates
  transitivos para emitir, como mucho, un puñado de campos por evento procesado — el mismo
  argumento que este árbol ya aplicó contra `axum`, `tiny-http` y los pools de conexión externos
  de `hexcell-storage`. El registro completo, escrito a mano, son unas pocas decenas de líneas en
  `crates/hexcell/src/registro.rs`, con el conjunto de campos tipado como mecanismo de privacidad
  (`evento: &'static str` no puede transportar un valor construido en tiempo de ejecución).
* **Registro normativo:** `docs/adr/adr-0019-registro-estructurado.md`, `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir**, salvo que el
  presupuesto de memoria por célula (NFR-01) deje de ser una restricción del producto.

### D-18
**`tokio-util::CancellationToken` para transportar la señal de apagado ordenado, en lugar de
`tokio::sync::watch`.**

* **Descartado:** 2026-07-30 (HEX-007).
* **Por qué se descartó:** `tokio::sync::watch` ya estaba habilitado en la característica `sync`
  que `crates/hexcell/Cargo.toml` ya declaraba, y expresa exactamente lo que el apagado ordenado
  necesita: un valor compartido que cambia una vez y que cualquier receptor observa.
  `CancellationToken` duplicaría esa expresividad a cambio de una dependencia nueva que no aporta
  nada que `watch` no cubra ya.
* **Registro normativo:** `docs/adr/adr-0018-apagado-ordenado.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir**, salvo que
  `tokio::sync::watch` deje de estar disponible en la característica `sync` ya habilitada.

### D-19
**API de respaldo en línea de `rusqlite` (característica `backup`, `Connection::backup`) para
copiar `sessions.db`, `knowledge_live.db` y el almacén de identidad del adaptador, en lugar de
`VACUUM INTO`.**

* **Descartado:** 2026-07-30 (HEX-008).
* **Por qué se descartó:** la API de respaldo en línea reinicia su copia cada vez que un escritor
  confirma una transacción; bajo un escritor activo de forma continua puede no llegar a terminar
  nunca, exactamente el escenario de una célula procesando eventos sin pausa. `VACUUM INTO` toma
  una única instantánea de lectura, no necesita activar ninguna característica adicional de
  `rusqlite` y produce, de regalo, un archivo defragmentado en vez de uno con el mismo desorden
  interno que el origen.
* **Registro normativo:** `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir**, salvo que
  `VACUUM INTO` deje de estar disponible en la serie de `rusqlite` que este workspace fija.

### D-20
**Planificador de respaldo periódico dentro del propio proceso de la célula.**

* **Descartado:** 2026-07-30 (HEX-008).
* **Por qué se descartó:** la planificación y el empaquetado de la célula son alcance de la etapa
  A-6, no de esta. Un temporizador propio dentro de cada proceso duplicaría el trabajo de un futuro
  orquestador de respaldo, a cambio de un hilo o una tarea de fondo por célula sobre un presupuesto
  de memoria de ≤ 80 MB (NFR-01) que ya está ajustado. `respaldar_celula` queda como una operación
  de biblioteca sin disparador de producción en esta tarea, invocada hoy solo por los tests de
  integración.
* **Registro normativo:** `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md`, `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir** antes de que la
  etapa A-6 decida el mecanismo real de planificación de la célula.

### D-21
**Usar trybuild como mecanismo de prueba compile-failure.**

* **Descartado:** 2026-08-09 (HEX-016).
* **Por qué se descartó:** el invariante `compile_fail` doctest es suficiente, `trybuild` añadiría una dependencia de desarrollo y un directorio de fixtures; la prueba E0639 no se refuerza en rustc estable 1.92.0 pero se mitiga con un doctest positivo emparejado que rompe si se renombra o elimina la API.
* **Registro normativo:** `docs/adr/adr-0021-testigo-de-entrante.md`.
* **Qué tendría que cambiar para reabrirlo:** si el doctest positivo deja de ser mitigación suficiente (p.ej. si rustc cambia la semántica de `compile_fail` en un modo que invalide el emparejamiento) o si se necesita probar más de un error de compilación en el mismo crate.

### D-22
**Respaldo concurrente sin pausa previa (steal-and-exit con reconexión automática del adaptador).**

* **Descartado:** 2026-08-19 (HEX-029).
* **Por qué se descartó:** El servidor IPC del sidecar aplica relevo de conexión única donde la más reciente gana (`servidor/manejo.go`, `protocolo-ipc-nucleo-sidecar.md`). La reconexión automática del núcleo en ejecución con `Retroceso::por_omision()` (500 ms inicial) desplaza al proceso de respaldo antes de que el sidecar concluya `VACUUM INTO`. La conexión IPC del respaldo queda cerrada, el `acuse_respaldo_sqlstore` se descarta y la operación falla con `RespaldoSinAcuse`.
* **Registro normativo:** `crates/hexcell/src/respaldar.rs`, `docs/runbook-restauracion-de-celula.md`.
* **Qué tendría que cambiar para reabrirlo:** Requeriría que el sidecar acepte múltiples conexiones activas concurrentes sobre IPC, lo cual alteraría el protocolo cerrado v1.3 (cable 4).

### D-23
**Disparador de respaldo en el propio proceso del núcleo mediante señales o variables de entorno.**

* **Descartado:** 2026-08-19 (HEX-029).
* **Por qué se descartó:** Un disparador interno por señales dentro del núcleo no puede entregar un código de salida (`ExitCode`) ni un mensaje estructurado en `stderr` nombrando la base concreta que falló al operador. Además, añadiría una segunda ruta de procesamiento de señales concurrente con `apagado.rs`.
* **Registro normativo:** `crates/hexcell/src/respaldar.rs`, `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** Requeriría una superficie cuyo resultado sea consumido por un orquestador que analice registros estructurados en lugar de un operador humano leyendo el código de salida de un subcomando.

### D-24
**Generalizar la orden de respaldo del `sqlstore` con un discriminador de almacén para cubrir también `identidad.db` (opción a del hallazgo 12).**

* **Descartado:** 2026-08-20 (HEX-032).
* **Por qué se descartó:** reutilizar `orden_respaldo_sqlstore` / `acuse_respaldo_sqlstore` con un campo que indique qué almacén copiar colisionaría en la correlación del núcleo. El adaptador Rust correlaciona los acuses por `identificador_de_ronda` en un `HashMap<String, oneshot::Sender<…>>` keyeado **solo por ronda**: dos acuses del **mismo tipo** en la misma ronda —uno del `sqlstore`, otro de identidad— se pisarían. Además, mutar la orden/acuse cerrada obligaría a reescribir los campos versionados de `docs/contrato-ipc-respaldo-del-sqlstore.md` (secciones 1 y 3), que las restricciones de la tarea prohíben tocar. Se eligió en su lugar un **par de mensajes dedicado** con un TIPO distinto por almacén (opción b), que deja los mensajes del `sqlstore` byte-idénticos y correlaciona cada acuse en su propio mapa de pendientes.
* **Registro normativo:** `docs/adr/adr-0022-respaldo-identidad-sidecar-por-ipc.md`, `docs/protocolo-ipc-nucleo-sidecar.md` (sección 7, versión 1.4).
* **Qué tendría que cambiar para reabrirlo:** que el núcleo dejara de correlacionar acuses solo por ronda (p. ej. si adoptara una clave compuesta `(ronda, almacén)` en un único mapa), en cuyo caso un mensaje parametrizado por almacén dejaría de colisionar. No reabrir mientras la correlación siga siendo por ronda y el contrato del `sqlstore` deba permanecer intacto.

### D-25
**Centralizar las bases de datos operativas (un RDBMS único multi-inquilino para el camino caliente).**

* **Descartado:** 2026-08-21 (HEX-034).
* **Por qué se descartó:** pierde la aislación por célula (FR-02: radio de explosión, y el mover/borrar/restaurar por cliente probado en A-3), compite por RAM/CPU en hardware modesto, y whatsmeow y sessions.db necesitan SQLite local con WAL vía driver de archivo (no una API de base remota).
* **Registro normativo:** `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** un despliegue en nube con múltiples máquinas donde se quiera un RDBMS gestionado con alta disponibilidad real, o la necesidad de consultas transaccionales cruzadas entre clientes como función central.

### D-26
**rqlite / libSQL sqld en el camino caliente (los almacenes operativos del bot por HTTP).**

* **Descartado:** 2026-08-21 (HEX-034).
* **Por qué se descartó:** latencia de consenso/HTTP en el bucle caliente sobre hardware modesto, opuesto al propósito del SQLite embebido de latencia cero; whatsmeow abre un archivo local vía database/sql y no habla la API HTTP de rqlite; la alta disponibilidad real de rqlite exige múltiples máquinas (en un solo servidor no hay HA de todas formas). RESERVA explícita: rqlite/libSQL no se descarta para la capa de lectura derivada (de cara al cliente); allí sí es candidata.
* **Registro normativo:** `docs/STATUS.md`.
* **Qué tendría que cambiar para reabrirlo:** se evalúa libSQL sqld / rqlite únicamente para la capa derivada cuando esa capa se apruebe (ver la entrada Pendiente correspondiente en STATUS), nunca para el camino caliente.

### D-27
**Alternativas descartadas para la inferencia HTTPS outbound (reqwest, native-tls/openssl, aws-lc-rs, backoff exponencial, reintentar HTTP 429, noveno crate de workspace).**

* **Descartado:** 2026-08-26 (HEX-044).
* **Por qué se descartó:** `reqwest` añade ~85 crates extra en el lockfile; `native-tls`/`openssl` requieren bibliotecas dinámicas del sistema anfitrión violando el empaquetado autónomo (`adr-0003`); `aws-lc-rs` exige `cmake` como herramienta de compilación adicional mientras `ring` solo exige el compilador C ya usado por SQLite; el backoff exponencial hace impredecible el tiempo total de cola de drenaje del proceso; reintentar HTTP 429 agrava el agotamiento de cuota y retrasa la liberación de reservas de presupuesto; y crear un noveno crate de workspace viola la regla de que lo que solo el binario consume vive como módulo de `hexcell`.
* **Registro normativo:** `docs/adr/adr-0012-inferencia-externa.md`, `crates/hexcell/Cargo.toml`.
* **Qué tendría que cambiar para reabrirlo:** Para `reqwest` o `aws-lc-rs`, que la pila `hyper`+`rustls`/`ring` deje de compilar en rustc estable sin `cmake`. Para HTTP 429 o backoff exponencial, que el proveedor especifique cabeceras Retry-After respetables dentro del margen de drenaje sin violar el límite total de apagado.

### D-28
**Alternativas descartadas para el puerto de embeddings y adaptador OpenRouter (compartir parser de chat, zipping posicional, reserva por fragmento/ingesta, elevar timeout, base64, pseudo-conversación).**

* **Descartado:** 2026-08-27 (HEX-051-a).
* **Por qué se descartó:**
  * *Compartir el analizador de chat:* `proveedor_openai.rs` exige obligatoriamente `completion_tokens` para evitar subfacturación. El endpoint `/embeddings` carece de completaciones; relajar la validación de chat abriría una vulnerabilidad financiera en la inferencia.
  * *Emparejamiento posicional:* los proveedores externos pueden retornar elementos desordenados o parciales; la unión por posición vincularía vectores al fragmento equivocado corrompiendo la búsqueda semántica.
  * *Granularidad por fragmento o por ingesta:* por fragmento multiplicaría filas y suelos mínimos; por ingesta global impediría la conciliación atómica tras cada lote HTTP.
  * *Elevar tiempo de espera o límite de drenaje:* rompería el presupuesto de apagado ordenado de 20 segundos; la solución arquitectónica correcta es acotar el tamaño del lote (`HEXCELL_EMBEDDINGS_TAMANO_DE_LOTE`).
  * *Formato base64:* incrementa la latencia de decodificación y riesgo de fallos silenciosos; se fija `encoding_format: "float"`.
  * *Pseudo-conversación artificial:* ensuciaría la auditoría de `consumo_por_conversacion` con registros ficticios; la reserva de catálogo es explícitamente sin conversación (`id_conversacion NULL`).
* **Registro normativo:** `docs/adr/adr-0025-puerto-de-embeddings.md`, `crates/hexcell-core/src/embeddings.rs`, `crates/hexcell/src/proveedor_embeddings.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-29
**Alternativas descartadas para la conmutación atómica de épocas de conocimiento (cerrojo en pool, unlink+symlink, copia en caliente, reinicio de proceso).**

* **Descartado:** 2026-08-30 (HEX-055).
* **Por qué se descartó:**
  * *Cerrojo (`Mutex` o `RwLock`) alrededor del puntero del pool de conocimiento:* `GestorDePools` vive detrás de `Arc` en múltiples puntos del sistema, por lo que no hay referencias mutables disponibles; un cerrojo penalizaría con adquisición de candado cada consulta de lectura conversacional para una conmutación que ocurre solo una vez por ingesta. Se adoptó `ArcSwap`.
  * *Reasignación de enlace mediante `unlink` seguido de `symlink`:* introduce una ventana temporal en la cual la ruta no resuelve a ningún archivo, provocando fallos en lectores concurrentes o creación errónea de bases vacías. Se adoptó el modismo POSIX de enlace temporal atómico con `rename()`.
  * *Copia en caliente (copy-on-promote / sobrescritura de archivo en vivo):* viola la inmutabilidad de las épocas y expone a lectores concurrentes a lecturas corruptas de páginas mixtas o archivos a medio transferir.
  * *Reinicio del proceso de la célula para conmutar de época:* provocaría caída de servicio y pérdida de conexiones de transporte activas en cada ciclo de ingesta, vulnerando el objetivo de disponibilidad continua (FR-07).
* **Registro normativo:** `docs/adr/adr-0006-epocas-y-conmutacion-atomica.md`, `crates/hexcell-storage/src/promocion.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-30
**Alternativas descartadas para el drenaje de la época superseída (notificación por Condvar, cierre forzado, remediación por borrado, sobrecarga de variable de apagado).**

* **Descartado:** 2026-08-31 (HEX-056).
* **Por qué se descartó:**
  * *Notificación reactiva mediante `Condvar` o canal en la ruta de lectura de conocimiento:* añadir señalización en `PoolDeConocimiento::con_lectura` penalizaría con sincronización cada consulta de lectura ordinaria en el camino crítico para un evento (conmutación y drenaje) que ocurre solo una vez por ingesta; el sondeo con `INTERVALO_DE_SONDEO_DE_DRENAJE` (5 ms) no bloquea y mantiene libre de sobrecarga el camino caliente.
  * *Cierre forzado o interrupción abrupta de conexiones con lectores en vuelo:* viola el invariante de consistencia de lecturas en curso; si el límite temporal expira, el drenaje falla cerrado retornando `DesenlaceDeDrenaje::Expirada` con el descriptor vivo para conservar la observabilidad y permitir reintentos sin corromper transacciones de lectura.
  * *Remediación por borrado automático de archivos secundarios (`-wal` o `-shm`) supervivientes:* si un archivo `-wal` sobrevive con tamaño mayor a cero tras el cierre, contiene datos no consolidados; eliminarlo destruiría la única evidencia para auditar la anomalía. Se aplica la doctrina de verificar y abortar (`CompanieroDeEpocaSobreviviente`), tolerando como residuo inocuo un `-wal` de cero bytes y un `-shm` de conexiones en solo lectura.
  * *Sobrecargar la variable de entorno `HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS`:* dicha variable gobierna el apagado ordenado del proceso (HEX-007) con un presupuesto de 20 s; el drenaje de época opera por evento de ingesta con una cota distinta (10 s, `HEXCELL_LIMITE_DE_DRENAJE_DE_EPOCA_MS`) y no debe acoplarse.
* **Registro normativo:** `docs/adr/adr-0006-epocas-y-conmutacion-atomica.md`, `crates/hexcell-storage/src/drenaje.rs`, `crates/hexcell/src/promocion.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-31
**Alternativas descartadas para la reversión de épocas y guardas de fallo silencioso (re-acuñación de épocas, comodín en partición semántica, guarda de enlace colgante en solo lectura, fallback silencioso de ruta canónica).**

* **Descartado:** 2026-08-31 (HEX-057-a).
* **Por qué se descartó:**
  * *Re-acuñar épocas promoviendo la versión anterior como una nueva época N+1, tratando la reversión como una repromoción:* incrementaría indefinidamente los números de época y duplicaría copias físicas en disco, creando ambigüedad sobre la procedencia de los embeddings y violando el principio de identidad intrínseca de los datos. La reversión reutiliza el número ordinal y el archivo físico existente (`knowledge_epoch_N.db`).
  * *Uso de comodín `_` en la función de partición semántica `es_motivo_semantico`:* el uso de un patrón comodín provocaría que cualquier nueva variante de error añadida en el futuro se clasificara silenciosamente en la rama por defecto, rompiendo la partición disjunta de compuertas (AC-6); se exige un `match` exhaustivo de todas las variantes de `MotivoDeRechazo`.
  * *Dispersar la guarda de enlace vivo colgante (`verificar_enlace_vivo_resoluble`) en `abrir_solo_lectura` o `promover_epoca`:* `abrir_solo_lectura` utiliza `SQLITE_OPEN_READ_ONLY`, por lo que SQLite ya falla limpiamente sin crear archivos ni alterar el disco; añadir la guarda allí sería código muerto redundante y violaría la separación de conjuntos de fallo disjuntos entre las guardas 3 y 4.
  * *Fallback silencioso mediante `.unwrap_or(ruta_de_apertura)` ante fallo de `canonicalize` en promoción:* ocultaría enlaces rotos o archivos eliminados, provocando que el descriptor superseído contenga una ruta errónea y que el posterior drenaje verifique el diario WAL del archivo equivocado; se mapea explícitamente a `ErrorDeAlmacen::ArchivoDeEpocaInaccesible`.
* **Registro normativo:** `docs/adr/adr-0026-reversion-de-epocas-y-guardas-de-fallo-silencioso.md`, `crates/hexcell-storage/src/reversion.rs`, `crates/hexcell-storage/src/pools.rs`, `crates/hexcell-storage/src/promocion.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-32
**Escribir la marca de época sospechosa (`.sospechosa`) después de reasignar el enlace simbólico en reversión.**

* **Descartado:** 2026-08-31 (HEX-057-b).
* **Por qué se descartó:** Si la marca se escribiera después de la conmutación de `knowledge_live.db`, cualquier caída del proceso o fallo de E/S en la escritura de la marca dejaría la conmutación consolidada pero la época previa sin marcar. Esto permitiría que un ciclo posterior de `numero_de_epoca_siguiente` reutilizara el número de la época descartada por sospecha de defecto, violando irreversiblemente la garantía de no-reutilización de identificadores. Escribir la marca antes de la conmutación invierte el riesgo: un fallo de escritura de la marca aborta limpiamente la reversión dejando la producción intacta sirviendo la época previa; el peor caso es una marca espuria sobre una época todavía activa, lo cual es recuperable y tiene un sesgo seguro a favor de la protección del sistema.
* **Registro normativo:** `docs/adr/adr-0027-retencion-y-purga-de-epocas.md`, `crates/hexcell-storage/src/reversion.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-33
**Serializar el binario de tests con `--test-threads=1` (o con el crate `serial_test`, o con cualquier otra forma de serialización de la suite) para hacer desaparecer el fallo intermitente de `cargo test --workspace`.**

* **Descartado:** 2026-09-01 (HEX-058).
* **Por qué se descartó:** Funciona, y es exactamente por eso que es peligroso. El fallo medido —1 de cada 25 corridas, con pánico en `crates/hexcell/src/motor.rs:518`— no era una aserción frágil sino comportamiento indefinido real: en la edición 2024, escribir el entorno del proceso puede hacer que `setenv` de glibc reasigne el array `environ` mientras otro hilo lo lee. Serializar la suite elimina la concurrencia, no la escritura: el código que muta estado global del proceso sigue ahí, listo para volver a morder en cuanto alguien ejecute los tests de otra manera, y el árbol paga además el coste permanente de una suite secuencial. Peor todavía, la próxima carrera de esta misma familia también quedaría oculta, y no habría ninguna señal de que existe. La decisión fue eliminar al escritor (inyección de `FuenteDeConfiguracion`, `adr-0028`), no callar al detector.
* **Registro normativo:** `docs/adr/adr-0028-fuente-de-configuracion-inyectable.md`, `crates/hexcell/src/configuracion.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.** Si en el futuro apareciera un estado global del proceso genuinamente inevitable —impuesto por una biblioteca de terceros y sin puerto posible—, la serialización se discutiría solo para ese caso concreto y acotado, nunca como política de la suite.

### D-34
**Mover los tests que mutan el entorno a un binario de integración aparte, dejándolos aislados del resto de la suite.**

* **Descartado:** 2026-09-01 (HEX-058).
* **Por qué se descartó:** Cierra el agujero de hoy y deja abierta la puerta de mañana. El aislamiento funciona solo mientras nadie añada a ese binario un test que **lea** el entorno, y `std::env::temp_dir()` —una lectura del entorno— es el modismo más corriente del árbol para crear un directorio de trabajo en un test: es una trampa que se arma sola. El defecto reaparecería sin ningún aviso, sin cerrojo que revisar y sin señal en la revisión de código, porque el archivo nuevo parecería inocente. La inyección, en cambio, hace la propiedad verificable de forma mecánica: la guarda de grep de CI falla en el momento en que alguien vuelve a escribir el entorno bajo `crates/hexcell/`, esté en el binario que esté.
* **Registro normativo:** `docs/adr/adr-0028-fuente-de-configuracion-inyectable.md`, `crates/hexcell/tests/configuracion.rs`, `crates/hexcell/tests/promocion.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir** mientras la lectura de configuración siga siendo inyectable. Solo se reconsideraría si apareciera una dependencia que exigiera mutar el entorno del proceso en tiempo de test y no admitiera inyección; en ese caso, el aislamiento por binario iría acompañado de una guarda automática que prohíba toda lectura del entorno dentro de ese binario.

### D-35
**Alternativas descartadas al escribir la prueba de estrés de conmutación de época (medir con la anchura de pool por omisión, correr la prueba dentro de la batería por defecto, contrastar NFR-03 contra el intervalo ancho, y relajar la aserción de descriptores con una tolerancia).**

* **Descartado:** 2026-09-07 (HEX-061).
* **Por qué se descartó:**
  * *Medir las veinte lecturas concurrentes con la anchura de pool por omisión (2 conexiones):* `PoolDeConocimiento::con_lectura` reparte con `fetch_add % len` y luego toma un `Mutex` **bloqueante**, así que con dos conexiones los veinte hilos no producen veinte lecturas simultáneas sino veinte lectores haciendo cola sobre dos cerrojos. Nunca habría más de dos conexiones SQLite vivas y `SQLITE_BUSY` sería imposible **por construcción**, no por corrección: la prueba pasaría siempre y no demostraría nada. La anchura se abre a 20 con el constructor que `adr-0029` ya había introducido para esta tarea.
  * *Correr la prueba dentro de `cargo test --workspace` en vez de marcarla `#[ignore]` con un paso propio de CI:* la prueba mide `/proc/self/fd`, que es del proceso entero; con el resto de la batería corriendo en paralelo, esa cuenta mediría el ruido de otros tests y no el ciclo de vida de los pools. La salida obvia —serializar la batería— está cerrada por D-33 y no se reabre. El `#[ignore]` **por sí solo** tampoco servía: habría dejado el criterio de QA del PRD escrito y jamás ejecutado, que es indistinguible de no tenerlo; por eso la decisión son las dos mitades a la vez, y no una.
  * *Contrastar el presupuesto de NFR-03 contra el intervalo desde `promover_epoca` hasta la primera lectura servida:* ese intervalo incluye la revalidación de integridad, el sellado, el punto de control, el renombrado y la apertura del pool nuevo; medido el 2026-09-07 ronda los 88 ms frente a los 0,02 ms de la conmutación real. NFR-03 acota la **conmutación interna**, que es lo que mide `duracion_de_conmutacion_ms`. Contrastarlo contra el intervalo ancho acusaría de incumplimiento a un requisito que no cubre ese trabajo; llamar «conmutación» al intervalo ancho sería medir una cosa y afirmar otra. Se miden y reportan las dos, y solo la estrecha se compara con el presupuesto.
  * *Relajar la aserción de descriptores a una tolerancia (`±1`) para absorber el descriptor extra observado tras la purga:* el desvío era real y explicable —el VFS unix de SQLite aparca por inodo el primer descriptor que no puede cerrar sin borrar cerrojos POSIX ajenos, y lo reutiliza después—, y una tolerancia lo habría tapado junto con cualquier fuga futura de exactamente un descriptor por conmutación, que es justo la magnitud que esta aserción existe para detectar. Se iguala en su lugar el estado de esa caché entre las dos mediciones con una purga en vacío previa, y la aserción sigue siendo de igualdad estricta.
* **Registro normativo:** `docs/adr/adr-0030-prueba-de-estres-de-conmutacion-de-epoca-bajo-lecturas-concurrentes.md`, `crates/hexcell-storage/tests/estres_conmutacion.rs`, `.github/workflows/ci.yml`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño para los tres primeros:* **no reabrir**. El cuarto se reconsideraría solo si el aislamiento por binario dejara de garantizar un proceso limpio (por ejemplo, si `cargo` pasara a ejecutar binarios de test en paralelo); en ese caso la respuesta no sería una tolerancia sino medir los descriptores por inodo de la ruta de datos de la célula, no por proceso.

### D-36
**Medir la simultaneidad de las lecturas con un medidor de pico de hilos alrededor de `recuperar_contexto`.**

* **Descartado:** 2026-09-07 (HEX-061).
* **Por qué se descartó:** La idea era llevar un `AtomicUsize` incrementado antes y decrementado después de cada llamada, con `fetch_max` sobre un pico, y afirmar que el pico supera la anchura por omisión. Mide lo que no se quiere medir: `PoolDeConocimiento::con_lectura` toma un `Mutex` **bloqueante**, así que un hilo esperando en cola está dentro de la llamada exactamente igual que uno leyendo, y el pico llegaría a veinte incluso con dos conexiones vivas. Sería una guarda que aparenta comprobar la simultaneidad sin comprobarla —el mismo defecto que la anchura configurada y nunca afirmada— y una guarda falsa es peor que ninguna, porque la ausencia se nota y la falsa tranquiliza. En su lugar se cuentan los descriptores del proceso que apuntan al archivo de la época viva: cada conexión de lectura abre ese archivo al construirse, de modo que ese número **son** las conexiones SQLite vivas, no los hilos que las esperan.
* **Registro normativo:** `docs/adr/adr-0030-prueba-de-estres-de-conmutacion-de-epoca-bajo-lecturas-concurrentes.md`, `crates/hexcell-storage/tests/estres_conmutacion.rs`.
* **Qué tendría que cambiar para reabrirlo:** Que `PoolDeConocimiento` expusiera el número de conexiones de lectura efectivamente ocupadas en un instante dado. Con esa cifra, un medidor de pico mediría conexiones y no hilos, y sería una señal legítima; hoy esa cifra no existe y añadirla queda fuera del alcance de una tarea de pruebas.

### D-37
**Afirmar el muro estricto de NFR-03 (< 10 ms) sobre `duracion_de_conmutacion_ms` dentro de la prueba de estrés de conmutación.**

* **Descartado:** 2026-09-07 (HEX-061, decisión humana).
* **Por qué se descartó:** El campo mide lo correcto y por eso mismo no se puede acotar ahí. `duracion_de_conmutacion_ms` no cronometra solo el intercambio del `ArcSwap`: el `Instant` de `crates/hexcell-storage/src/promocion.rs` abarca el intercambio **más** la toma de un cerrojo de lectura del pool nuevo y la consulta de vitalidad, es decir el tramo «de la reasignación del puntero a la primera lectura servida» que NFR-03 define. Dentro de la prueba de estrés, esa consulta tiene que ganarle un cerrojo del pool a veinte hilos que lo están saturando a propósito. En esta máquina el valor cae entre 0,018 y 0,047 ms (44 corridas del 2026-09-07, incluidas 12 fijadas a dos núcleos con carga externa), un margen de unas 200 veces contra el muro; pero en un runner de dos núcleos con sobresuscripción 20:2, una sola expropiación del planificador de unos 10 ms lo rompe, y la latencia de cola no es proporcional a la media. Sería una intermitencia cableada en CI, que fallaría semanas después sobre trabajo ajeno y sin relación con la causa. El muro no compraba nada, además: `crates/hexcell-storage/tests/promocion.rs:377` **ya** afirma `duracion_de_conmutacion_ms < 10.0` y ese archivo no lanza ningún hilo, o sea que NFR-03 está certificado bajo la condición no contendida y parecida a producción que el requisito describe. La prueba de estrés duplicaba ese muro bajo una contención que el requisito nunca contempló. En su lugar se reportan ambas duraciones y se afirma un techo de regresión catastrófica de 1000 ms, cuyo propósito declarado es detectar que la conmutación empezó a *esperar* por algo (E/S, convoy de cerrojos) y no certificar una latencia. Ese techo no es ciego, y conviene que el motivo viva también aquí y no solo en `adr-0030`: queda unas 21.000 veces por encima del peor caso observado (0,047 ms), de modo que el ruido del planificador no lo alcanza, y a la vez queda muy por encima de la secuencia de promoción **entera** (88–140 ms en esta máquina, revalidación de índice incluida), de modo que superarlo significaría que el intercambio del puntero tardó más de siete veces lo que tarda la promoción completa de la que es una parte diminuta. Eso no sería una latencia peor sino un cambio de clase: la conmutación dejó de calcular y pasó a esperar.
* **Registro normativo:** `docs/adr/adr-0030-prueba-de-estres-de-conmutacion-de-epoca-bajo-lecturas-concurrentes.md`, `crates/hexcell-storage/tests/estres_conmutacion.rs`, `crates/hexcell-storage/tests/promocion.rs`.
* **Qué tendría que cambiar para reabrirlo:** Que NFR-03 dejara de estar certificado fuera de esta prueba —si alguien debilitara o borrara la aserción estricta de `tests/promocion.rs`, el requisito se quedaría sin guarda y habría que reponerla, allí y no aquí—, o que la conmutación dejara de tomar cerrojos del pool en su camino de medición, momento en el cual un muro estricto bajo contención volvería a medir el sistema en vez del planificador. Lo que **no** justifica reabrirlo es querer «más cobertura»: dos aserciones del mismo umbral sobre el mismo campo no certifican más que una, solo fallan más a menudo.

### D-38
**Añadir exclusión mutua real entre `respaldar_en` y `iniciar_promocion`/`promover_epoca` — mediante un parámetro `promotion_guard` en `respaldar_en`, una bandera compartida consultada desde el respaldo, o cualquier variante que pause la promoción mientras un respaldo está en vuelo.**

* **Descartado:** 2026-09-08 (HEX-062, decisión humana).
* **Por qué se descartó:** Invierte el diseño fail-open del árbol. `GestorDePools::respaldar_en` ya toma `&self`, no toma promoción guard, y la razón está en la propia tarea que esta entrada cierra: un `VACUUM INTO` sobre la base de conocimiento **sí** puede durar lo bastante como para que una promoción posterior tenga que esperarlo, y bajo un cerrojo compartido esa espera pagaría sobre el camino caliente de la ingesta. El comportamiento actual —el respaldo se ejecuta cuando puede, y si sobrevive a la conmutación el drenaje falla cerrado con `DesenlaceDeDrenaje::Expirada` y la purga posterior conserva la época huérfana como `SuperseidaSinDrenar`— está verificado por la prueba `un_respaldo_que_supera_el_limite_de_drenaje_deja_la_epoca_superseida_sin_drenar_y_protegida` (`crates/hexcell-storage/tests/respaldo_durante_conmutacion.rs`), así que cerrar la ventana por encima del problema es legítimo: el invariante de no-pérdida se sostiene desde la **retención**, no desde la promoción. La otra cara del descarte es que añadir el cerrojo traería un modo de fallo nuevo —un respaldo colgado bloquearía la promoción indefinidamente— que hoy no existe, sin un cambio en la disciplina operacional que lo justifique.
* **Registro normativo:** `docs/adr/adr-0031-respaldo-concurrente-con-conmutacion-de-epoca.md`, `crates/hexcell-storage/src/pools.rs` (doc comment de `respaldar_en` con la justificación explícita), `crates/hexcell-storage/tests/respaldo_durante_conmutacion.rs` (prueba H3 que demuestra el comportamiento que se conserva).
* **Qué tendría que cambiar para reabrirlo:** O bien que el tiempo de `VACUUM INTO` sobre `knowledge_live.db` se acotara por construcción a una fracción demostrablemente pequeña del presupuesto de promoción (por ejemplo, si la base se compactara a una métrica de tiempo de copia subsegundo y se midiera en CI), en cuyo caso un cerrojo compartido sería un coste despreciable y un seguro útil; o bien que la promoción adoptara una cola acotada con descarte de notificaciones de inmediatez —justificación económica que no se ha registrado—. Lo que **no** justifica reabrirlo es la observación aislada de que «un respaldo puede coincidir con una conmutación»: esa coincidencia es exactamente lo que la prueba H1+H2 verifica, y el resultado es una copia etiquetada con la época que físicamente contiene, no una condición de fallo.

### D-39
**Derivar `Serialize`/`Deserialize` sobre `hexcell_storage::DocumentoDeIngesta` o añadir `serde` a `crates/hexcell-storage`.**

* **Descartado:** 2026-09-09 (HEX-063).
* **Por qué se descartó:** `conocimiento.rs:26-28` documenta explícitamente que `DocumentoDeIngesta` se mantiene libre de decoraciones JSON o serializadores externos, asegurando que el modelo de datos de almacenamiento no quede condicionado por el formato de transporte de red. Derivar `Deserialize` sobre este tipo violaría la frontera de diseño de `hexcell-storage` y añadiría una dependencia no deseada a una capa deliberadamente delgada. Se implementa en su lugar el DTO local `DocumentoEntrante` en `crates/hexcell/src/admin.rs` que convierte limpiamente a `DocumentoDeIngesta`.
* **Registro normativo:** `crates/hexcell/src/admin.rs`, `crates/hexcell-storage/src/conocimiento.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-40
**Ejecutar `ejecutar_ingesta` mediante `tokio::task::spawn_blocking` desde el servidor administrativo.**

* **Descartado:** 2026-09-09 (HEX-063).
* **Por qué se descartó:** `ejecutar_ingesta` es una función asíncrona cuya latencia dominante es la llamada de embeddings que requiere `.await`. Mover una función asíncrona entera a `spawn_blocking` exigiría restructurar la ingesta. Las escrituras síncronas a la base en sombra están acotadas por lotes (`tamano_de_lote`) vía `escribir_lote_de_fragmentos`, cediendo el control al ejecutor en cada lote. Se ejecuta inline mediante `tokio::task::spawn` en el runtime `current_thread`, extendiendo el precedente sentado en `promocion.rs`.
* **Registro normativo:** `crates/hexcell/src/admin.rs`, `crates/hexcell/src/promocion.rs`.
* **Qué tendría que cambiar para reabrirlo:** Que se mida degradación inaceptable de la latencia de mensajería mientras una ingesta escribe sus lotes sobre el runtime `current_thread`. Esa medición **no existe todavía**: la prueba de estrés de la tarea 11 del plan (HEX-061, cerrada el 2026-09-07) midió la conmutación de época bajo lecturas RAG concurrentes, no una ingesta larga compitiendo con el motor de mensajería, así que no acredita ni desmiente este descarte. Hace falta una medición nueva, con el motor procesando eventos mientras corre una ingesta de muchos lotes.

### D-41
**Usar `ArcSwap` o `tokio::sync::Mutex` para la compuerta del estado administrativo de ingesta en `EstadoDeAdmin`.**

* **Descartado:** 2026-09-09 (HEX-063).
* **Por qué se descartó:** `arc-swap` no es dependencia de `crates/hexcell` (es de workspace y se usa en storage), y una rutina compare-and-set con `ArcSwap` es más compleja que un cerrojo síncrono estándar. `tokio::sync::Mutex` no es necesario porque ningún guardián de cerrojo cruza un `.await`, respetando la regla del módulo `salud.rs`. Un `std::sync::Mutex<FaseDeIngesta>` resuelve el compare-and-set atómico en una única sección crítica síncrona sin sobrecarga.
* **Registro normativo:** `crates/hexcell/src/admin.rs`, `crates/hexcell/src/salud.rs`.
* **Qué tendría que cambiar para reabrirlo:** *Principio de diseño.* **No reabrir.**

### D-42
**Añadir variables de entorno adicionales (`HEXCELL_TEXTO_SONDA`, `HEXCELL_FRAGMENTACION_*`) para configurar el texto de la sonda y los parámetros de troceado.**

* **Descartado:** 2026-09-09 (HEX-063).
* **Por qué se descartó:** No son parámetros de despliegue, son parámetros del **contenido** de una época de conocimiento. El texto de la sonda, su umbral de aceptación y el troceado determinan qué se escribió dentro de `knowledge_staging.db` y cómo se comparan después los vectores; una época solo es comparable consigo misma si esos valores fueron los mismos cuando se construyó. Puestos en el entorno pasan a ser mutables entre dos arranques del mismo proceso, sin dejar rastro en el árbol ni en la época, y dos ingestas de la misma célula podrían producir épocas incomparables sin que ningún archivo lo delate. Como constantes con nombre (`TEXTO_DE_LA_SONDA_POR_DEFECTO`, `UMBRAL_DE_ACEPTACION_POR_DEFECTO`, `CONFIGURACION_DE_FRAGMENTACION_DE_INGESTA` en `admin.rs`) el valor vigente está versionado y cambiarlo deja un commit. Las dos puertas que sí se abren —dirección del listener y límite de cuerpo— son lo contrario: propiedades del despliegue, que no tocan nada de lo que la época contiene.
* **Registro normativo:** `crates/hexcell/src/admin.rs`.
* **Qué tendría que cambiar para reabrirlo:** Si el primer piloto de producción en la etapa A-7 requiere personalizar el texto de la sonda o el solapamiento de fragmentación para un catálogo específico de cliente.

### D-43
**Extraer también a `abrirRecursosDeArranque` el cableado de `main()` posterior al buzón (`colaSalida`, `srv`, `supervisor`, `traductor`).**

* **Descartado:** 2026-09-10 (HEX-066).
* **Por qué se descartó:** La extracción de esta tarea existe por una razón concreta y acotada: `main.go` era la única parte del sidecar sin ninguna prueba, y ese hueco es lo que dejó vivir durante meses el defecto de orden que HEX-066 cierra. Para taparlo alcanza con hacer probable la **secuencia de apertura**, que es lineal —cada recurso se abre y el siguiente lo consume— y por lo tanto se puede ejercitar contra un directorio vacío. Lo que viene después del buzón no es lineal: `colaSalida`, `srv`, `supervisor` y `traductor` se referencian entre sí, así que extraerlos exige decidir un orden de construcción y una forma de romper esa circularidad. Eso es rediseñar la raíz de composición del sidecar, no hacerla probable, y es una decisión que merece su propia tarea con su propio contrato en vez de entrar de prestado en el arreglo de un defecto de arranque.
* **Registro normativo:** `sidecar/arranque.go`, `sidecar/main.go`.
* **Qué tendría que cambiar para reabrirlo:** Si aparece un segundo defecto en el cableado circular posterior al buzón, o si la etapa A-6 tarea 5 (componer la célula) necesita construir esas piezas en un orden distinto al actual.

### D-44
**Liberar explícitamente los recursos ya abiertos cuando el arranque falla en un paso posterior.**

* **Descartado:** 2026-09-10 (HEX-066).
* **Por qué se descartó:** Es un hallazgo **real** de la auditoría de arranque en frío de esta tarea, no un falso positivo: si `abrirRecursosDeArranque` falla en un paso intermedio, los recursos abiertos en los pasos anteriores no se cierran en esa ruta. Hoy eso no filtra nada observable porque el único consumidor es `main()`, que responde al error con `os.Exit(1)`, y el sistema operativo reclama los descriptores del proceso al terminar. Se descarta arreglarlo **acá** por una razón de disciplina, no porque no importe: HEX-066 existe para cerrar un defecto de ORDEN, y su prueba por mutación acredita exactamente eso. Meter en el mismo diff un cambio de gestión de recursos —que necesita su propia prueba, la de que el fallo intermedio efectivamente cierra lo ya abierto— mezclaría dos defectos de naturaleza distinta bajo una sola guarda, y la segunda quedaría sin acreditar. Se deja escrito para que exista, en vez de arreglarse a medias.
* **Registro normativo:** `sidecar/arranque.go`.
* **Qué tendría que cambiar para reabrirlo:** Si `abrirRecursosDeArranque` gana un segundo consumidor que no sea `main()` —una prueba que la invoque en bucle, o un modo de reintento de arranque—, el momento en que el proceso deja de terminar tras el fallo es el momento en que la fuga pasa a ser observable y este descarte se reabre.

### D-45
**Convertir la clasificación del acuse de entrega/lectura (`events.Receipt`) en un mensaje IPC —sobrecargando `acuse_envio` o añadiendo un tipo nuevo— en lugar de exponerla solo por un sumidero en-proceso.**

* **Descartado:** 2026-09-11 (HEX-072-a).
* **Por qué se descartó:** el acuse de entrega/lectura que clasifica `sidecar/internal/canal/acuses.go` no tiene consumidor fuera del propio proceso del sidecar: la señal que produce es el insumo crudo del productor de métricas periódicas de HEX-072-b, que vive dentro del mismo binario. Convertirla en un mensaje IPC —reutilizando `acuse_envio` o inventando un tipo nuevo— obligaría a ampliar el conjunto cerrado de tipos del protocolo (sección 6 de `docs/protocolo-ipc-nucleo-sidecar.md`), a subir su versión de cable y, con ello, a tocar el extremo Rust del cable y al menos un crate, todo por una señal que nadie fuera del proceso consume. Sobrecargar `acuse_envio` en particular sería peor: su vocabulario `estado` (`enviado`, `entregado`, `leido`, `fallido`) ya está cerrado y correlacionado con `mensaje_saliente`, y reutilizarlo para un acuse **sin** `id_mensaje` conocido en el núcleo ensuciaría esa correlación.
* **Registro normativo:** `sidecar/internal/canal/acuses.go` (el sumidero en-proceso `SumideroDeAcuses`), `sidecar/internal/ipc/mensajes.go` (constantes `EstadoEnvioEntregado`/`EstadoEnvioLeido` reutilizadas sin tocar el protocolo).
* **Qué tendría que cambiar para reabrirlo:** *reabrible si aparece un consumidor fuera del proceso.* Si una pieza que no sea el sidecar —el núcleo, el orquestador, un panel— necesitara la clasificación entregado/leído, habría que abrir un **tipo de mensaje IPC nuevo y explícito** (nunca sobrecargar en silencio el vocabulario `estado` de `acuse_envio`), con su propia versión de cable y su propia correspondencia en el extremo Rust. Eso es exactamente el trabajo que HEX-072-b descarta a su vez si decide que la métrica se queda dentro del sidecar.

### D-46
**Usar una lista enlazada de LRU real (`container/list` o equivalente, con un nodo por entrada movido a la cabeza en cada actividad) para el desalojo de `contactos` y `correlaciones` de `sidecar/internal/metricas`, en vez de un escaneo lineal de mínimo sobre el mapa acotado en cada desalojo.**

* **Descartado:** 2026-09-12 (HEX-072-b).
* **Por qué se descartó:** una lista de LRU real baja el desalojo de O(n) a O(1) por entrada, pero esa ganancia no se cobra aquí: `n` está acotado por construcción a `MaximoContactos = 256` y `MaximoCorrelaciones = 1024`, y el desalojo solo ocurre al insertar una entrada *nueva* que ya excede la cota, nunca en el camino caliente de `ObservarAcuse` sobre una entrada existente. Un escaneo de mínimo sobre a lo sumo 1024 punteros, bajo un mutex que de todos modos hay que tomar para la propia inserción, es una fracción despreciable del trabajo por evento. La lista de LRU, en cambio, traería dos costos reales: (1) el desempate por id ascendente de la doctrina D-08 no es el orden natural de una lista de "más reciente primero" —dos entradas con la misma marca de actividad exigirían de todos modos comparar sus id, así que la lista no elimina esa comparación, solo la complica—; y (2) cada observación (`ObservarEnvio`, `ObservarAcuse`) tendría que mover un nodo en la lista además de actualizar el mapa, dos estructuras a mantener sincronizadas en vez de una, con más superficie para un defecto de sincronización silencioso. El escaneo lineal, al ser una función pura sobre los valores del mapa, es además trivialmente correcto de leer y de probar por mutación: el resultado no depende de qué estructura auxiliar se recorra primero, solo de los valores comparados.
* **Registro normativo:** `sidecar/internal/metricas/metricas.go` (`desalojarContacto`, `desalojarCorrelacion`), `sidecar/internal/metricas/metricas_test.go` (escenarios de desalojo y de desempate).
* **Qué tendría que cambiar para reabrirlo:** Que `MaximoContactos` o `MaximoCorrelaciones` crecieran en órdenes de magnitud —miles o decenas de miles— hasta que el escaneo lineal por desalojo se volviera medible frente al resto del trabajo por evento. Eso exigiría primero revisar el presupuesto de memoria de `adr-0033` (~110 KB), no solo cambiar la estructura de datos.

### D-47
**Declarar la señal de parada con la clave `stop_signal:` de `deploy/cell.compose.yml` en lugar de la directiva `STOPSIGNAL` de cada Dockerfile.**

* **Descartado:** 2026-09-13 (HEX-075).
* **Por qué se descartó:** `stop_signal:` en el YAML de composición y `STOPSIGNAL` en el Dockerfile expresan la misma señal, pero en capas distintas y con alcance distinto. La imagen es el artefacto que se distribuye y se ejecuta también fuera de esta plantilla de composición (`docker run` directo, otra orquestación futura); si la señal de parada viviera solo en `deploy/cell.compose.yml`, cualquier consumidor de la imagen que no pasara por esa plantilla heredaría el valor por omisión de Docker sin saber que el contrato explícito es SIGTERM. Fijarla en el Dockerfile la hace parte del contrato de la IMAGEN, no de un despliegue particular, y es además lo que permite anclarla con un guardia que lee el Dockerfile directamente (`deploy/verificar_senales.sh`) sin depender de que `docker compose config` la resuelva. Con ambas rutas disponibles, elegir la del Dockerfile es coherente con cómo esta tarea ya trata USER y el resto de instrucciones de endurecimiento de HEX-069: en la imagen, no en la composición.
* **Registro normativo:** `Dockerfile`, `sidecar/Dockerfile` (directiva `STOPSIGNAL SIGTERM`).
* **Qué tendría que cambiar para reabrirlo:** *reabrible si aparece un caso donde la señal de parada deba variar por célula* (hoy no existe: SIGTERM es fijo para las dos imágenes y no es una dimensión per-célula). Si tal caso apareciera, `stop_signal:` en la plantilla de composición sería la vía correcta, porque ahí sí vive lo que distingue a una célula de otra.

### D-48
**Ejercer los vectores de cruce de red y de socket IPC del script en vivo de aislamiento (`deploy/verificar_aislamiento.sh`) con `docker exec` directo dentro de los contenedores `nucleo`/`sidecar` reales, usando `nc`/`stat` de BusyBox como ya hace `deploy/verificar_endurecimiento.sh` sobre el YAML resuelto.**

* **Descartado:** 2026-09-13 (HEX-076).
* **Por qué se descartó:** el endurecimiento de HEX-069 retira `/bin/sh` y `/bin/busybox` de las dos imágenes finales (`Dockerfile:126`, `sidecar/Dockerfile:270`; confirmado corriendo `alpine:3` sin endurecer, donde `nc`/`stat`/`sh` sí existen, contra las imágenes finales del proyecto, donde no queda ningún binario salvo el propio `ENTRYPOINT` estático). Un `docker exec` contra `nucleo` o `sidecar` no tiene ningún intérprete ni herramienta que invocar: la premisa de que "BusyBox ya está presente en las imágenes finales `alpine:3`" es cierta para la imagen `alpine:3` sin modificar, pero falsa para las imágenes finales de este proyecto, que la retiran deliberadamente en la misma tarea que impone el resto del endurecimiento. Se optó, en cambio, por un contenedor auxiliar efímero `alpine:3` (la misma base ya usada por `deploy/verificar_apagado_ordenado.sh` para inspeccionar el WAL, no una herramienta nueva) lanzado con `--network container:<contenedor>` y `--volumes-from <contenedor>`: comparte el espacio de nombres de red y los montajes exactos del contenedor objetivo sin ejecutar nada dentro de la imagen endurecida ni añadirle un binario.
* **Registro normativo:** `deploy/verificar_aislamiento.sh` (funciones `desde`, `leer_volumen`, `escribir_volumen`).
* **Qué tendría que cambiar para reabrirlo:** *reabrible si una tarea futura reintroduce un intérprete o BusyBox en las imágenes finales* (hoy no existe: la retirada es deliberada y documentada en ambos Dockerfiles como parte del endurecimiento de HEX-069). Si eso cambiara, `docker exec` directo volvería a ser viable y más simple que el contenedor auxiliar compartido.

---

### D-49

**Probar que el socket IPC de una célula no es alcanzable desde otra comparando únicamente el dispositivo de archivos (`stat -c %d`) de ambos sockets, y declarar el aislamiento roto cuando coinciden.**

* **Descartado:** 2026-09-13 (HEX-076).
* **Por qué se descartó:** dos volúmenes nombrados distintos de Docker viven en el **mismo sistema de archivos del anfitrión**, así que su número de dispositivo coincide siempre. Medido el 2026-09-13 sobre dos volúmenes recién creados: ambos devuelven dispositivo `31` con inodos distintos (`20349905` y `20349970`). La aserción, por lo tanto, no podía pasar nunca: era una **guarda invertida**, roja incluso con el aislamiento intacto, y así se comportó en la primera corrida real del script en vivo, que reportó `FALLA` en AC-8 mientras AC-6 demostraba con marcadores reales que los volúmenes sí estaban aislados. El discriminante correcto es el par **dispositivo:inodo** (`stat -c %d:%i`), que es la identidad de archivo de POSIX; con él las once aserciones pasan.
* **Registro normativo:** `deploy/verificar_aislamiento.sh` (bloque AC-8).
* **Qué tendría que cambiar para reabrirlo:** *reabrible solo si los volúmenes de una célula pasaran a residir en sistemas de archivos separados* (por ejemplo, un dispositivo de bloque dedicado por célula). En ese escenario el número de dispositivo volvería a discriminar, pero seguiría siendo redundante frente al par dispositivo:inodo, que es correcto en ambos casos.

---

### D-50

**Promedio móvil de latencias a través de múltiples acuses en `sidecar/internal/metricas`, en lugar de la métrica de "última observada" entre `ObservarEnvio` y `ObservarAcuse` que `adr-0035` define como `latencia_hasta_acuse_ms`.**

* **Descartado:** 2026-09-13 (HEX-077-c).
* **Por qué se descartó:** la alternativa introduce estado nuevo (un contador de acuses y un acumulador de deltas, además del campo de salida) para una funcionalidad que ni la promesa original de A-3 ni la tarea 20 de A-6 (`docs/plan/fase-a-6-empaquetado-cli.md:320-323`) piden. La promesa del plan enumera la latencia como una serie más del productor, en la misma cardinalidad que `reconexiones_por_hora` y `silencio_entrante_ms` (un único entero por célula), no como una distribución. Un promedio móvil rompería esa cardinalidad —pasaría de un escalar a un escalar + un contador, o a un escalar con un factor de decadencia que exige disciplina de promoción a la vista— y lo haría **silenciosamente**: HEX-077-c está acotado por contrato a una sola clave nueva, y un cambio de cardinalidad sin un ADR específico que lo justifique sería una re-implementación del contrato de la tarea 20, no una mejora de implementación. Además, el evento de interés para la alerta futura no es la "tendencia" sino el **último caso**: una latencia puntual en degradación sostenida es detectable con un umbral simple sobre la última observada (la forma que ya cubre la métrica agregada de las tres series de adr-0033), y promediar la escondería bajo el peso de los acuses anteriores. Por último, el cálculo del promedio añadiría a `ObservarAcuse` una sección crítica adicional bajo el mismo `mu` que ya protege la unión `id_correlacion -> id_conversacion`: hoy ese mutex solo lee `corr.creadaMs` y muta `ultimaLatenciaAcuseMs`; un promedio añadiría dos mutaciones más y un invariante de no-desborde, más superficie para un defecto de sincronización silencioso. Una métrica de "última observada" se reduce a una resta y una asignación, trivialmente correcta de leer y de probar por mutación: la prueba `TestLatenciaHastaAcuseReflejaElMasRecienteNoElPrimero` (escenario 15) documenta que un latch de tipo `if zero` sería un ataque al invariante, y bajo `-count=1` falla en rojo exactamente cuando se introduce.
* **Registro normativo:** `docs/adr/adr-0035-latencia-hasta-el-acuse-en-metricas-del-sidecar.md` (sección "Alternativas consideradas y descartadas"), `sidecar/internal/metricas/metricas.go` (`ObservarAcuse`, `ultimaLatenciaAcuseMs`, `Instantanea`), `sidecar/internal/metricas/metricas_test.go` (escenario 15 con su `// MUTACIÓN:` que ancla la guarda contra el latch).
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño.* **No reabrir** mientras la promesa de la tarea 20 siga siendo "un entero por célula, comparable con `reconexiones_por_hora` y `silencio_entrante_ms`". Se reconsideraría solo si la tarea 20 del plan (HEX-077-b, alertas) necesitara una **tendencia** y no un valor puntual, situación en la que la métrica agregada se quedaría corta por contrato y la alternativa exigiría su propio ADR —con su propio D-NN que la registrara como descarte aquí—. Lo que **no** justifica reabrirlo es la observación aislada de que "un promedio suaviza el ruido": ese es exactamente el argumento que el latch de la prueba de mutación desenmascara como semánticamente distinto, y la diferencia entre "tendencia" y "último caso" es una diferencia de contrato que la promesa del plan no autoriza a tomar en silencio.

---

### D-51

**Vigilancia externa (dead-man's switch, HEX-077-d): implementarla como subcomando de
`hexcell-admin`, como binario/crate Rust nuevo, gatear el ping a la salud de la célula, y darle
reintento/backoff local ante un fallo transitorio.**

* **Descartado:** 2026-09-13 (HEX-077-d).
* **Por qué se descartó:** son cuatro alternativas distintas, agrupadas bajo un solo número siguiendo
  el precedente de D-28, porque las cuatro se estudiaron y rechazaron en el mismo diseño y ninguna
  sobrevive sola. (1) **Subcomando de `hexcell-admin`**: la URL de healthchecks.io es `https`, y
  `crates/hexcell-admin/src/docker/transporte.rs` es un cliente HTTP/1.1 escrito a mano sobre
  `UnixStream`, documentado explícitamente "sin bollard, sin hyper y sin tokio"; hablar `https` desde
  ahí exigiría meter una pila TLS completa (rustls + hyper-rustls + tokio + hyper) en el único crate
  cuyo punto de diseño es casi-cero dependencias. (2) **Binario o crate nuevo**: es estrictamente un
  superconjunto del costo anterior más un miembro de workspace nuevo, sin ninguna ventaja a cambio.
  (3) **Gatear el ping a la salud de la célula**: `deploy/cell.compose.yml` no publica ningún puerto y
  `HEXCELL_DIRECCION_SALUD` vive dentro de la red Docker propia de cada célula; alcanzarla desde un
  cron del anfitrión exigiría publicar un puerto o usar `docker exec`, debilitando el aislamiento por
  célula que ancla NFR-05 y la tarea 17 de la etapa A-6. Además, el propósito de este vigilante es
  probar que el ANFITRIÓN está vivo, no las células: mezclar ambas cosas volvería la ausencia de
  ping ambigua entre "el anfitrión murió" y "una célula está caída". (4) **Reintento o backoff
  local**: la tolerancia a un traspié transitorio es responsabilidad del período de gracia del
  servicio externo; un reintento local permitiría que un anfitrión degradado siguiera pareciendo sano
  al cruzar el límite del intervalo de 5 minutos del cron, que es exactamente la ventana que la
  alarma debe cubrir. El diseño elegido —un script `deploy/*.sh` disparado por un cron por-servidor,
  sin reintento— es el único que comparte el destino del propio anfitrión que atestigua (fate-sharing):
  si el anfitrión muere, el cron muere con él, y esa ausencia ES la alarma.
* **Registro normativo:** `deploy/ping_de_vigilancia_externa.sh`, `deploy/verificar_ping_de_vigilancia.sh`,
  `docs/runbook-vigilancia-externa.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño, no reabrir por defecto.* El
  gateo a salud de célula solo sería reabrible si una tarea futura decidiera deliberadamente
  publicar un puerto de salud por célula al anfitrión (lo que hoy NFR-05 y A-6 tarea 17 prohíben);
  el subcomando de `hexcell-admin` solo si ese crate decidiera adoptar una pila TLS por otra razón
  independiente que ya pagara ese costo.

---

### D-52

**Dos técnicas descartadas al diseñar el guardia de límites de recursos (HEX-078): la forma
explícita `soft`/`hard` de `ulimits.nofile`, y la comprobación de "campo presente" en lugar de la
igualdad exacta contra el referente.**

* **Descartado:** 2026-09-13 (HEX-078).
* **Por qué se descartó:** son dos alternativas estudiadas y rechazadas en el mismo diseño,
  agrupadas bajo un solo número siguiendo el precedente de D-28 y D-51. (1) **Forma explícita
  `soft`/`hard` de `ulimits.nofile`**: permite un límite blando distinto del duro, pero una banda
  elástica es flexibilidad muerta en una célula contenida cuyo único objetivo es no agotar
  descriptores; la forma corta (un solo escalar fija blando == duro) resuelve en `docker compose
  config` a un entero plano (`nofile: 1024`, medido 2026-09-13) y expresa exactamente la intención.
  (2) **Comprobación de "campo presente" en el guardia**: comparar solo que `mem_limit`/`cpus`/
  `ulimits.nofile` existan en el YAML resuelto es estrictamente más débil que la igualdad exacta —
  no detecta un valor que se desvía en silencio del referente (una memoria que deja de sumar 80 MB,
  una CPU fuera de lo decidido, un nofile que cambia sin anotarlo), que es justo el modo de fallo
  que la tarea 6 existe para impedir. El guardia compara en igualdad exacta y, además, convierte el
  sufijo `<N>m` del referente a bytes porque compose resuelve `mem_limit` a una cadena de bytes
  crudos ("50331648" para 48m).
* **Registro normativo:** `deploy/verificar_limites.sh` (comparación exacta), `deploy/cell.compose.yml` y `deploy/celula.env.ejemplo` (forma corta de `ulimits.nofile` y valores 1024).
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño, no reabrir por defecto.* La forma
  `soft`/`hard` solo se reconsideraría si una tarea futura necesitara deliberadamente un tope blando
  distinto del duro (hoy no existe ningún caso); la presencia solo si se quisiera aceptar límites no
  fijados en el referente, que es exactamente lo contrario de lo que esta tarea decide.
### D-53

**Bibliotecas externas de análisis de argumentos (`clap`, `argh`, `pico-args`, `structopt`) para
la CLI `hexcell-admin`.**

* **Descartado:** 2026-09-14 (HEX-074-c).
* **Por qué se descartó:** son cuatro alternativas distintas, agrupadas bajo un solo número
  siguiendo el precedente de D-28 y D-51, porque las cuatro se estudiaron y rechazaron en el
  mismo diseño y ninguna sobrevive sola. (1) **`clap`**: árbol de dependencias grande
  (`clap_builder`, `clap_lex`, `anstream`, `anstyle`, `clap_derive` con `syn` completo) y
  modelo de `Command`/`Arg` con atributos de derivación ajeno a la disciplina de enumerados
  cerrados sin `#[non_exhaustive]` que ya siguen `EstadoDeCelula`, `CodigoDeSalida` y
  `TransicionInvalida` en este mismo crate. (2) **`argh`**: más ligero que `clap` pero
  introduce una dependencia nueva en un crate cuyo punto de diseño es casi-cero
  dependencias (solo `serde` + `serde_json`, justificados en el `Cargo.toml` por el análisis
  del JSON del motor Docker) y duplica la superficie que ya resuelve a mano
  `crates/hexcell/src/emparejar.rs` para `hexcell emparejar`. (3) **`pico-args`**: la más
  pequeña de las cuatro y la más cercana a un analizador a mano, pero sigue siendo una
  dependencia externa para un trabajo que el proyecto ya sabe hacer —el precedente de
  `emparejar.rs` lo demuestra— y que, además, necesita reglas de validación específicas por
  subcomando (opciones obligatorias, opciones rechazadas por subcomando, confirmación de
  destructivos) que `pico-args` no modela y que habría que escribir de todos modos
  alrededor. (4) **`structopt`**: *wrapper* derivacional sobre `clap`; hereda todo su árbol
  de dependencias y su modelo de `Command`/`Arg` sin aportar nada que justifique el coste;
  el propio proyecto lo considera obsoleto y recomienda migrar a `clap` v4 con derivación.
  El analizador a mano sobre `std::env::args` que entrega HEX-074-c
  (`crates/hexcell-admin/src/argumentos.rs`) cumple las mismas reglas de gramática, expone
  los mismos errores tipados y se deja ejercitar desde las pruebas externas con un
  `Vec<String>` propio, sin sumar ninguna dependencia al `Cargo.toml` de `hexcell-admin` y
  sin romper la línea de minimización de dependencias abierta por `adr-0019`.
* **Registro normativo:** `crates/hexcell-admin/src/argumentos.rs`,
  `crates/hexcell-admin/tests/argumentos.rs`,
  `docs/adr/adr-0036-contrato-de-analisis-de-argumentos-y-modo-de-simulacion.md`.
* **Qué tendría que cambiar para reabrirlo:** *principio de diseño, no reabrir por defecto.*
  Solo sería reabrible si una tarea futura añadiera un subcomando con una gramática
  suficientemente compleja (subcomandos anidados, opciones de valor múltiple, completado
  automático para shell) que el analizador a mano dejara de cubrir de forma legible; incluso
  en ese caso, la reapertura tendría que justificar por qué la extensión se hace con una
  biblioteca externa y no con un segundo módulo de análisis dentro del mismo crate, siguiendo
  la línea de `adr-0019`.
### D-54: Alerta de bucle de reinicios de contenedores en HEX-077-b

**Descartado el:** 2026-09-13  
**Decisión registrada en:** `docs/plan/fase-a-6-empaquetado-cli.md` (la línea «Alertas activas») y `docs/adr/adr-0037-condiciones-de-alerta-sobre-senales-existentes.md`

### Qué se consideró

Incluir la octava condición de alerta de la tarea 20 del plan —bucle de reinicios de cualquiera de
los dos contenedores (núcleo o sidecar)— entre las siete condiciones que HEX-077-b entrega.

### Por qué se descartó

No existe ningún productor de señal para contar o persistir reinicios de contenedor en el
repositorio: ni `crates/hexcell` ni el sidecar cuentan ni persisten reinicios, y la política de
reinicio de Docker no es observable por la aplicación. Construir uno aquí violaría el non-goal de
HEX-077-b («no añadir ningún productor de señal que no exista ya»). Leer el estado de reinicio de
Docker o persistir conteos de arranque es un problema distinto que merece su propio blueprint.

### Qué tendría que cambiar para reabrirlo

Que una tarea futura decida construir un observador de reinicios de contenedor (por ejemplo, un
contador persistido en volumen que el entrypoint del contenedor incrementa en cada arranque, o una
integración con la Docker API del anfitrión). Esa tarea definiría la señal, su ubicación y su
coste; HEX-077-b entonces la consumiría como las demás.

### D-55

**Reutilizar `crates/hexcell/tests/carga.rs` como generador externo de carga contra la célula
compuesta en vivo (HEX-079, tarea 16 de la etapa A-6).**

* **Descartado:** 2026-09-19 (HEX-079).
* **Por qué se descartó:** el criterio de aceptación revisado de la tarea 16 pedía medir bajo la
  carga de `carga.rs`, pero el arnés no puede servir como generador externo sin modificarlo, y el
  spec de HEX-079 prohíbe modificarlo (non-goal explícito). Cuatro motivos independientes,
  verificados sobre el archivo: (1) construye el `Motor` en-proceso y no tiene ningún cliente de
  red ni de IPC que apuntar contra una célula viva; (2) conduce `AdaptadorSimulado`, el adaptador
  simulado, lo que contradice la exigencia de medir la célula compuesta con el adaptador whatsmeow;
  (3) usa `RelojDePrueba`, un reloj falso, para hacer determinista la admisión GCRA, y en un
  contenedor en marcha no existe un reloj falso; (4) mide la memoria residente del proceso
  (`leer_vm_rss_kb`) leyendo el archivo de estado del proceso en /proc, exactamente la fuente que
  el invariante central de HEX-079 prohíbe. En su lugar, `deploy/medir_memoria_y_imagenes.sh`
  declara la limitación (bloque LIMITACION) y genera la carga con un contenedor auxiliar efímero
  alpine:3 unido al espacio de nombres de red del núcleo; la cifra bajo carga es una cota inferior,
  no el peor caso.
* **Registro normativo:** `deploy/medir_memoria_y_imagenes.sh`, `docs/plantilla-celula.md`
  (valores de referencia), `docs/plan/fase-a-6-empaquetado-cli.md` (tarea 16).
* **Qué tendría que cambiar para reabrirlo:** que exista un arnés de carga que sea un cliente de
  red o de IPC capaz de inyectar eventos a una célula viva (sin reloj falso y sin medir vía /proc);
  entonces el generador sustituto pasaría a ser la alternativa descartada y la medición bajo carga
  real podría reemplazar a la cota inferior.

---

## D-55

**Parser externo para la configuración de células.**

**Descartado el:** 2026-09-19

### Qué se consideró

Usar TOML, YAML u otra biblioteca externa para leer los archivos de configuración de las células.

### Por qué se descartó

El contrato requiere el formato KEY=VALUE y el árbol ya decidió mantener manual el análisis de
argumentos. Añadir otro parser aumentaría dependencias para una gramática deliberadamente pequeña,
sin aportar validación que el esquema cerrado no pueda expresar.

### Qué tendría que cambiar para reabrirlo

Que la configuración dejara de ser KEY=VALUE y exigiera una estructura anidada cuya complejidad
justificase una dependencia, con una revisión explícita del contrato y del presupuesto de dependencias.

---

## Deuda de esta bitácora

Tres descartes **no tienen ningún registro documental** y solo sobreviven en el historial de git:
**D-03** (el plan mono-canal original completo, borrado sin explicación), **D-13** (la alternativa de
encolado ante `FueraDeVentana`) y **D-14** (los renombres). D-03 es el más costoso: se perdió el
motivo por el que se abandonó un plan entero de ocho etapas.

Es exactamente el agujero que este documento existe para no volver a abrir. **A partir de ahora, todo
descarte se anota aquí en el mismo commit en que se descarta.**
