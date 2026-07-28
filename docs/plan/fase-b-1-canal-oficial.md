# Fase B · Etapa 1 — Canal oficial: adaptador Cloud API y entrada pública

**Duración relativa:** sin estimar. **La Fase B permanece sin planificar hasta que aparezca un
cliente que justifique el canal oficial**; sus etapas se describen en alcance y dependencias, no en
días de trabajo.

---

## Objetivo

Esta etapa es la razón por la que existe el puerto de canal. Si el diseño de la etapa A-1 es correcto,
**añadir** el canal oficial debe ser **escribir un segundo adaptador**, no reescribir el producto.
Esta etapa lo demuestra o lo refuta.

Desde el 28 de julio de 2026 (`adr-0014`) esta etapa **no es una migración, sino una incorporación
aditiva**. El
canal oficial (Cloud API) **se añade y convive** con el canal propio (whatsmeow): **no lo sustituye,
no obliga a migrar a ninguna célula existente y no retira el sidecar** de las células que siguen
sobre el canal propio, donde el sidecar es permanente. Lo que abre esta etapa ya no es una compuerta
por número de clientes, sino **la aparición de un cliente que justifique el canal oficial**
—típicamente una empresa medianamente grande capaz de asumir el alta y el coste—.

El canal oficial invierte la dirección de la conexión. Sobre el canal propio el servidor abre un
websocket saliente hacia WhatsApp y no necesita nada del mundo exterior. Sobre el canal oficial es
Meta quien llama: los mensajes llegan como **webhooks HTTPS entrantes**, lo que obliga a que exista
una entrada pública con un certificado válido, dirigida a un servidor que vive detrás de un router
doméstico. Ese es el problema que esta etapa resuelve, y tiene dos soluciones con consecuencias muy
distintas.

Además del adaptador y de la entrada, esta etapa conserva el **procedimiento de migración de una
célula** del canal propio al oficial, ahora **opcional y por demanda**: se ejecuta cuando un cliente
concreto decide cambiar de canal, no por defecto. El procedimiento no se descarta, porque sigue
haciendo falta y porque sigue siendo la prueba definitiva de la frontera entre el núcleo y el
transporte: si el historial conversacional sobrevive intacto al cambio de canal, la decisión de no
persistir nunca identificadores de transporte crudos en `sessions.db` habrá valido cada minuto
invertido.

---

## Alcance

### Qué entra

* **Adaptador de Cloud API** implementando el trait `ChannelAdapter` (FR-12):
  * Receptor de webhooks con el desafío de suscripción (`hub.mode`, `hub.verify_token`,
    `hub.challenge`), con **comparación en tiempo constante** del token de verificación.
  * Validación de firma **HMAC-SHA256** (`X-Hub-Signature-256`) sobre el cuerpo crudo recibido, sin
    reserializar.
  * Respuesta `HTTP 200 OK` inmediata antes de procesar, con encolado del trabajo real.
  * Traducción del payload de Meta al evento entrante canónico, con su identificador de deduplicación.
  * Mapeo del `wa_id` al identificador interno de conversación, **dentro del adaptador**.
  * Envío mediante la API de mensajes, y traducción de los estados de entrega de Meta a los acuses
    normalizados `sent`/`delivered`/`read`/`failed`.
  * **No** implementa el sub-trait de ciclo de vida de sesión: la Cloud API no requiere emparejamiento
    ni persistencia de credenciales de dispositivo.
* Reincorporación del patrón ***Fast-Reject***: el módulo GCRA de la etapa A-4 se reutiliza sin
  cambios, y el adaptador añade la respuesta `HTTP 200 OK` sintética al exceso, para anular las
  tormentas de reintentos que la API Graph dispara ante códigos 429/503 (FR-08, variante de Fase B).
