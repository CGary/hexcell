# Cotejo del puerto de canal contra la documentación oficial de la Meta Cloud API

* **Fecha del cotejo:** 2026-07-29.
* **Etapa:** A-1, tarea 1 (declaración del puerto). Requisito: **FR-12**.
* **Objeto cotejado:** `crates/hexcell-core/src/canal.rs` y `crates/hexcell-core/src/identidad.rs`.
* **Estado:** vigente, con **un hallazgo abierto** (sección 4) que se escala como decisión de
  producto y **no** se resuelve aquí.

---

## Por qué existe este documento

El plan de la etapa A-1 obliga a cotejar cada variante del resultado de envío y el evento entrante
canónico **contra la documentación oficial de la Meta Cloud API, no solo contra el PRD**. El motivo
es concreto: el PRD es la fuente normativa del producto, pero no es la fuente de verdad sobre la API
de un tercero. Si el puerto se coteja únicamente contra el PRD, cualquier error que el PRD arrastre
desde su redacción se traslada intacto al código, y además queda blindado, porque a partir de
entonces el código y el documento coinciden y nadie vuelve a mirar.

El vehículo es un archivo versionado y no el cuerpo de un commit o de una propuesta de cambio: un
cotejo que solo vive en el historial no se lee desde un clon limpio y no se puede volver a verificar
cuando la API cambie. Este mismo documento recoge además, en la sección 5, los dos experimentos que
demuestran que las pruebas **muerden** en lugar de limitarse a existir.

## Fuentes consultadas el 2026-07-29

* Códigos de error de la Cloud API —fuente principal de este cotejo—:
  <https://developers.facebook.com/docs/whatsapp/cloud-api/support/error-codes/>
* Ejemplos de carga útil de los webhooks:
  <https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/payload-examples/>
* Componentes de los webhooks:
  <https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/components/>

Las comillas de esta página reproducen la redacción oficial en inglés tal y como aparece en esas
fuentes. La prosa del repositorio es española; el texto citado no se traduce, para que el cotejo
siga siendo verificable palabra por palabra.

---

## 1. Cotejo del resultado tipado del envío (`ResultadoEnvio`)

FR-12 fija cinco desenlaces: un éxito y cuatro fallos. Este es su anclaje en la documentación
oficial.

| Variante | Anclaje oficial | Veredicto |
| :--- | :--- | :--- |
| `Aceptado` | Respuesta de `POST /messages` sin objeto de error. El desenlace real llega después por el webhook de estado. | Correcta. El éxito de la llamada no es el éxito de la entrega, y por eso los acuses son un elemento aparte. |
| `FueraDeVentana` | **131047** — *"More than 24 hours have passed since the recipient last replied to the sender number"*. | Correcta, con el matiz de la sección 2: es el **único** código de ventana cerrada. |
| `PlantillaRequerida` | Sin código propio en la documentación oficial. | Correcta como **veredicto previo del adaptador**, no como transcripción de un error de la API. Ver sección 2. |
| `LimiteDeTasa` | **130429** — *"Cloud API message throughput has been reached"*; **80007** — *"The WhatsApp Business Account has reached its rate limit"*; **4** — *"The app has reached its API call rate limit"*. | Correcta. Tres códigos distintos con la misma reacción del núcleo: reintentar más tarde con retroceso. Colapsarlos en una variante no pierde información accionable. |
| `DestinatarioInvalido` | **131026** — *"Unable to deliver message"* (entre sus causas, que el destinatario no use WhatsApp, tenga una versión obsoleta o no haya aceptado las políticas); **131021** — *"Sender and recipient phone number is the same"*. | Correcta, con la nota de precisión de más abajo. |

**Nota de precisión sobre 131026.** El código agrupa causas heterogéneas y solo algunas son
estrictamente "el destinatario no sirve". Aun así, todas comparten la misma reacción del núcleo —no
reintentar el mismo envío al mismo destino— que es el criterio con el que FR-12 justifica un
resultado tipado. La nota queda escrita para que la etapa A-3, cuando el adaptador tenga que
traducir el código real, sepa que está agrupando y no descubriéndolo entonces. **No exige variante
nueva.**

