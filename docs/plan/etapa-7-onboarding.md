# Etapa 7 — Onboarding de inquilinos y handshake de red

**Duración relativa:** Media.

---

## Objetivo

Todas las piezas existen: el contenedor procesa mensajes, se protege de las ráfagas, controla su
gasto, actualiza su conocimiento sin detenerse, está aislado y se puede gobernar desde la CLI. Falta
lo que convierte todo eso en un negocio: **dar de alta a una microempresa real, de principio a fin,
sin fricción técnica para el dueño del negocio**.

El obstáculo técnico central es sutil y merece explicarse despacio. El servidor vive en una red
local, detrás de un router doméstico. Muchos routers domésticos carecen de *Hairpin NAT*, la
capacidad de que un equipo de la red interna alcance su propia dirección pública. Eso significa que
el servidor no puede comprobar por sí mismo, usando su dominio público, que su certificado y su
enrutamiento funcionan desde fuera. Y hacer esa comprobación es imprescindible: si registramos la URL
en Meta antes de que Let's Encrypt haya emitido el certificado, Meta recibe un fallo TLS y la
suscripción se rechaza, dejando al cliente a medio dar de alta.

FR-04 resuelve el problema forzando la resolución del socket del cliente HTTP hacia la interfaz de
loopback mientras se envían el SNI y el encabezado `Host` del dominio público. Con ese truco, Caddy
recibe una petición que cree externa, se ve obligado a completar el desafío ACME con la autoridad
certificadora, y esa autoridad **sí** valida el entorno WAN desde fuera. Cuando el handshake
sintético devuelve el `hub.challenge` correcto, tenemos la garantía de que el camino público existe.
Solo entonces se registra en Meta.

---

## Alcance

### Qué entra

* Comando `tenant create` en `zeroclaw-admin`, que ejecuta la secuencia completa de alta.
* Generación del identificador del inquilino, su subdominio, su token de verificación criptográfico
  y sus secretos, con almacenamiento seguro.
* Aprovisionamiento: creación del volumen, alta de la ruta en Caddy y arranque del contenedor con la
  configuración del inquilino.
* Handshake sintético de red conforme a FR-04: petición `GET /webhook` con resolución forzada del
  socket a `127.0.0.1:443`, SNI y encabezado `Host` del dominio público, y comprobación de que el
  `hub.challenge` vuelve intacto y de que el certificado es válido y de confianza.
* Reintento con espera progresiva mientras la autoridad certificadora completa la emisión, con
  límite temporal y diagnóstico claro si no se consigue.
* Registro del webhook en la Meta Graph API usando `override_callback_uri` para dirigir el tráfico
  del WABA al subdominio del inquilino.
* Soporte del flujo **Meta Embedded Signup** bajo la aplicación única del proveedor: recepción del
  código de autorización, intercambio por credenciales del cliente y asociación con el inquilino.

> **Nota de fuente.** El flujo *Meta Embedded Signup* y el uso de `override_callback_uri` para
> dirigir el tráfico del WABA al subdominio del inquilino **no aparecen en el PRD**: provienen del
> [README.md del proyecto](../../README.md), sección "Flujo de Onboarding e Inyección de Red
> (Anti-Hairpin NAT)". No son requisitos inventados por este plan, pero tampoco tienen rango
> normativo: el PRD es la fuente normativa y solo fija FR-04 (handshake sintético). Si producto
> decide otro mecanismo de alta, esta parte del alcance cambia sin afectar a FR-04.
* Verificación de extremo a extremo del alta: envío de un mensaje real de prueba y comprobación de
  que llega, se procesa y se responde.
* Reversión automática: si cualquier paso falla, deshacer los anteriores para no dejar inquilinos a
  medio crear.
* Carga del conocimiento inicial del cliente mediante el pipeline de la etapa 4.

### Qué NO entra

* El diseño comercial del proceso de alta: qué datos se piden al cliente, quién los recoge, qué
  contrato se firma y en qué orden. Es una decisión de producto pendiente.
* La interfaz de usuario del Embedded Signup del lado del cliente final.
* La facturación del alta.

### Requisitos del PRD cubiertos

* **FR-04** — handshake sintético de red con inyección de SNI y resolución forzada a la interfaz
  local.
* Cierre operativo de **FR-01** (la suscripción del webhook queda efectivamente registrada en Meta),
  de **FR-03** (alta real de subdominio) y de **NFR-04** (certificado emitido y validado
  externamente).

---

## Entregables

* Comando `tenant create` completo, con reversión automática ante fallo.
* Módulo de handshake sintético reutilizable, capaz de forzar la resolución del socket, el SNI y el
  encabezado `Host`.
* Ampliación de `zeroclaw-meta` con el registro de webhook, `override_callback_uri` y el intercambio
  de credenciales del Embedded Signup.
* Almacén de secretos por inquilino con su política de acceso.
* `docs/adr/adr-0009-handshake-sintetico.md` explicando el mecanismo y por qué es necesario.
* `docs/runbook-onboarding.md`: guía operativa del alta paso a paso, incluida la resolución de los
  fallos más probables.
* Prueba de resiliencia con el Hairpin NAT bloqueado artificialmente.

---

## Tareas

