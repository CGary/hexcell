# adr-0040 — Protocolo IPC: versión de cable 7 (restablecimiento de contacto)

* **Estado:** Vigente (2026-09-30).
* **Etapa que lo produce:** A-6 (HEX-091, restablecimiento de contacto de pruebas).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0032-protocolo-ipc-version-de-cable-6.md`,
  que registró la subida de cable 5 → 6. No supersede a ningún ADR: este ADR registra la evolución
  del protocolo IPC de la versión de cable 6 a la 7.

## Contexto

`docs/STATUS.md` mantiene pendiente la superficie del operador para restablecer un contacto de
pruebas: borrar el estado de `cortacircuitos` y `presentacion_de_conversacion` y, solo con una
bandera explícita, la baja de `baja_de_contacto` (la lista STOP de consentimiento protegida por
FR-11). `identidad.db` es propiedad del sidecar y solo se toca por IPC (`adr-0022`); el cliente del
anfitrión no puede abrirla (D-57, D-59). El protocolo cerrado en diecisiete tipos no tenía ninguna
orden capaz de expresarlo.

## Decisión

**Se sube la versión de cable a 7 con dos tipos nuevos:** `orden_restablecer_contacto`
(`contacto`, `incluir_baja`) y `acuse_restablecer_contacto` (`contacto`, `incluir_baja`, `resultado`,
`existe`, `cortacircuitos`, `presentacion_de_conversacion`, `baja_de_contacto`, `motivo`). El
conjunto cerrado pasa de diecisiete a **diecinueve** tipos. Las reglas del protocolo se mantienen:
los booleanos `incluir_baja` y `existe` viajan como las cadenas cerradas `si`/`no` (regla 2) y
`motivo` va siempre presente (regla 4); solo la ruta HTTP del núcleo usa booleanos JSON.

El sidecar comprueba la existencia del contacto en `identidad` y ejecuta los borrados en **una única
transacción** SQLite; cualquier fallo revierte todo. Un `id_interno` ausente da `contacto_desconocido`
sin tocar nada; un contacto existente sin filas es un éxito con contadores en cero. `baja_de_contacto`
se toca únicamente si la orden trae `incluir_baja` exactamente igual a `si`. El cambio se aplica **en
lockstep en los dos lenguajes** (`VersionProtocolo` en Go, `VERSION_PROTOCOLO` en Rust) y ambos
extremos siguen **fallando cerrado** ante desajuste de versión en el saludo.

## Consecuencias

* La versión de cable es **7** (documento 1.6); el documento lista diecinueve tipos y su tabla de
  correspondencia gana la fila `1.6 | 7`.
* **Despliegue en lockstep:** núcleo y sidecar de una célula se actualizan juntos. Un par mezclado
  falla cerrado en el saludo y el bot permanece en silencio, sin pérdida de datos, hasta que ambas
  imágenes coinciden.
* La ruta `POST /admin/contacto/restablecer` no lleva autenticación, como las demás rutas
  administrativas: la frontera de seguridad es la red interna de la célula. Cualquier par de esa red
  podría revivir una baja con `incluir_baja` verdadero; la confirmación adicional (`--confirmar`)
  vive solo en la CLI y no se ensancha aquí.
* Un agotamiento de plazo en la ruta informa `fallido`, pero el sidecar aún puede confirmar el
  borrado: el resultado es desconocido. El restablecimiento es idempotente, así que repetirlo es seguro.
* La CLI, el runbook y la nota de plan quedan fuera de este ADR (tarea hija HEX-091-b).

## Referencias

* `docs/protocolo-ipc-nucleo-sidecar.md`, versión 1.6, secciones 6 y 7.
* `docs/adr/adr-0022-respaldo-identidad-sidecar-por-ipc.md` (titularidad de `identidad.db` y
  precedente de subida 4 → 5).
* `docs/adr/adr-0032-protocolo-ipc-version-de-cable-6.md` (precedente de subida 5 → 6).
* `docs/bitacora-de-descartes.md`, D-51, D-57 y D-59.
