# Fase B · Etapa 2 — Plano de control y onboarding comercial

**Duración relativa:** sin estimar. **La Fase B está CONGELADA hasta la compuerta del tercer
cliente**; sus etapas se describen en alcance y dependencias, no en días de trabajo.

---

## Objetivo

Esta etapa fusiona lo que en el plan anterior eran dos etapas separadas —el plano de control y el
onboarding— porque con el canal oficial ya resuelto en la etapa B-1 ambas responden a la misma
pregunta: **cómo se da de alta, se gobierna y se da de baja a un cliente de pago sin que Meta note
nada y sin fricción técnica para el dueño del negocio**.

El requisito que da forma a la mitad del trabajo es NFR-02: **cero errores HTTP 502 o 503 expuestos
hacia la WAN de Meta durante suspensiones o reactivaciones**. Es una exigencia dura, porque la forma
natural de apagar un backend es apagarlo, y entonces el proxy inverso responde 502. La respuesta del
PRD es invertir el orden de las operaciones: primero se sustituye el proxy inverso por una respuesta
estática de `HTTP 200 OK` en Caddy (*blackholing*), y solo después se envía el `SIGTERM` al
contenedor. Mientras el contenedor se apaga, Caddy sigue absorbiendo los webhooks y confirmándolos a
Meta. Al reactivar se hace lo simétrico.

Nótese que este problema **no existía en la Fase A**: sin petición entrante que contestar, la
desconexión del websocket bastaba. La complejidad del blackholing es el precio del canal oficial, y
se paga solo cuando hay clientes que lo justifican.

La otra mitad es el alta. El obstáculo técnico central es sutil y merece explicarse despacio. El
servidor vive en una red local, detrás de un router doméstico. Muchos routers domésticos carecen de
*Hairpin NAT*, la capacidad de que un equipo de la red interna alcance su propia dirección pública.
Eso significa que el servidor no puede comprobar por sí mismo, usando su dominio público, que su
certificado y su enrutamiento funcionan desde fuera. Y hacer esa comprobación es imprescindible: si
registramos la URL en Meta antes de que la autoridad certificadora haya emitido el certificado, Meta
recibe un fallo TLS y la suscripción se rechaza, dejando al cliente a medio dar de alta.

FR-04 resuelve el problema forzando la resolución del socket del cliente HTTP hacia la interfaz de
loopback mientras se envían el SNI y el encabezado `Host` del dominio público. Con ese truco, Caddy
recibe una petición que cree externa, se ve obligado a completar el desafío ACME con la autoridad
certificadora, y esa autoridad **sí** valida el entorno WAN desde fuera.

> **Alcance condicionado por el ADR de entrada pública (etapa B-1).** Todo lo relativo al handshake
> anti-Hairpin (FR-04) y al On-Demand TLS de Caddy (NFR-04) **solo aplica si la entrada pública
> elegida termina el TLS en el propio servidor** (opción VPS + WireGuard). Si se elige Cloudflare
> Tunnel, el TLS termina en el edge y ambos mecanismos desaparecen del alcance, sustituidos por la
> configuración de rutas del túnel. La etapa no puede planificarse en detalle antes de esa decisión.

---

## Alcance

### Qué entra

#### Plano de control

* Integración con la API de administración de Caddy: alta, modificación y baja de rutas por
  subdominio de forma programática, sin recargar la configuración global ni interrumpir a terceros.
* Configuración de TLS automático en Caddy, incluida la emisión bajo demanda y su restricción a los
  dominios legítimamente registrados. *(Solo si el TLS termina en el servidor.)*
* Ampliación de los comandos de la etapa A-6 con la dimensión de Caddy:
  * `cell pause` — blackholing en Caddy y después `SIGTERM` al contenedor con 30 segundos de gracia.
  * `cell unpause` — arranque, sondeo de `GET /health/ready` cada 100 ms y conmutación de la respuesta
    estática al proxy inverso solo tras la primera confirmación positiva.
  * `cell terminate` — desasociación del webhook en la Meta Graph API, además del drenaje y la
    destrucción de volúmenes que ya hacía la Fase A, y purga de la ruta y de la caché de certificados
    en Caddy.

