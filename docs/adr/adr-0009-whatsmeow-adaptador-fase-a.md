# ADR-0009 — whatsmeow como adaptador no oficial de la Fase A

* **Estado:** Vigente desde el 2026-07-29.
* **Supersede a:** nada. Formaliza una decisión ya tomada y registrada hasta ahora solo en
  `docs/adr/README.md` y `docs/STATUS.md`.
* **Etapa:** A-1.
* **Requisitos tocados:** FR-01, FR-12, NFR-01.

---

## Contexto

El canal propio de la Fase A necesita una biblioteca que hable el protocolo de WhatsApp multidevice
sin pasar por la Cloud API oficial de Meta. El hardware objetivo es modesto (i7 de 10 años, 8 GB RAM)
y el presupuesto de memoria por célula sobre canal propio es de **≤ 80 MB**, repartido entre el
núcleo Rust y el sidecar del canal (ver `docs/STATUS.md`). Desde `adr-0014`, este canal deja de ser
un adaptador temporal de validación y pasa a ser el canal de producción **permanente**, con clientes
de pago reales, lo que exige una biblioteca madura y no un experimento.

## Alternativas contrastadas

**A. Baileys (Node.js/TypeScript).** Es la biblioteca más popular en el ecosistema no oficial y con
mayor volumen de adopción comunitaria. Se descarta por dos motivos ligados directamente a los
criterios del proyecto: (1) requiere el runtime de Node.js además del núcleo Rust, lo que suma un
tercer proceso y su propia huella de memoria a un presupuesto ya ajustado a 80 MB por célula, contra
un binario Go que compila estático y arranca liviano; y (2) su historial de estabilidad frente a
cambios de protocolo es más irregular — el propio ecosistema documenta rupturas recurrentes que tardan
en resolverse cuando WhatsApp cambia su versión mínima de cliente (ver, por ejemplo, el seguimiento en
[whatsapp-web.js#2988](https://github.com/pedroslopez/whatsapp-web.js/issues/2988) para la variante
basada en Puppeteer/whatsapp-web.js, que además exige un navegador Chromium embebido y multiplica la
huella de memoria muy por encima del presupuesto).

**B. whatsapp-web.js (Node.js sobre Puppeteer/Chromium).** Emula un navegador completo para hablar con
WhatsApp Web, lo que implica cargar Chromium por instancia. Se descarta de inmediato: un Chromium
embebido por célula excede por sí solo el presupuesto de memoria de la célula entera, antes de sumar
el núcleo Rust.

**C. Meta Cloud API directa, incluso en la Fase A.** Elimina el riesgo de baneo por Términos de
Servicio y la necesidad de un sidecar. Se descarta para la Fase A porque exige verificación de
negocio (WABA) y plantillas aprobadas antes de que exista ningún cliente, contradice el objetivo de
onboarding rápido de microempresas sin trámite previo, y es exactamente la limitación estructural que
`adr-0010` (puerto de canal) existe para acotar detrás de una frontera, no para adoptar de entrada. Es,
además, la decisión que corresponde a la Fase B, no a la A.

**D. whatsmeow (Go) — elegida.** Biblioteca no oficial en Go, con soporte multidevice maduro, que
compila a un binario nativo ligero adecuado al presupuesto de ≤ 80 MB por célula sin runtime
adicional. Su base de código es activa —con actividad casi diaria documentada en junio y julio de
2026 (ver `adr-0014`)— y su historial de rupturas de protocolo, aunque recurrente
(`Client outdated (405)`), se resuelve en días mediante un simple *bump* de versión, como documenta
el precedente de abril de 2026 en
[lharries/whatsapp-mcp#216](https://github.com/lharries/whatsapp-mcp/issues/216).

## Decisión

Se adopta **whatsmeow** como biblioteca del sidecar del canal propio de la Fase A, por tres criterios
del proyecto, en orden de peso:

1. **Proceso Go nativo y liviano.** Un binario Go compilado estático encaja en el presupuesto de
   ≤ 80 MB por célula sin sumar un runtime de lenguaje interpretado ni un navegador embebido, a
   diferencia de Baileys (Node.js) o whatsapp-web.js (Node.js + Chromium).
2. **Multidevice maduro.** whatsmeow implementa el protocolo multidevice de WhatsApp de forma nativa,
   sin depender de una sesión de navegador que emular.
3. **Madurez y velocidad de reparación ante roturas de protocolo.** Las rupturas por
   `Client outdated (405)` son recurrentes en todo el ecosistema no oficial, pero el historial de
   whatsmeow muestra reparación rápida (días, no semanas) mediante actualización de versión.

**Esta elección no reduce el riesgo estructural de baneo documentado en `adr-0015`.** Ninguna
biblioteca no oficial lo evita: Meta detecta la huella de protocolo del cliente, no la implementación
concreta. whatsmeow se elige por su relación coste de memoria / madurez / velocidad de reparación,
no porque prometa inmunidad frente a los mecanismos antiabuso de Meta.

## Consecuencias

### Positivas

* Encaja en el presupuesto de memoria del hardware objetivo sin sumar un runtime adicional.
* Multidevice nativo sin necesidad de mantener una sesión de navegador.
* Las rupturas de protocolo documentadas se resuelven en días, no en semanas.

### Negativas

* **whatsmeow tiene bus factor 1** (ver `adr-0014`): prácticamente todos sus commits provienen de un
  único mantenedor. No se puede comprometer ningún tiempo de recuperación que dependa de un
  mantenedor voluntario ante una rotura mayor de protocolo.
* Es una biblioteca no oficial: el riesgo de baneo por parte de Meta es estructural y permanente, no
  un defecto que esta elección corrija. La política frente a ese riesgo se desarrolla en `adr-0015`,
  no aquí.
* Requiere un sidecar Go separado del núcleo Rust, comunicado por IPC (`adr-0011`), lo que añade un
  segundo proceso por célula frente a una hipotética integración en un único binario.

## Referencias

* `adr-0010-puerto-de-canal.md`: frontera `ChannelAdapter` que aísla al núcleo de esta elección de
  biblioteca.
* `adr-0011-whatsmeow-sidecar-e-ipc.md`: arquitectura de sidecar e IPC que implementa esta decisión.
* `adr-0014-canal-propio-permanente.md`: convierte este canal en producción permanente y documenta el
  riesgo de mantenimiento (bus factor 1) con más detalle.
* `adr-0015-politica-de-convivencia-con-el-baneo.md`: política frente al riesgo estructural de baneo
  que esta elección de biblioteca no evita.
* `docs/adr/README.md`: fila de este ADR.