---

## 2. Primera discrepancia, resuelta: 131047 es el único código de ventana cerrada

Leyendo solo el PRD, `FueraDeVentana` y `PlantillaRequerida` parecen dos fallos distintos del canal
oficial. La documentación oficial no los distingue: **131047 es el único código publicado para la
ventana de servicio de 24 horas cerrada**. No existe un código separado que signifique "esto
requería una plantilla".

Esa es una discrepancia real entre la lectura ingenua del PRD y la API, y el puerto la resuelve así,
sin tocar el conjunto de variantes que FR-12 fija:

* **`FueraDeVentana` refleja una respuesta real de la API.** Es lo que el adaptador devuelve cuando
  ha habido viaje de ida y vuelta y la Cloud API ha contestado 131047.
* **`PlantillaRequerida` es el veredicto de la política **previa al vuelo** del adaptador.** Cuando
  el adaptador ya sabe —por su propio estado de ventana— que la ventana está cerrada y el mensaje
  que se le entrega es `RespuestaLibre`, no llega a llamar a la API: devuelve `PlantillaRequerida`
  directamente, porque **esa es la variante que le dice al núcleo qué hacer en su lugar**, que es
  enviar una plantilla aprobada. Devolver `FueraDeVentana` describiría el estado del mundo; devolver
  `PlantillaRequerida` describe la salida.

Esto es una **decisión de diseño, no una transcripción**. Si alguien no está de acuerdo, el
desacuerdo se discute aquí y en `adr-0002-estructura-workspace.md`, nunca cambiando el enumerado en
silencio: su conjunto de variantes lo fija FR-12 y ampliarlo o reducirlo es una decisión sobre el
PRD.

Consecuencia práctica para el adaptador del canal propio: **ninguna**. Su transporte no impone
ventana, su estado de ventana es siempre abierta y por tanto nunca produce ninguno de los dos
resultados. El tipo admite el resultado restrictivo; la política de cada adaptador decide si lo
produce.

---

## 3. Cotejo del evento entrante canónico (`EventoEntrante`)

Los cinco campos que FR-12 enumera, contra los campos que la Cloud API entrega en el webhook de
mensaje entrante.

| Campo del dominio | Origen en la Cloud API | Veredicto |
| :--- | :--- | :--- |
| `remitente: IdRemitente` | `contacts[].wa_id` y `messages[].from`, ambos identificadores de transporte. | Correcto **solo porque el adaptador traduce**. El identificador crudo no cruza la frontera; el núcleo recibe identidad interna. |
| `conversacion: IdConversacion` | La Cloud API **no entrega** un identificador de hilo en el mensaje entrante. El campo `statuses[].conversation.id` existe, pero pertenece al webhook de estado y su función es de facturación, no de identidad de hilo. | Correcto, y es la razón de que el mapeo tenga que existir: la conversación la **asigna el adaptador** desde su tabla, no la lee del transporte. |
| `contenido: String` | `messages[].text.body`. | Correcto para mensajes de texto. Los tipos no textuales —imagen, audio, ubicación, interactivos— no se modelan en la etapa A-1; entran cuando la etapa A-2 defina qué hace el núcleo con ellos. |
| `marca_temporal: SystemTime` | `messages[].timestamp`, marca Unix entregada como cadena. | Correcto. La conversión a tiempo absoluto es trabajo del adaptador; el dominio no manipula formatos del transporte. |
| `deduplicacion: IdDeduplicacion` | `messages[].id`, el identificador de mensaje de la API. | Correcto. El núcleo solo lo compara consigo mismo para descartar reentregas; no lo interpreta. |