> **Nota de fuente.** El PRD cubre explícitamente la suspensión y la reactivación (FR-11 y las
> matrices de ciclo de vida de la sección 5), pero **no la eliminación definitiva**. El comando
> `cell terminate` y su secuencia provienen del [README.md del proyecto](../../README.md),
> "Manual de Operación de la CLI de Administración", apartado 3. No es un requisito inventado por
> este plan, pero su rango es inferior al de los FR: ante conflicto, manda el PRD.

#### Onboarding

* Comando **`cell create` completo** en `hexcell-admin`, que ejecuta la secuencia de alta de
  principio a fin con reversión automática ante fallo.
* Generación del identificador de la célula, su subdominio, su token de verificación criptográfico
  y sus secretos, con almacenamiento seguro.
* Aprovisionamiento: creación del volumen, alta de la ruta en Caddy y arranque del contenedor con la
  configuración de la célula.
* **Handshake sintético de red** conforme a FR-04: petición `GET /webhook` con resolución forzada del
  socket a `127.0.0.1:443`, SNI y encabezado `Host` del dominio público, y comprobación de que el
  `hub.challenge` vuelve intacto y de que el certificado es válido y de confianza. *(Solo si el TLS
  termina en el servidor.)*
* Reintento con espera progresiva mientras la autoridad certificadora completa la emisión, con
  límite temporal y diagnóstico claro si no se consigue.
* Registro del webhook en la Meta Graph API usando `override_callback_uri` para dirigir el tráfico
  del WABA al subdominio de la célula.
* Soporte del flujo **Meta Embedded Signup** bajo la aplicación única del proveedor: recepción del
  código de autorización, intercambio por credenciales del cliente y asociación con la célula.

> **Nota de fuente.** El flujo *Meta Embedded Signup* y el uso de `override_callback_uri` para
> dirigir el tráfico del WABA al subdominio de la célula **no aparecen en el PRD**: provienen del
> [README.md del proyecto](../../README.md), sección "Flujo de Onboarding e Inyección de Red
> (Anti-Hairpin NAT)". No son requisitos inventados por este plan, pero tampoco tienen rango
> normativo: el PRD es la fuente normativa y solo fija FR-04 (handshake sintético). Si producto
> decide otro mecanismo de alta, esta parte del alcance cambia sin afectar a FR-04.

* Verificación de extremo a extremo del alta: envío de un mensaje real de prueba y comprobación de
  que llega, se procesa y se responde.
* Reversión automática: si cualquier paso falla, deshacer los anteriores para no dejar células a
  medio crear.
* Carga del conocimiento inicial del cliente mediante el pipeline de la etapa A-5.

### Qué NO entra

* El diseño comercial del proceso de alta: qué datos se piden al cliente, quién los recoge, qué
  contrato se firma y en qué orden. Es una decisión de producto pendiente.
* La interfaz de usuario del Embedded Signup del lado del cliente final.
* La facturación del alta.
* Cualquier interfaz gráfica de administración.
* La lógica interna del contenedor, terminada en las etapas A-2 a A-5.

### Requisitos del PRD cubiertos

* **FR-03** — gestión de configuración dinámica de Caddy por subdominio sin interrumpir a terceros.
* **FR-04** — handshake sintético de red, condicionado al ADR de entrada pública.
* **FR-11** — variante de Fase B: blackholing previo al `SIGTERM`.
* **NFR-02** — cero errores 502/503 hacia Meta durante suspensiones y reactivaciones.
* **NFR-04** — cifrado HTTPS con TLS 1.2/1.3, condicionado al ADR de entrada pública.
* Cierre operativo de **FR-01** y **FR-03** sobre clientes comerciales reales.

---

## Entregables

* `hexcell-admin` con `cell create` completo y con los comandos de ciclo de vida ampliados a Caddy.
* Módulo cliente de la API de administración de Caddy.
* Configuración base de Caddy versionada en el repositorio, con la política de TLS.
* Módulo de handshake sintético reutilizable, capaz de forzar la resolución del socket, el SNI y el
  encabezado `Host`. *(Condicionado al ADR.)*
* Ampliación de `hexcell-meta` con el registro de webhook, `override_callback_uri` y el intercambio
  de credenciales del Embedded Signup.