1. **Diseñar la secuencia de alta y sus puntos de reversión** (1 día). Enumerar cada paso, qué deja
   creado y cómo se deshace; documentarlo en el ADR antes de escribir código.
2. **Implementar la generación de identidad y secretos del inquilino** (1 día). Identificador,
   subdominio, token de verificación criptográficamente aleatorio y secreto de firma, con
   almacenamiento cifrado o con permisos restringidos.
3. **Implementar el aprovisionamiento de infraestructura** (1 día). Creación del volumen, alta de la
   ruta en Caddy y arranque del contenedor, reutilizando los módulos de la etapa 6.
4. **Implementar el handshake sintético** (1,5 días). Cliente HTTP con resolución de socket forzada a
   `127.0.0.1:443`, SNI y encabezado `Host` del dominio público, validación de la cadena de
   certificados y comprobación del `hub.challenge` devuelto.
5. **Añadir espera progresiva y diagnóstico** (0,5 días). Reintentos mientras la autoridad
   certificadora emite, con límite temporal y mensajes que distingan entre fallo de DNS, fallo de
   emisión y fallo de la aplicación.
6. **Implementar el registro del webhook en Meta** (1,5 días). Suscripción del WABA con
   `override_callback_uri` apuntando al subdominio del inquilino, y verificación de que Meta acepta.
7. **Integrar el flujo Meta Embedded Signup** (2 días). Recepción del código de autorización,
   intercambio por credenciales duraderas, asociación al inquilino y almacenamiento seguro.
8. **Implementar la reversión automática** (1 día). Ante fallo en cualquier paso, deshacer los
   anteriores en orden inverso y dejar constancia de lo ocurrido.
9. **Implementar la carga de conocimiento inicial** (0,5 días). Invocación del pipeline de la etapa 4
   con el catálogo del cliente como parte del alta.
10. **Verificación de extremo a extremo** (1 día). Envío de un mensaje real de WhatsApp al número del
    cliente y comprobación de que se recibe, se procesa y se responde.
11. **Escribir la prueba de resiliencia con Hairpin NAT bloqueado** (1 día). Simulación de la
    ausencia de Hairpin NAT y ejecución completa del alta.
12. **Redactar el runbook de onboarding** (0,5 días). Secuencia operativa, requisitos previos y
    resolución de los fallos frecuentes.

---

## Criterios de aceptación

* **Ligado al criterio de QA "Prueba de Resiliencia del Enlace TLS" del PRD:** con el Hairpin NAT del
  router bloqueado artificialmente, `tenant create` completa el alta con éxito gracias a la
  resolución forzada del socket a nivel de red.
* El handshake sintético devuelve el `hub.challenge` intacto y valida una cadena de certificados de
  confianza antes de que se ejecute cualquier llamada de registro a Meta.
* Si el handshake falla, **no** se registra nada en la Meta Graph API y el sistema queda sin residuos
  del alta abortada.
* Tras un alta exitosa, un mensaje real enviado al número del cliente llega al contenedor correcto,
  se procesa y recibe respuesta.
* El tráfico de un WABA llega exclusivamente al subdominio de su inquilino, verificado con al menos
  dos inquilinos dados de alta simultáneamente.
* Un fallo inyectado en cualquier paso de la secuencia deja el sistema exactamente como estaba antes
  de iniciar el alta.
* Cada inquilino tiene su propio token de verificación y su propio secreto de firma; ninguno es
  reutilizado entre clientes.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| El certificado no está emitido cuando se registra en Meta. | Alto: Meta rechaza la suscripción y el cliente queda a medio dar de alta. | El handshake sintético es bloqueante: sin certificado válido no se llama a Meta. |
| El DNS comodín o el registro del subdominio no está propagado. | Medio: la emisión ACME falla por razones ajenas al código. | Comprobación previa de DNS con diagnóstico específico, y requisito de DNS comodín documentado en el runbook. |
| Cambios en el flujo Embedded Signup o en las políticas de la aplicación de Meta. | Alto: el alta deja de funcionar sin aviso. | Aislar la integración detrás de una interfaz propia, cubrirla con pruebas de contrato y vigilar los avisos de cambio de la plataforma. |
| Fuga de secretos de inquilinos por almacenamiento inadecuado. | Muy alto. | Almacén con permisos restringidos, secretos nunca escritos en logs y rotación documentada. |
| **Proceso exacto de onboarding sin definir** (pendiente en STATUS.md). | Alto: la secuencia técnica puede no encajar con el proceso comercial real. | Esta etapa entrega la secuencia técnica completa y parametrizable. La captura de datos, el orden comercial y los flujos de usuario quedan explícitamente bloqueados hasta que producto los defina. |

---

## Dependencias

* **De otras etapas:** etapa 6 completa. Sin capacidad de crear rutas, arrancar contenedores y, sobre
  todo, revertir mediante `tenant terminate`, el alta no debe intentarse.
* **Externas:** dominio propio con DNS comodín bajo control, una aplicación de Meta aprobada con los
  permisos del Embedded Signup, y al menos una microempresa dispuesta a servir de piloto.
* **Decisiones de producto pendientes (bloqueantes):** el **proceso exacto de alta de una
  microempresa** y los **flujos de usuario finales** de STATUS.md. Sin ellos puede construirse y
  probarse la secuencia técnica, pero no ponerse en marcha el alta comercial real.