**Cotejo de los acuses (`Acuse`).** La documentación de webhooks describe explícitamente que cada
mensaje saliente puede generar hasta tres webhooks de estado: `sent`, `delivered` y `read`. El
cuarto estado que FR-12 normaliza, `Fallido`, llega por el mismo objeto `statuses` acompañado de su
array `errors`. **Límite de verificación declarado:** las páginas consultadas el 2026-07-29 no
publican la lista cerrada de valores admitidos del campo `statuses[].status`, de modo que el anclaje
de `Fallido` se apoya en la presencia del array de errores y no en un literal confirmado. Se deja
escrito en lugar de darlo por bueno: la etapa A-3 lo confirmará con tráfico real, y ese es el
momento honesto de cerrarlo.

**Cotejo del mensaje saliente (`MensajeSaliente`).** La distinción `RespuestaLibre` / `Plantilla` es
correcta y necesaria: fuera de la ventana la API solo acepta plantillas aprobadas. Los parámetros se
modelan hoy como `Vec<String>` posicionales, que es la forma mínima suficiente para la etapa A-1; el
modelo de componentes completo de la Cloud API (encabezado, cuerpo, botones) se decide cuando exista
un adaptador que lo necesite. Queda escrito como limitación conocida, no como cotejo favorable.

---

## 4. Segunda discrepancia: **hallazgo ABIERTO**, sin variante nueva

Hay una familia de códigos oficiales que **no encaja limpiamente en ninguna de las cuatro
variantes** que FR-12 enumera.

**Familia de fallos de plantilla:**

| Código | Redacción oficial | Por qué no encaja |
| :--- | :--- | :--- |
| **132000** | *"The number of variable parameter values included in the request did not match the number of variable parameters defined in the template"* | Es un error del emisor, corregible reintentando con los parámetros correctos. No es "se requiere plantilla": ya se envió una. |
| **132001** | *"The template does not exist in the specified language or the template has not been approved"* | Fallo de configuración del canal. Reintentar no arregla nada; hace falta intervención humana en el panel de Meta. |
| **132015** | *"Template is paused due to low quality so it cannot be sent in a template message"* | La plantilla existe y está aprobada, pero está suspendida temporalmente. La reacción correcta es usar otra plantilla, no reintentar. |
| **132016** | *"Template has been paused too many times due to low quality and is now permanently disabled"* | Como el anterior, pero definitivo. Reintentar con esa plantilla no volverá a funcionar nunca. |

**Códigos de ecosistema y de reputación:**

| Código | Redacción oficial | Por qué no encaja |
| :--- | :--- | :--- |
| **131049** | *"This message was not delivered to maintain healthy ecosystem engagement"* | No es límite de tasa, no es ventana, no es destinatario inválido y no es plantilla. Es una decisión de la plataforma sobre la calidad del tráfico. |
| **131048** | Restricción del envío desde el número por mensajes bloqueados o marcados por los destinatarios. | Se parece a `LimiteDeTasa` solo en que el mensaje no sale; la causa es de reputación y la reacción sensata no es reintentar más tarde, sino dejar de enviar y avisar. |

**Qué se hace con esto: nada, y esa es la decisión.** Este cotejo **no añade una quinta variante**.
El conjunto de variantes de `ResultadoEnvio` lo fija `docs/PRD.md` (FR-12) y ampliarlo es una
decisión de producto sobre el PRD, no una decisión de implementación que se pueda tomar de pasada
mientras se escribe un esqueleto de tipos. El hallazgo queda **abierto** y escalado:

* Registrado aquí, con los códigos concretos y el motivo de cada desencaje.
* Registrado como decisión pendiente en `docs/STATUS.md`, para que no dependa de que alguien
  recuerde leer este archivo.

