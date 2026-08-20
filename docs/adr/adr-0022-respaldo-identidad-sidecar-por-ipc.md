# adr-0022 — Respaldo del almacén de identidad del sidecar (`identidad.db`) por un par de mensajes IPC dedicado

* **Estado:** Vigente (2026-08-20).
* **Etapa que lo produce:** A-3 (fix del hallazgo 12 de la sesión de laboratorio del 2026-08-20).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0020-respaldo-y-restauracion-por-celula.md`
  y el contrato `docs/contrato-ipc-respaldo-del-sqlstore.md`. No supersede a ningún ADR: `adr-0010`
  y `adr-0020` siguen vigentes tal cual; este ADR añade una quinta base al conjunto de respaldo y
  registra la evolución del protocolo IPC de la versión de cable 4 a la 5.

## Contexto

El respaldo por célula cubría **cuatro** bases (`adr-0010`, punto 7; `adr-0020`): `sessions.db`,
`knowledge_live.db`, el almacén de identidad del **adaptador** (`adapter_identity.db`) y el
`sqlstore` del sidecar. El ensayo de restauración del 2026-08-20 (hallazgo 12, `docs/STATUS.md`)
descubrió en vivo una **quinta base viva** no cubierta: el almacén de identidad del **sidecar Go**,
`identidad.db` (por omisión `/var/lib/hexcell/identidad.db`), que guarda:

* la **lista STOP / baja** (`sidecar/internal/identidad/baja.go`): contactos que pidieron no recibir
  más mensajes;
* el **mapeo de conversación** (contacto → identificador interno de hilo);
* el **estado del cortacircuitos** conversacional.

**Consecuencia probada en el laboratorio:** una restauración que omite `identidad.db` pierde la
lista STOP, de modo que un contacto dado de baja **volvería a recibir mensajes** tras una
restauración —una violación directa de la regla del plan de que un re-emparejamiento o una
restauración no deben revivir bajas, y un problema real de consentimiento y de daño al usuario. Es
por eso un hallazgo con etiqueta **PRIORIDAD**.

`identidad.db` es un archivo **distinto** de `adapter_identity.db`: la identidad se materializó en
dos archivos, uno por proceso. No se deben confundir; los dos permanecen en el conjunto de respaldo
con nombres canónicos distintos.

Como el sidecar tiene `identidad.db` abierto bajo WAL, **no puede copiarlo un segundo proceso con
seguridad** (mismo argumento que el `sqlstore`: una copia externa puede capturar una página
desgarrada de un WAL vivo). El propio sidecar debe producir la copia con `VACUUM INTO` sobre una
conexión dedicada de solo lectura. Eso obliga a ordenar la copia **sobre el IPC**, y el protocolo
`docs/protocolo-ipc-nucleo-sidecar.md` estaba **cerrado** en once tipos (versión 1.3, cable 4).

## Decisión

**Se añade un par de mensajes IPC dedicado: `orden_respaldo_identidad` / `acuse_respaldo_identidad`,
espejo 1:1 del par del `sqlstore`, con los mismos campos pero un TIPO de mensaje distinto.** El
campo `orden` lleva la cadena fija `respaldar_identidad`.

Esto añade dos tipos al conjunto cerrado (el 12.º y el 13.º), de modo que **la versión de cable
sube 4 → 5 en lockstep en los dos lenguajes** (`VersionProtocolo` en Go, `VERSION_PROTOCOLO` en
Rust), y el documento del protocolo pasa a la versión 1.4. El sidecar aplica a `identidad.db` la
misma máquina de copia verificada y fail-closed que ya usa para el `sqlstore`
(`sidecar/internal/canal/respaldo.go`): captura `user_version` del origen, ejecuta `VACUUM INTO`,
verifica `integrity_check` y `user_version` en la copia y, ante cualquier fallo posterior a la
escritura, elimina la copia sin verificar antes de responder, para no dejar nunca un archivo sin
verificar bajo el nombre canónico `identidad.db`.

El modo de operador `hexcell respaldar` (HEX-029) pasa a producir **cinco** copias verificadas en
vez de cuatro, conservando el orden de fallo-en-vacío (PAT-038): las **dos** bases ordenadas por
IPC (`sqlstore`, `identidad`) se producen **antes** que las tres locales, tras un pre-chequeo de
los cinco destinos, de modo que una disciplina de pausa violada deja el destino **vacío** en vez de
parcial-que-parece-completo.

## Alternativas consideradas y descartadas

### (a) Generalizar la orden del `sqlstore` con un discriminador de almacén — **DESCARTADA**

Reutilizar `orden_respaldo_sqlstore` / `acuse_respaldo_sqlstore` con un campo que indique qué
almacén copiar. Se descarta por dos motivos concretos:

1. **Colisión de correlación.** El adaptador Rust correlaciona los acuses por
   `identificador_de_ronda` en un `HashMap<String, oneshot::Sender<…>>` keyeado **solo por ronda**.
   Dos acuses del **mismo tipo** en la misma ronda —uno del `sqlstore`, otro de identidad—
   colisionarían por clave: el segundo sobrescribiría o no encontraría su `oneshot`. Un TIPO de
   acuse distinto por almacén, con su propio mapa de pendientes, es inequívoco.
2. **Reescritura de un contrato cerrado.** Mutar la orden/acuse del `sqlstore` obligaría a reescribir
   los campos versionados de `docs/contrato-ipc-respaldo-del-sqlstore.md`, secciones 1 y 3, que las
   restricciones de la tarea prohíben tocar. Un par nuevo deja los mensajes del `sqlstore`
   byte-idénticos.

Este descarte se registra además en `docs/bitacora-de-descartes.md` como **D-24**.

### (c) Que un segundo proceso copie `identidad.db` desde fuera — **DESCARTADA (ya en registro)**

Copiar el archivo vivo desde el núcleo o un proceso externo puede capturar una página desgarrada de
un WAL en uso (mismo argumento que el `sqlstore`, `docs/contrato-ipc-respaldo-del-sqlstore.md`). Ya
está descartado por **D-22** (respaldo concurrente sin pausa) y por el criterio general de que la
copia siempre sale del proceso dueño del archivo. El invariante de esta tarea lo reafirma: **el
sidecar produce su propia copia**; el núcleo nunca abre `identidad.db`, ni de solo lectura.

## Consecuencias

* La versión de cable del protocolo IPC es **5** (documento 1.4). Un desajuste v5/v4 rompe el
  saludo: los dos lenguajes se mueven en lockstep y el cambio es **atómico** —no descomponible en
  hijos independientes, porque cualquier estado intermedio no comunica.
* El conjunto de respaldo por célula es de **cinco** bases. Una restauración completa (rama B / rama
  1 del runbook) restaura `identidad.db`, de modo que **la lista STOP sobrevive**: un contacto dado
  de baja sigue de baja tras la restauración. `identidad.db` es **no credencial**, así que también
  se restaura en la rama `device_removed` (rama A / rama 2).
* `adr-0020` y `adr-0010` no se reescriben. Su texto de «cuatro bases» refleja el diseño de la etapa
  A-2; este ADR registra la quinta base descubierta en A-3 y la evolución del protocolo.

## Referencias

* `docs/protocolo-ipc-nucleo-sidecar.md`, versión 1.4, sección 7 (el par de mensajes de identidad).
* `docs/contrato-ipc-respaldo-del-sqlstore.md`, sección 7 (extensión 2026-08-20).
* `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md` (respaldo por célula, extendido por este ADR).
* `docs/adr/adr-0010-puerto-de-canal.md`, punto 7 (los dos almacenes de identidad son archivos distintos).
* `docs/runbook-restauracion-de-celula.md` (restauración de las cinco bases).
* `docs/bitacora-de-descartes.md`, D-24 (descarte de la generalización, opción a).
* `docs/STATUS.md` (hallazgo 12, resuelto el 2026-08-20).