* Almacén de secretos por célula con su política de acceso.
* ADR del plano de control con el orden de operaciones de cada secuencia y su justificación.
* ADR del handshake sintético, si aplica.
* `docs/runbook-operacion.md` ampliado y `docs/runbook-onboarding.md`.
* Prueba de integración que mide códigos HTTP durante un ciclo completo de pausa y reactivación.
* Prueba de resiliencia con el Hairpin NAT bloqueado artificialmente, si aplica.

---

## Tareas

*(Sin estimación: la Fase B no se dimensiona hasta que se abra la compuerta. El desglose depende
además de la opción de entrada pública elegida en la etapa B-1.)*

1. **Implementar el cliente de la API de administración de Caddy**, con operaciones de grano fino
   sobre la ruta concreta.
2. **Establecer la configuración base de Caddy y su política TLS**, con la emisión bajo demanda
   restringida a los dominios registrados en el plano de control.
3. **Ampliar `cell pause` con el blackholing**, verificando que la respuesta estática está activa
   antes de emitir el `SIGTERM`.
4. **Ampliar `cell unpause` con la conmutación final** de la respuesta estática al proxy inverso.
5. **Ampliar `cell terminate` con la desuscripción en Meta** y la purga de ruta y certificados,
   con reintentos acotados y estado pendiente reejecutable ante límites de tasa de la API Graph.
6. **Ampliar `cell list` y `cell status` con la dimensión de Caddy.** El estado consolidado pasa a
   cruzar tres fuentes en lugar de dos —plano de control, Docker y Caddy—, señalando las
   discrepancias: una célula activa cuya ruta esté en blackholing, o una ruta viva apuntando a un
   contenedor detenido, deben aparecer como tales y no como estado normal.
7. **Diseñar la secuencia de alta y sus puntos de reversión**, documentándola en el ADR antes de
   escribir código.
8. **Implementar la generación de identidad y secretos de la célula**: identificador, subdominio,
   token de verificación criptográficamente aleatorio y secreto de firma.
9. **Implementar el aprovisionamiento de infraestructura**, reutilizando los módulos de la etapa A-6.
10. **Implementar el handshake sintético** con validación de la cadena de certificados y comprobación
    del `hub.challenge` devuelto. *(Solo si el TLS termina en el servidor.)*
11. **Añadir espera progresiva y diagnóstico**, distinguiendo entre fallo de DNS, fallo de emisión y
    fallo de la aplicación.
12. **Implementar el registro del webhook en Meta** con `override_callback_uri`.
13. **Integrar el flujo Meta Embedded Signup** con el intercambio por credenciales duraderas.
14. **Implementar la reversión automática** ante fallo en cualquier paso.
15. **Implementar la carga de conocimiento inicial** como parte del alta.
16. **Verificación de extremo a extremo** con un mensaje real de WhatsApp.
17. **Construir la prueba de ciclo de vida con tráfico continuo** y la prueba de resiliencia con
    Hairpin NAT bloqueado.
18. **Redactar los runbooks** de operación y de onboarding.

---

## Criterios de aceptación

* Durante un ciclo completo de `cell pause` seguido de `cell unpause`, con tráfico continuo contra el
  subdominio, **el 100 % de las respuestas son `HTTP 200 OK`**: ni un solo 502 ni 503 (NFR-02).
* `cell pause` deja el contenedor detenido con código de salida 0 y la ruta de Caddy devolviendo una
  respuesta estática `200 OK` con cuerpo `{}`.
* `cell unpause` no conmuta el tráfico al proxy inverso hasta que `GET /health/ready` ha respondido
  `200 OK` al menos una vez; forzar un backend que nunca esté listo produce un fallo explícito y el
  tráfico permanece absorbido por la respuesta estática.
* Alta y baja de una ruta en Caddy para una célula no interrumpen ni alteran el tráfico de las demás
  células activas, verificado con tráfico concurrente (FR-03).
* Todos los subdominios sirven exclusivamente sobre TLS 1.2 o 1.3, y una conexión con protocolos
  anteriores es rechazada (NFR-04).
* **Ligado al criterio de QA "Prueba de Resiliencia del Enlace TLS" del PRD:** con el Hairpin NAT del
  router bloqueado artificialmente, `cell create` completa el alta con éxito gracias a la resolución
  forzada del socket. *(Solo si el TLS termina en el servidor; en caso contrario, se sustituye por la
  verificación del túnel.)*
