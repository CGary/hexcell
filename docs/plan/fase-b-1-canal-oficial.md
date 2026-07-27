# Fase B · Etapa 1 — Canal oficial: adaptador Cloud API y entrada pública

**Duración relativa:** sin estimar. **La Fase B está CONGELADA hasta la compuerta del tercer
cliente**; sus etapas se describen en alcance y dependencias, no en días de trabajo.

---

## Objetivo

Esta etapa es la razón por la que existe el puerto de canal. Si el diseño de la etapa A-1 es correcto,
pasar del canal no oficial al oficial debe ser **escribir un segundo adaptador**, no reescribir el
producto. Esta etapa lo demuestra o lo refuta.

El cambio de canal invierte la dirección de la conexión. En la Fase A el servidor abría un websocket
saliente hacia WhatsApp y no necesitaba nada del mundo exterior. En la Fase B es Meta quien llama:
los mensajes llegan como **webhooks HTTPS entrantes**, lo que obliga a que exista una entrada pública
con un certificado válido, dirigida a un servidor que vive detrás de un router doméstico. Ese es el
problema que esta etapa resuelve, y tiene dos soluciones con consecuencias muy distintas.

Además del adaptador y de la entrada, esta etapa cubre la **migración de las dos células piloto** al
canal oficial. Es la prueba definitiva de la frontera de migración: si el historial conversacional de
piloto-01 sobrevive intacto al cambio de canal, la decisión de no persistir nunca identificadores de
transporte crudos en `sessions.db` habrá valido cada minuto invertido.

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
  | Dirección de la conexión | Saliente desde el servidor local, igual que en la Fase A. | Túnel WireGuard entre VPS y servidor local. |
  | Coste | Cero. | ~3 USD/mes fijos. |
  | Handshake anti-Hairpin (FR-04) | **Innecesario**: no hay certificado local que validar ni Hairpin NAT que sortear. | **Necesario**: se conserva íntegro. |
  | On-Demand TLS de Caddy (NFR-04) | **Innecesario**. | **Necesario**. |
  | Dependencia externa | Cloudflare, con los términos de su capa gratuita. | Proveedor de VPS. |
  | Arquitectura del PRD original | Se simplifica: desaparecen dos mecanismos. | Se conserva sin cambios. |

  La decisión condiciona directamente el alcance de la etapa B-2 y la vigencia de FR-04 y NFR-04.
* Aprovisionamiento de la aplicación de Meta, del WABA de prueba y de las credenciales necesarias.
* **Migración de las células piloto** al canal oficial: adquisición del número o migración del
  existente al canal oficial, conmutación del adaptador y verificación de que el historial
  conversacional y el conocimiento sobreviven intactos.
* Retirada del sidecar en las células migradas, con la recuperación del presupuesto de memoria de
  NFR-01 hasta los 50 MB por célula.

### Qué NO entra

* Caddy, subdominios, blackholing y el `cell create` completo: etapa B-2.
* Embedded Signup: etapa B-2.
* Cualquier cambio en el núcleo, la persistencia, el conocimiento o el control de admisión. Si esta
  etapa necesita tocarlos, el puerto de canal estaba mal diseñado y eso es en sí mismo un hallazgo.

### Requisitos del PRD cubiertos

* **FR-01** — implementación de la variante de Fase B: recepción y verificación de webhooks de la Meta
  Graph API.
* **FR-08** — variante de Fase B: Fast-Reject con `HTTP 200 OK` hacia Meta, sobre el mismo módulo GCRA
  ya construido.
* **FR-12** — segunda implementación del puerto, que valida el contrato.
* **FR-04** y **NFR-04** — su vigencia queda determinada por el ADR de entrada pública.

---

## Entregables

* Implementación `CloudApiAdapter` del trait `ChannelAdapter`.
* `hexcell-meta` completado: verificación de firma, tipos del payload de webhook y cliente de envío.
* ADR de **entrada pública de la Fase B** con la decisión tomada y sus consecuencias sobre FR-04 y
  NFR-04.
* ADR del adaptador de Cloud API.
* Entrada pública desplegada y verificada según la opción elegida.
* Informe de migración de las células piloto, incluida la comprobación de continuidad del historial.
* **Banco de pruebas local reutilizable (`scripts/`) capaz de emitir webhooks firmados como lo haría
  Meta**, con payloads realistas y firma HMAC válida. Es la herramienta sobre la que se apoyan las
  pruebas de firma, de deduplicación y de carga, y permite ejercitar el canal oficial sin depender de
  tráfico real de Meta.