* **Decisión de entrada pública mediante ADR**, entre dos opciones:

  | | **Cloudflare Tunnel (capa gratuita)** | **VPS ~3 USD/mes + WireGuard** |
  | :--- | :--- | :--- |
  | Terminación TLS | En el edge de Cloudflare. | En el propio Caddy del servidor local. |
  | Dirección de la conexión | Saliente desde el servidor local, igual que en el canal propio. | Túnel WireGuard entre VPS y servidor local. |
  | Coste | Cero. | ~3 USD/mes fijos. |
  | Handshake anti-Hairpin (FR-04) | **Innecesario**: no hay certificado local que validar ni Hairpin NAT que sortear. | **Necesario**: se conserva íntegro. |
  | On-Demand TLS de Caddy (NFR-04) | **Innecesario**. | **Necesario**. |
  | Dependencia externa | Cloudflare, con los términos de su capa gratuita. | Proveedor de VPS. |
  | Arquitectura del PRD original | Se simplifica: desaparecen dos mecanismos. | Se conserva sin cambios. |

  La decisión condiciona directamente el alcance de la etapa B-2 y la vigencia de FR-04 y NFR-04.
* Aprovisionamiento de la aplicación de Meta, del WABA de prueba y de las credenciales necesarias.
* **Evaluación del modo coexistencia de Meta como opción preferente**
  (`developers.facebook.com/docs/whatsapp/embedded-signup/custom-flows/onboarding-business-app-users/`).
  Un mismo número funciona a la vez en la app de WhatsApp Business del móvil y en la Cloud API,
  sincronizando 180 días de historial y contactos, y el integrador recibe por webhook
  (`smb_message_echoes`) lo que el dueño responde a mano desde su app. Importa por dos motivos:
  desmonta el argumento de que el cliente pierde la bandeja de su móvil, y **resuelve el pendiente de
  la interfaz de intervención humana** sin construirla. Requiere **Embedded Signup de un Solution
  Partner o Tech Provider: no hay ruta de Cloud API directa**. Limitaciones que la evaluación debe
  pesar: 20 mensajes por segundo fijos, sin grupos, sin mensajes efímeros, sin vista única, sin
  ubicación en vivo, sin listas de difusión y sin catálogo ni pedidos por API.
* **Recálculo del coste por conversación de toda célula sobre canal oficial.** El 1 de julio de 2026
  Meta anunció que **desde el 1 de octubre de 2026 cobrará también los mensajes de servicio** —las
  respuestas dentro de la ventana de 24 h—, con tarifas publicables hasta el 1 de septiembre de 2026.
  Esto **invalida el supuesto anterior de que el transporte del canal oficial cuesta aproximadamente
  cero**. *Estado de la evidencia: confirmado por múltiples BSPs, todavía **no** reflejado en la
  página oficial de precios de Meta; debe tratarse con ese matiz y reverificarse antes de fijar
  ningún precio.*
* **Convivencia de ambos adaptadores vivos a la vez**, en células distintas del mismo servidor: el
  núcleo debe soportarlos de forma simultánea, y la **suite de pruebas de contrato del puerto es
  única y se ejecuta contra los dos adaptadores**, no una por canal.
* **Procedimiento de migración de una célula, opcional y por demanda**: adquisición del número o
  migración del existente al canal oficial, conmutación del adaptador y verificación de que el
  historial conversacional y el conocimiento sobreviven intactos. Se documenta y se ensaya aunque
  ninguna célula vaya a migrar todavía.
* Retirada del sidecar **únicamente en las células efectivamente migradas**, con la recuperación del
  presupuesto de memoria de NFR-01 hasta los 50 MB por célula. Las células que permanecen sobre el
  canal propio **conservan su sidecar de forma permanente** y se siguen midiendo contra su
  presupuesto de 80 MB.

### Qué NO entra

* Caddy, subdominios, blackholing y el `cell create` completo: etapa B-2.
* La implementación del Embedded Signup: etapa B-2. Aquí solo se **evalúa** el modo coexistencia y se
  decide si es la opción preferente.
* **La migración obligatoria de ninguna célula existente.** El canal propio sigue siendo el canal de
  producción por defecto; migrar es una decisión de cada cliente, célula a célula.
* **La retirada del sidecar de las células que permanecen sobre el canal propio.** Ahí es permanente
  y no se retira nunca.
* Cualquier cambio en el núcleo, la persistencia, el conocimiento o el control de admisión. Si esta
  etapa necesita tocarlos, el puerto de canal estaba mal diseñado y eso es en sí mismo un hallazgo.

