# adr-0032 — Protocolo IPC: versión de cable 6 (cierre de sesión y pausa de envío)

* **Estado:** Vigente (2026-09-11).
* **Etapa que lo produce:** A-6 (tarea 24, HEX-071).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0022-respaldo-identidad-sidecar-por-ipc.md`,
  que registró la subida de cable 4 → 5. No supersede a ningún ADR: este ADR registra la evolución
  del protocolo IPC de la versión de cable 5 a la 6.

## Contexto

Las tareas 12 (`cell terminate`) y 13 (`cell rebind`) de la etapa A-6 exigen dos operaciones de canal
que el protocolo IPC no era capaz de expresar: un **cierre de sesión** que desvincule el dispositivo de
WhatsApp antes de destruir volúmenes, y una **orden de pausa de envío** para que la célula no responda
sin sesión durante un re-emparejamiento. El protocolo, cerrado en trece tipos (cable 5), solo conocía
el estado `pausada` como proyección del estado de sesión; no existía ni un tipo de logout ni una orden
de pausa dirigible desde el núcleo. Sin ese vocabulario, `cell terminate` y `cell rebind` quedaban
bloqueados por su propio criterio de aceptación («no se implementa un `terminate` que borre volúmenes
con la sesión viva»).

## Decisión

**Se sube la versión de cable a 6 con cuatro tipos nuevos, cada orden con su acuse:**
`orden_cierre_de_sesion` / `acuse_cierre_de_sesion` y `orden_pausa_de_envio` / `acuse_pausa_de_envio`.
El conjunto cerrado pasa de trece a **diecisiete** tipos. El par de cierre desvincula la sesión y
reporta su desenlace; el par de pausa ordena pausar o reanudar el envío saliente y reporta su desenlace.
Como en las subidas anteriores, el cambio se aplica **en lockstep en los dos lenguajes**
(`VersionProtocolo` en Go, `VERSION_PROTOCOLO` en Rust) y ambos extremos siguen **fallando cerrado**
ante desajuste de versión en el saludo.

## Consecuencias

* La versión de cable del protocolo IPC es **6** (documento 1.5). Todas las instalaciones de prueba
  (fixtures) se actualizan 5 → 6, y la sección 6 del documento lista **diecisiete** tipos.
* `cerrar_sesion` deja de ser un stub y queda implementado en
  `crates/hexcell-canal-whatsmeow/src/adaptador.rs:947`.
* Las tareas 12 (`cell terminate`) y 13 (`cell rebind`) de A-6 quedan **desbloqueadas**: el cierre de
  sesión y la pausa de envío tienen ahora tipo IPC propio.

## Referencias

* `docs/protocolo-ipc-nucleo-sidecar.md`, versión 1.5, sección 6 (los diecisiete tipos).
* `docs/adr/adr-0022-respaldo-identidad-sidecar-por-ipc.md` (precedente de subida 4 → 5).
* `docs/plan/fase-a-6-empaquetado-cli.md`, tareas 12, 13 y 24.