* Si el handshake falla, **no** se registra nada en la Meta Graph API y el sistema queda sin residuos
  del alta abortada.
* Tras un alta exitosa, un mensaje real enviado al número del cliente llega a la célula correcta, se
  procesa y recibe respuesta.
* El tráfico de un WABA llega exclusivamente al subdominio de su célula, verificado con al menos dos
  células dadas de alta simultáneamente.
* Un fallo inyectado en cualquier paso de la secuencia deja el sistema exactamente como estaba antes
  de iniciar el alta.
* Cada célula tiene su propio token de verificación y su propio secreto de firma; ninguno es
  reutilizado entre clientes.
* `cell terminate` deja el sistema sin rastro de la célula: sin contenedor, sin volúmenes en disco,
  sin ruta en Caddy y sin suscripción de webhook en Meta.
* Interrumpir cualquier comando a mitad y reejecutarlo lleva el sistema al estado pretendido sin
  intervención manual.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Enviar el `SIGTERM` antes de aplicar el blackholing. | Muy alto: se generan 502 hacia Meta, incumpliendo NFR-02 y activando reintentos. | El orden está fijado en el ADR y verificado por la prueba de ciclo de vida con tráfico continuo. |
| Una modificación de la configuración de Caddy afecta a rutas de otras células. | Muy alto: caída de clientes ajenos a la operación. | Usar operaciones de grano fino sobre la ruta concreta y probar siempre con varias células activas. |
| La API de administración de Caddy queda expuesta más allá del host local. | Muy alto: control total del enrutamiento para un atacante. | Vincularla exclusivamente a la interfaz de loopback y documentarlo en el runbook. |
| El certificado no está emitido cuando se registra en Meta. | Alto: Meta rechaza la suscripción y el cliente queda a medio dar de alta. | El handshake sintético es bloqueante: sin certificado válido no se llama a Meta. |
| El DNS comodín o el registro del subdominio no está propagado. | Medio: la emisión ACME falla por razones ajenas al código. | Comprobación previa de DNS con diagnóstico específico, y requisito de DNS comodín documentado en el runbook. |
| Cambios en el flujo Embedded Signup o en las políticas de la aplicación de Meta. | Alto: el alta deja de funcionar sin aviso. | Aislar la integración detrás de una interfaz propia, cubrirla con pruebas de contrato y vigilar los avisos de cambio de la plataforma. |
| Fallo parcial de `cell terminate` que deja datos en disco o una suscripción viva en Meta. | Alto: fuga de datos o tráfico entrante hacia una célula inexistente. | Orden de operaciones que desconecta primero y destruye después, con idempotencia y verificación final de cada paso. |
| Límites de tasa de la API Graph al desuscribir. | Medio: la baja de una célula falla por saturación de la API y no por un error propio. | Reintentos acotados y registro del estado pendiente para reejecución posterior, de modo que la desuscripción quede encolada y no se pierda. |
| Fuga de secretos de células por almacenamiento inadecuado. | Muy alto. | Almacén con permisos restringidos, secretos nunca escritos en logs y rotación documentada. |
| Se planifica esta etapa en detalle antes de decidir la entrada pública. | Medio: la mitad del trabajo planificado podría no existir. | El ADR de entrada pública es la primera tarea de la etapa B-1, anterior a cualquier detalle de esta. |
| **Proceso exacto de onboarding sin definir** (pendiente en STATUS.md). | Alto: la secuencia técnica puede no encajar con el proceso comercial real. | Los pilotos de la etapa A-7 aportan experiencia real de alta antes de llegar aquí. La captura de datos y el orden comercial siguen bloqueados hasta que producto los defina. |

---

## Dependencias

* **De otras etapas:** etapa B-1 completa, y muy en particular **el ADR de entrada pública**, que
  determina la mitad del alcance de esta etapa.
* **Externas:** dominio propio con DNS comodín bajo control, una aplicación de Meta aprobada con los
  permisos del Embedded Signup, y credenciales de la Meta Graph API con permiso para suscribir y
  desuscribir webhooks.
* **Decisiones de producto pendientes (bloqueantes):** el **proceso exacto de alta de una
  microempresa** y los **flujos de usuario finales** de STATUS.md. El **modelo de monetización**
  define además cuándo se suspende a un cliente por falta de pago: el mecanismo se entrega aquí; la
  política que lo activa, no.
