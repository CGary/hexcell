# ADR-0008 — Estrategia de canal en dos fases con compuerta en el tercer cliente

* **Estado:** **Derogada** — *superseded* por `adr-0014-canal-propio-permanente.md` (2026-07-28).
* **Supersede a:** nada. Fue la primera decisión de estrategia de canal del proyecto.
* **Etapa:** A-1.
* **Requisitos tocados (en su momento):** FR-01, FR-12, NFR-01.

---

> **Este ADR es un registro histórico.** Se conserva sin reescribir para dejar rastro de qué se
> decidió, cuándo y por qué, y de qué condiciones cambiaron para derogarlo. La decisión vigente sobre
> estrategia de canal es `adr-0014-canal-propio-permanente.md`, que **supersede** a este documento; la
> política de convivencia con el riesgo de baneo que hace operable esa decisión vive en
> `adr-0015-politica-de-convivencia-con-el-baneo.md`. Ningún párrafo de este archivo describe el
> estado actual del proyecto.

## Contexto (histórico)

En el arranque de la etapa A-1 se decidió una estrategia de canal en dos fases secuenciales, no
convivientes. La **Fase A** validaba el negocio sobre el canal no oficial (whatsmeow) con exactamente
dos células piloto, bajo la regla explícita de que **no se comercializa sobre canal no oficial**: el
riesgo frente a los Términos de Servicio de WhatsApp se aceptaba solo como riesgo temporal de
validación, nunca como postura comercial estable. La **Fase B**, sobre la Meta Cloud API, era la fase
comercial: la que sí podía vender.

La compuerta que articulaba el paso de una fase a otra era numérica y cerraba la anterior: **el
tercer cliente no se sumaba a la Fase A, la cerraba**, forzando la migración a la Fase B antes de
aceptar más altas.

## Decisión (histórica, derogada)

1. La Fase A opera exclusivamente sobre canal no oficial (whatsmeow), limitada a dos células piloto,
   sin comercialización real.
2. El tercer cliente dispara el cierre de la Fase A y la apertura de la Fase B sobre Meta Cloud API,
   que pasa a ser el único canal de producción comercial.
3. La regla "no se comercializa sobre canal no oficial" se mantiene como condición permanente mientras
   dure la Fase A.

## Por qué quedó derogada

El 2026-07-28, `adr-0014-canal-propio-permanente.md` invalidó la premisa económica que sostenía la
compuerta: el coste de gestión comercial por cliente para migrar cada microempresa a una WABA
verificada recae íntegramente sobre el tiempo del fundador, el recurso más escaso del proyecto, y el
2026-07-01 Meta anunció que desde el 2026-10-01 cobrará también los mensajes de servicio de la Cloud
API, con lo que el transporte de la Fase B deja de ser gratuito. Con esas dos premisas caídas,
`adr-0014` deroga tanto la compuerta del tercer cliente como la regla "no se comercializa sobre canal
no oficial", y convierte el canal propio en canal de producción **permanente**, con clientes de pago
reales, mientras el canal oficial queda pospuesto a una segunda etapa que **convive** con el propio y
se activa por demanda de un cliente que la justifique — no por número de clientes ni por fecha.

**Este documento no debe leerse, ni citarse, como si la Fase B sustituyera, reemplazara o cerrara la
Fase A, ni como si el sidecar de whatsmeow fuera transitorio.** Esa lectura era la vigente cuando se
escribió este ADR y dejó de serlo el 2026-07-28. La compuerta del tercer cliente, derogada aquí, no se
reintroduce como política activa bajo ningún nombre nuevo; lo que disciplina el crecimiento de la
cartera hoy son las compuertas de riesgo (techo duro de cartera y umbral de incidentes que congela
altas) descritas en `adr-0014` y `adr-0015`, cuyos valores numéricos siguen pendientes como decisión
de negocio en `docs/STATUS.md`.

## Referencias

* Superseded por: `adr-0014-canal-propio-permanente.md`.
* Política de convivencia con el riesgo de baneo que se deriva de la derogación de este ADR:
  `adr-0015-politica-de-convivencia-con-el-baneo.md`.
* `docs/adr/README.md`: fila de este ADR, marcada **Derogada — superseded por `adr-0014`**.
* `docs/bitacora-de-descartes.md`: descartes relacionados con la estrategia de canal.