### Requisitos del PRD cubiertos

* **FR-01** — implementación de la variante de Fase B: recepción y verificación de webhooks de la Meta
  Graph API.
* **FR-08** — variante de Fase B: Fast-Reject con `HTTP 200 OK` hacia Meta, sobre el mismo módulo GCRA
  ya construido.
* **FR-12** — segunda implementación del puerto, que valida el contrato, **con ambos adaptadores
  vivos a la vez en células distintas** y una única suite de contrato ejercitada contra los dos.
* **FR-04** y **NFR-04** — su vigencia queda determinada por el ADR de entrada pública.

---

## Entregables

* Implementación `CloudApiAdapter` del trait `ChannelAdapter`.
* `hexcell-meta` completado: verificación de firma, tipos del payload de webhook y cliente de envío.
* ADR de **entrada pública de la Fase B** con la decisión tomada y sus consecuencias sobre FR-04 y
  NFR-04.
* ADR del adaptador de Cloud API.
* **Evaluación escrita del modo coexistencia**, con su decisión razonada, sus limitaciones y su
  consecuencia sobre el pendiente de la interfaz de intervención humana.
* **Recálculo documentado del coste por conversación sobre canal oficial**, con la tarifa de mensajes
  de servicio vigente y la fecha de la fuente consultada.
* Entrada pública desplegada y verificada según la opción elegida.
* **Procedimiento de migración de una célula del canal propio al oficial**, documentado y ensayado al
  menos una vez, con la comprobación de continuidad del historial. Se conserva como procedimiento
  vivo aunque no haya migraciones pendientes.
* **Banco de pruebas local reutilizable (`scripts/`) capaz de emitir webhooks firmados como lo haría
  Meta**, con payloads realistas y firma HMAC válida. Es la herramienta sobre la que se apoyan las
  pruebas de firma, de deduplicación y de carga, y permite ejercitar el canal oficial sin depender de
  tráfico real de Meta.
* Pruebas: verificación de firma con vector conocido, rechazo ante alteración de un solo byte del
  cuerpo, deduplicación de reintentos de Meta y Fast-Reject bajo carga.

---

## Tareas

*(Sin estimación: la Fase B no se dimensiona hasta que aparezca el cliente que la justifique.)*

1. **Decidir la entrada pública y escribir el ADR.** Es la primera tarea porque condiciona todo lo
   demás, incluida la mitad del alcance de la etapa B-2.
2. **Evaluar el modo coexistencia y decidir si es la opción preferente.** Va al principio porque
   condiciona la ruta de alta —exige Embedded Signup de un Solution Partner o Tech Provider, sin ruta
   de Cloud API directa— y porque resuelve, o no, el pendiente de la interfaz de intervención humana.
3. **Recalcular el coste por conversación sobre canal oficial** con la tarifa de mensajes de servicio
   vigente desde el 1 de octubre de 2026, reverificándola contra la página oficial de precios de Meta
   en el momento de ejecutar la etapa. Sin este número no se puede fijar precio a un cliente sobre
   canal oficial.
4. **Desplegar y verificar la entrada pública elegida.** Certificado válido, alcance desde fuera y
   estabilidad del túnel.
5. **Implementar el receptor de webhooks** con el desafío de verificación —comparando el token en
   tiempo constante, para no filtrar información por el tiempo de respuesta— y la respuesta `200 OK`
   inmediata anterior al procesamiento.
6. **Implementar la validación de firma HMAC-SHA256** sobre el cuerpo crudo, antes de cualquier
   deserialización.
7. **Traducir el payload de Meta al evento canónico** y mapear el `wa_id` al identificador interno
   dentro del adaptador.
8. **Implementar el envío y la traducción de acuses** a `sent`/`delivered`/`read`/`failed`.
9. **Reincorporar el Fast-Reject** sobre el módulo GCRA existente, sin modificarlo.
10. **Ejecutar la suite de contrato del puerto contra ambos adaptadores** y levantar en el mismo
    servidor una célula sobre canal propio y otra sobre canal oficial, comprobando que conviven sin
    interferirse.