**Consecuencia mientras siga abierto.** Un adaptador de Cloud API que reciba hoy cualquiera de estos
seis códigos tendría que plegarlos sobre una de las cuatro variantes existentes o sobre el error de
transporte del tipo asociado, y en ambos casos se pierde información accionable. No se elige aquí
cuál de las dos salidas es la buena, precisamente porque elegirla sería resolver la decisión
pendiente por la puerta de atrás. Lo que sí queda fijado es el plazo natural: **la decisión tiene
que estar tomada antes de que la etapa B-1 escriba el adaptador oficial**, porque ese es el primer
momento en que estos códigos pueden llegar de verdad. Sobre canal propio no llegan: whatsmeow no
tiene plantillas ni ventana.

---

## 5. Registro de los experimentos

Una prueba que existe no demuestra nada. Lo que se registra aquí es que **muerde cuando se le
provoca**. Los cuatro experimentos se ejecutaron el 2026-07-29 sobre `rustc 1.92.0` y
`cargo 1.92.0`, en la rama `ai/HEX-002`, y todos se revirtieron con `git checkout --` sobre el
archivo tocado. El árbol quedó limpio y en verde al terminar, comprobado con `cargo test
--workspace` y `cargo fmt --check`.

### 5.1 El `match` exhaustivo muerde al **añadir** una variante

* **Qué se introdujo:** una variante `PlantillaPausada` al final de `ResultadoEnvio`, en
  `crates/hexcell-core/src/canal.rs`.
* **Qué falló:** la compilación de las **pruebas**, no la del crate.

```
error[E0004]: non-exhaustive patterns: `ResultadoEnvio::PlantillaPausada` not covered
   --> crates/hexcell-core/tests/exhaustividad_resultado_envio.rs:46:11
error: could not compile `hexcell-core` (test "exhaustividad_resultado_envio") due to 1 previous error
```

* **Cómo se revirtió:** `git checkout -- crates/hexcell-core/src/canal.rs`.

### 5.2 El `match` exhaustivo muerde al **quitar** una variante

* **Qué se introdujo:** la eliminación de la variante `LimiteDeTasa` de `ResultadoEnvio`.
* **Qué falló:** de nuevo la compilación de las pruebas, en los tres puntos donde la variante se
  nombra.

```
error[E0599]: no variant or associated item named `LimiteDeTasa` found for enum `ResultadoEnvio` in the current scope
  --> crates/hexcell-core/tests/exhaustividad_resultado_envio.rs:27:21
error: could not compile `hexcell-core` (test "exhaustividad_resultado_envio") due to 3 previous errors
```

* **Cómo se revirtió:** `git checkout -- crates/hexcell-core/src/canal.rs`.

Que estos dos errores aparezcan en `tests/` y no en `src/` es justamente el efecto buscado: las
pruebas viven en un **crate externo**, ven el enumerado como cerrado y por eso pueden recorrerlo sin
brazo comodín. Si el enumerado se declarase abierto, el crate externo estaría obligado a escribir un
brazo comodín y estos dos experimentos **no fallarían**: el guardián existiría y no mordería.

### 5.3 El guardián muerde ante la filtración estructural

* **Qué se introdujo:** un campo visible con nombre de identificador de transporte en un tipo de
  dominio —`pub wa_id: String` en `EventoEntrante`—, que es exactamente la filtración que
  `adr-0010` prohíbe.
* **Qué falló, por dos caminos independientes:**

1. La compilación de la prueba guardián, porque construye el evento canónico con todos sus campos:

```
error[E0063]: missing field `wa_id` in initializer of `EventoEntrante`
   --> crates/hexcell-core/tests/guardian_identidad_conversacion.rs:135:18
error: could not compile `hexcell-core` (test "guardian_identidad_conversacion") due to 1 previous error
```

2. La comprobación léxica del contrato, que localizó la línea infractora y solo la infractora,
   dejando pasar la prosa de los comentarios que menciona legítimamente `wa_id` y JID:

```
crates/hexcell-core/src/canal.rs:67:    pub wa_id: String,
```

* **Cómo se revirtió:** `git checkout -- crates/hexcell-core/src/canal.rs`.