* Pruebas: verificación de firma con vector conocido, rechazo ante alteración de un solo byte del
  cuerpo, deduplicación de reintentos de Meta y Fast-Reject bajo carga.

---

## Tareas

*(Sin estimación: la Fase B no se dimensiona hasta que se abra la compuerta.)*

1. **Decidir la entrada pública y escribir el ADR.** Es la primera tarea porque condiciona todo lo
   demás, incluida la mitad del alcance de la etapa B-2.
2. **Desplegar y verificar la entrada pública elegida.** Certificado válido, alcance desde fuera y
   estabilidad del túnel.
3. **Implementar el receptor de webhooks** con el desafío de verificación —comparando el token en
   tiempo constante, para no filtrar información por el tiempo de respuesta— y la respuesta `200 OK`
   inmediata anterior al procesamiento.
4. **Implementar la validación de firma HMAC-SHA256** sobre el cuerpo crudo, antes de cualquier
   deserialización.
5. **Traducir el payload de Meta al evento canónico** y mapear el `wa_id` al identificador interno
   dentro del adaptador.
6. **Implementar el envío y la traducción de acuses** a `sent`/`delivered`/`read`/`failed`.
7. **Reincorporar el Fast-Reject** sobre el módulo GCRA existente, sin modificarlo.
8. **Migrar piloto-01 al canal oficial** y verificar la continuidad del historial conversacional.
9. **Migrar piloto-02** una vez piloto-01 lleve tiempo estable sobre el canal oficial.
10. **Retirar el sidecar de las células migradas** y volver a medir el consumo de memoria contra el
    presupuesto de 50 MB.

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
* Tras migrar una célula piloto, su historial conversacional y su conocimiento siguen siendo
  accesibles y correctos, y las conversaciones anteriores se continúan sin ruptura.
* El consumo de memoria de una célula migrada, ya sin sidecar, es inferior a 50 MB en reposo.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| El puerto de canal resulta insuficiente y el adaptador obliga a tocar el núcleo. | Muy alto: la premisa de las dos fases falla y el coste de la Fase B se dispara. | Diseño del puerto con ambos canales delante desde la etapa A-1: la abstracción hacia el caso más restrictivo (semántica de la Cloud API) y sus tests de contrato son la garantía de compatibilidad, no una firma anticipada — `hexcell-meta` nace vacío hasta que se resuelva el ADR-0013. Si aun así ocurre, no se acepta la etapa: se detiene el trabajo, se analiza la desviación y se corrige el contrato mediante una revisión explícita del ADR-0010. |
| La opción de entrada pública se elige por coste sin evaluar sus consecuencias. | Alto: se arrastra o se descarta indebidamente FR-04 y NFR-04. | ADR obligatorio con la tabla de tradeoffs, decidido antes de cualquier despliegue. |
| Dependencia de la capa gratuita de Cloudflare y de sus términos. | Medio: un cambio de política obligaría a migrar la entrada. | Aislar la entrada tras una frontera clara, de modo que cambiar de opción no toque el adaptador. |
| Aprobación de la aplicación de Meta más lenta de lo previsto. | Medio: retrasa toda la Fase B. | Iniciar el trámite en cuanto se abra la compuerta, en paralelo al trabajo técnico. |
| La migración de una célula piloto rompe su historial. | Alto: pérdida de contexto conversacional de clientes reales. | Respaldo previo obligatorio con restauración verificada, y comprobación de continuidad como criterio de aceptación. |
| Reserializar el cuerpo del webhook antes de validar la firma. | Alto: la firma falla de forma intermitente e inexplicable. | Validar siempre sobre los bytes crudos recibidos, antes de cualquier deserialización, con prueba explícita. |
| Procesar el mensaje antes de responder a Meta. | Alto: se superan los tiempos de espera y se dispara la tormenta de reintentos. | Responder `200 OK` y encolar; la prueba de integración mide el tiempo hasta la respuesta. |

---

## Dependencias

* **De otras etapas:** **la compuerta de la etapa A-7 debe estar abierta.** Esta etapa no se inicia
  antes. Requiere además todas las etapas de la Fase A completas.
* **Externas:** una aplicación de Meta aprobada con acceso a la Cloud API, un WABA, y —según la
  opción elegida— una cuenta de Cloudflare o un VPS contratado.
* **Decisiones de producto pendientes:** el **modelo de monetización**, porque a partir de esta fase
  hay clientes de pago.