11. **Documentar y ensayar el procedimiento de migración** de una célula del canal propio al oficial,
    verificando la continuidad del historial conversacional. Se ensaya aunque ninguna célula vaya a
    migrar: el procedimiento debe existir el día que un cliente lo pida.
12. **Migrar por demanda** las células cuyo cliente lo solicite, de una en una y esperando a que la
    anterior lleve tiempo estable sobre el canal oficial. Ninguna célula se migra por defecto.
13. **Retirar el sidecar solo de las células efectivamente migradas** y volver a medir su consumo de
    memoria contra el presupuesto de 50 MB. Las células sobre canal propio conservan el sidecar y su
    presupuesto de 80 MB.

---

## Criterios de aceptación

* Un `GET /webhook` con el token correcto devuelve el `hub.challenge` tal cual; con token incorrecto
  devuelve un error y nunca el desafío. La comparación del token se realiza en **tiempo constante**,
  de modo que el tiempo de respuesta no revele cuántos caracteres iniciales del token son correctos.
* Un `POST /webhook` con firma válida responde `200 OK` y el evento queda registrado; con firma
  inválida se rechaza y no se escribe nada.
* Alterar un solo byte del cuerpo invalida la firma y la petición se rechaza.
* Reenviar el mismo webhook dos veces produce un único registro conversacional.
* El `wa_id` **no aparece** en ninguna estructura del núcleo ni en `sessions.db`.
* Bajo sobrecarga, el exceso recibe `HTTP 200 OK` rápido y en ningún escenario se devuelve `429`,
  `502` o `503` hacia Meta.
* **El núcleo, la persistencia, el conocimiento y el módulo de admisión no han requerido ninguna
  modificación** para soportar el canal oficial. Este criterio no admite excepción documentada como
  deuda: si el adaptador de Cloud API exige tocar el núcleo, la etapa **no se acepta**. El trabajo
  se detiene, la desviación se analiza, el contrato del puerto se corrige mediante una revisión
  explícita del ADR-0010 (un nuevo ADR o uno que lo supere), y solo entonces la etapa puede cerrarse
  con este criterio cumplido de verdad. Es la prueba de fuego de toda la estrategia de dos fases: si
  falla en silencio, la Fase A construyó sobre una premisa falsa.
* **El núcleo soporta ambos adaptadores vivos a la vez**, en células distintas del mismo servidor, y
  una **única suite de pruebas de contrato del puerto se ejecuta contra los dos** y pasa en ambos. No
  se aceptan dos suites divergentes, una por canal: eso sería bifurcar el puerto de facto.
* Levantar una célula sobre canal oficial **no altera el funcionamiento ni el presupuesto de memoria
  de ninguna célula sobre canal propio**, que conserva su sidecar.
* Tras migrar una célula, su historial conversacional y su conocimiento siguen siendo accesibles y
  correctos, y las conversaciones anteriores se continúan sin ruptura. El procedimiento de migración
  está documentado y ensayado al menos una vez, con independencia de cuántas células migren.
* El consumo de memoria de una célula migrada, ya sin sidecar, es inferior a 50 MB en reposo. Las
  células que permanecen sobre canal propio se siguen midiendo contra su presupuesto de 80 MB, con el
  sidecar incluido.
* Existe una **evaluación escrita del modo coexistencia** con su decisión razonada y sus limitaciones,
  y consta si resuelve o no el pendiente de la interfaz de intervención humana.
* El **coste por conversación sobre canal oficial está recalculado** con la tarifa de mensajes de
  servicio vigente, con la fecha de consulta de la fuente y con la salvedad de que la página oficial
  de precios de Meta podía no reflejarla todavía.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| El puerto de canal resulta insuficiente y el adaptador obliga a tocar el núcleo. | Muy alto: la premisa de las dos fases falla y el coste de la Fase B se dispara. | Diseño del puerto con ambos canales delante desde la etapa A-1: la abstracción hacia el caso más restrictivo (semántica de la Cloud API) y sus tests de contrato son la garantía de compatibilidad, no una firma anticipada — `hexcell-meta` nace vacío hasta que se resuelva el ADR-0013. Si aun así ocurre, no se acepta la etapa: se detiene el trabajo, se analiza la desviación y se corrige el contrato mediante una revisión explícita del ADR-0010. |