### 5.4 El guardián muerde también ante la filtración **semántica**

El experimento anterior demuestra que el guardián detecta un campo mal llamado. No demuestra lo más
importante, porque una prueba puramente léxica se esquiva con solo cambiarle el nombre al campo. Por
eso se hizo un cuarto experimento, sin tocar ningún nombre.

* **Qué se introdujo:** que la tabla de mapeo devolviese el identificador de transporte **tal cual**
  como identificador interno, es decir, la función identidad como "traducción". Ninguna firma
  cambió, ningún nombre menciona el transporte y la comprobación léxica sigue pasando.
* **Qué falló:** dos pruebas del guardián, en ejecución:

```
thread 'el_identificador_interno_no_contiene_rastro_del_transporte' panicked at
crates/hexcell-core/tests/guardian_identidad_conversacion.rs:120:13:
el identificador interno IdConversacion("5491122334455@s.whatsapp.net") filtra el fragmento 5491122334455@s.whatsapp.net

thread 'el_evento_canonico_se_construye_solo_con_identidad_traducida' panicked at
crates/hexcell-core/tests/guardian_identidad_conversacion.rs:148:9:
el evento canónico filtra el fragmento 5491122334455@s.whatsapp.net del transporte

test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out
```

* **Cómo se revirtió:** `git checkout --` sobre
  `crates/hexcell-core/tests/guardian_identidad_conversacion.rs`.

Este es el experimento que justifica que el guardián exista además de la comprobación léxica: la
regla textual se puede cumplir mientras se comete el error de diseño, y aquí está la prueba de que
la comprobación semántica lo caza igualmente.

---

## 6. Límites declarados de este cotejo

Se escriben porque un cotejo que no declara lo que no cubre invita a confiar de más en él.

* **La comprobación léxica es necesaria pero insuficiente.** Que ninguna firma nombre `wa_id`, JID,
  `msisdn` o `phone_number` no demuestra que el diseño sea correcto: el mismo error puede repetirse
  bajo otro nombre. La parte semántica la cubren el guardián (sección 5.4) y, sobre todo, los tests
  de contrato de la etapa A-2 contra el adaptador simulado.
* **Este cotejo certifica forma y anclaje documental, no comportamiento.** No hay ningún adaptador
  implementado en esta etapa, ni real ni simulado, así que nada de lo escrito aquí se ha ejercitado
  contra la API de verdad.
* **Está fechado a propósito.** La Cloud API cambia sus códigos y su documentación sin avisar al
  proyecto. Este archivo dice lo que decía la documentación oficial el 2026-07-29; cuando la etapa
  B-1 escriba el adaptador oficial, el cotejo se repite y se actualiza, no se da por bueno.
* **El canal propio no queda cotejado aquí, y es correcto que así sea.** El puerto se abstrae hacia
  el caso más restrictivo, que es la Cloud API. whatsmeow no impone ventana ni plantillas, de modo
  que su adaptador nunca produce los resultados restrictivos. Los dos canales conviven; el oficial
  se suma en células distintas cuando aparezca un cliente que lo justifique (`adr-0014`).

---

## Referencias

* `docs/PRD.md`, FR-12 — enumeración normativa de los siete elementos del puerto.
* `docs/adr/adr-0010-puerto-de-canal.md` — porqué de la frontera y dueño del mapeo de identidad.
* `docs/adr/adr-0002-estructura-workspace.md` — división en crates y consecuencias del diseño del
  trait.
* `docs/plan/fase-a-1-fundaciones.md`, tareas 1 y 9 — origen de la obligación de cotejar y de
  demostrar que el guardián muerde.
* `docs/STATUS.md` — decisión pendiente sobre la ampliación del conjunto enumerado de FR-12.
* `crates/hexcell-core/tests/exhaustividad_resultado_envio.rs` y
  `crates/hexcell-core/tests/guardian_identidad_conversacion.rs` — las pruebas cuyos experimentos
  recoge la sección 5.