| La opción de entrada pública se elige por coste sin evaluar sus consecuencias. | Alto: se arrastra o se descarta indebidamente FR-04 y NFR-04. | ADR obligatorio con la tabla de tradeoffs, decidido antes de cualquier despliegue. |
| Dependencia de la capa gratuita de Cloudflare y de sus términos. | Medio: un cambio de política obligaría a migrar la entrada. | Aislar la entrada tras una frontera clara, de modo que cambiar de opción no toque el adaptador. |
| Aprobación de la aplicación de Meta más lenta de lo previsto. | Medio: retrasa la incorporación del canal oficial y, con ella, al cliente que la justificaba. | Iniciar el trámite en cuanto aparezca ese cliente, en paralelo al trabajo técnico. |
| La migración de una célula rompe su historial. | Alto: pérdida de contexto conversacional de clientes reales. | Respaldo previo obligatorio con restauración verificada, comprobación de continuidad como criterio de aceptación, y ensayo del procedimiento antes de aplicarlo a una célula de cliente. |
| **Se trata esta etapa como una migración y se arrastra a células que no la necesitan.** | Alto: se rompe lo que funciona, se retira un sidecar que es permanente sobre canal propio y se traslada al cliente un coste de transporte que no había pedido. | El canal oficial **se añade, no sustituye**: la migración es **opcional y por demanda**, la retirada del sidecar se limita a las células efectivamente migradas, y los criterios de aceptación miden los dos presupuestos de memoria por separado. |
| **El coste del canal oficial se calcula con el supuesto derogado de "transporte ≈ 0".** | Alto: se fija precio por debajo del coste real desde el primer cliente. | Recálculo obligatorio (tarea 3) con la tarifa de mensajes de servicio vigente desde el 1 de octubre de 2026, reverificada en el momento de ejecutar la etapa y con la salvedad de que la página oficial de precios podía no reflejarla. |
| **Los dos adaptadores divergen y el puerto se bifurca de facto.** | Muy alto: se pierde justo la propiedad que justificaba el puerto, y con retraso, porque nada falla de golpe. | Suite de contrato **única** ejecutada contra ambos adaptadores, con ambos vivos a la vez en el mismo servidor, como criterio de aceptación. |
| Reserializar el cuerpo del webhook antes de validar la firma. | Alto: la firma falla de forma intermitente e inexplicable. | Validar siempre sobre los bytes crudos recibidos, antes de cualquier deserialización, con prueba explícita. |
| Procesar el mensaje antes de responder a Meta. | Alto: se superan los tiempos de espera y se dispara la tormenta de reintentos. | Responder `200 OK` y encolar; la prueba de integración mide el tiempo hasta la respuesta. |

---

## Dependencias

* **De otras etapas:** **la aparición de un cliente que justifique el canal oficial.** La condición
  anterior —la compuerta del tercer cliente de la etapa A-7, de `adr-0008`— queda **derogada el 28 de
  julio de 2026 por `adr-0014`**. Requiere además todas las etapas de la Fase A completas y en operación, incluido el canal
  propio funcionando, que no se detiene por esta etapa.
* **Externas:** una aplicación de Meta aprobada con acceso a la Cloud API, un WABA, —según la opción
  elegida— una cuenta de Cloudflare o un VPS contratado, y —si se adopta el modo coexistencia— un
  **Solution Partner o Tech Provider** que provea el Embedded Signup, porque no hay ruta de Cloud API
  directa. Requiere además la **tarifa de mensajes de servicio de Meta publicada** para poder cerrar
  el coste por conversación.
* **Decisiones de producto pendientes:** el **modelo de monetización**, ahora con un componente
  adicional: el transporte del canal oficial deja de costar aproximadamente cero desde el 1 de
  octubre de 2026, de modo que una célula sobre canal oficial y una sobre canal propio tienen
  estructuras de coste distintas y no admiten el mismo precio sin analizarlo.
