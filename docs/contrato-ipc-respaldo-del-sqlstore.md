# Contrato IPC del respaldo del `sqlstore` del sidecar

* **Versión de este contrato:** 1.0, fijada el 2026-07-30.
* **Etapa que lo redacta:** A-2 (tarea 14 de `docs/plan/fase-a-2-nucleo-persistencia.md`).
* **Etapa que lo ejecuta:** A-3. Este documento se redacta y se versiona aquí; no existe en este
  repositorio ningún cliente ni servidor IPC que lo hable todavía, porque el sidecar con este
  contrato implementado es entregable de esa etapa, no de esta.
* **Bases a las que se refiere:** la cuarta del respaldo por célula, el `sqlstore` de whatsmeow
  (`docs/adr/adr-0010-puerto-de-canal.md`, punto 7). Las otras tres —`sessions.db`,
  `knowledge_live.db` y el almacén de identidad del adaptador— las respalda directamente el binario
  `hexcell` (`crates/hexcell/src/respaldo.rs`), con `VACUUM INTO` sobre sus propias conexiones; este
  documento no las regula.

---

## Por qué existe este documento y no un cliente IPC

El plan de la etapa A-2 pide dejar el mecanismo del respaldo del `sqlstore` **fijado y versionado**
antes de que exista el sidecar que lo ejecuta, para que la etapa A-3 no tenga que diseñarlo bajo la
presión de un canal real ya en marcha. Un documento versionado se puede revisar, discutir y cambiar
de número de versión sin tocar ningún proceso en producción; un fragmento de código sin sidecar que
lo consuma no se puede probar y solo simula una certeza que no existe todavía.

**No se elige aquí ningún transporte IPC concreto** (socket Unix, tubería nombrada, protocolo
serializado): elegirlo sin el sidecar delante para contrastarlo sería fijar una decisión de
infraestructura sin poder verificarla. Este contrato fija el **mensaje**, el **responsable de
ejecutar la copia**, la **frecuencia** y el **destino**; el mecanismo de transporte concreto se
decide en `adr-0011-whatsmeow-sidecar-e-ipc.md`, todavía por escribir, cuando exista el sidecar
contra el que contrastarlo.

## Por qué el `sqlstore` lo respalda el propio sidecar y no el núcleo ni un proceso externo

El núcleo Rust —o cualquier proceso externo que abriera el archivo del `sqlstore` desde fuera—
**nunca** debe copiar ese archivo directamente mientras whatsmeow lo tiene abierto. Copiar un
archivo SQLite en uso desde fuera del proceso que lo tiene abierto puede capturar una escritura a
medias entre dos páginas, sin que WAL —que solo protege lecturas y escrituras dentro del propio
proceso que abrió la conexión— tenga ninguna manera de evitarlo. La copia resultante puede parecer
válida y solo revelar su corrupción al intentar restaurarla, que es el peor momento posible para
descubrirlo.

Por eso este contrato exige que sea **el propio proceso del sidecar** quien ejecute `VACUUM INTO`
sobre sus propias conexiones abiertas, exactamente el mismo criterio que ya aplica el binario
`hexcell` a `sessions.db`, a `knowledge_live.db` y al almacén de identidad del adaptador
(`crates/hexcell-storage/src/respaldo.rs`): la copia siempre sale de una conexión que el proceso
dueño del archivo ya tiene abierta, nunca de un archivo leído desde fuera.

## 1. Mensaje de disparo

| Campo | Descripción |
| :--- | :--- |
| `orden` | Cadena fija que identifica la orden. En este contrato: `respaldar_sqlstore`. |
| `destino` | Ruta del directorio de destino de la copia, ya resuelta por quien dispara la orden (el núcleo o un futuro orquestador de respaldo). El sidecar no decide el destino; lo recibe. |
| `identificador_de_ronda` | Cadena opaca que agrupa esta orden con las de las otras bases de la misma ronda de respaldo, para que quien audite los registros pueda reconstruir que las cuatro copias corresponden al mismo instante lógico. El sidecar no interpreta su contenido. |

El **quién** dispara este mensaje —el núcleo por sí mismo, un futuro orquestador de respaldo de la
etapa A-6, o un operador humano siguiendo el runbook— es una decisión de la etapa A-3, condicionada
por el mecanismo de transporte que `adr-0011` fije. Este contrato solo fija la forma del mensaje,
no quién lo envía ni por qué canal.

## 2. Quién ejecuta la copia

**El proceso del sidecar, siempre.** Al recibir la orden, el sidecar:

1. Ejecuta `VACUUM INTO` sobre sus propias conexiones al `sqlstore`, respetando el modo WAL de la
   misma manera que `crates/hexcell-storage/src/respaldo.rs` ya lo hace para las otras tres bases:
   la copia sale de una conexión que el proceso ya tiene abierta, nunca de un archivo leído desde
   fuera, y nunca bloquea la conexión que whatsmeow usa para el protocolo en curso.
2. Escribe la copia bajo el destino recibido en el mensaje, con un nombre canónico que la etapa A-3
   fija junto con el resto de la implementación del sidecar.
3. Verifica la copia con el mismo criterio que las otras tres bases: abrir la copia en solo lectura
   y comprobar `PRAGMA integrity_check` y `PRAGMA user_version`, nunca solo su existencia.

El núcleo **nunca** ejecuta `VACUUM INTO` sobre el `sqlstore` ni abre ese archivo directamente por
ningún motivo, ni siquiera de solo lectura: es exclusivamente del sidecar.

## 3. Acuse de vuelta al núcleo

| Campo | Descripción |
| :--- | :--- |
| `identificador_de_ronda` | El mismo recibido en la orden, para que el núcleo pueda correlacionar el acuse con su disparo. |
| `resultado` | `completado` o `fallido`. |
| `ruta_de_la_copia` | Presente solo si `resultado = completado`. |
| `bytes` | Tamaño de la copia, presente solo si `resultado = completado`. |
| `motivo` | Descripción legible del fallo, presente solo si `resultado = fallido`. Nunca lleva ninguna credencial del protocolo ni ningún contenido de mensaje. |

## 4. Frecuencia

**Cada pocas horas, no diaria.** Las credenciales de sesión del protocolo Signal que sostiene
whatsmeow evolucionan de forma continua durante el uso normal del canal, no solo en el momento del
emparejamiento: un respaldo con una frecuencia diaria dejaría, en el peor caso, casi un día entero
de esa evolución sin capturar, y una restauración desde esa copia arrancaría con credenciales ya
desactualizadas frente al servidor de WhatsApp. El valor numérico exacto —cada cuántas horas
concretas— es un parámetro de calibración de la etapa A-3, no de este contrato: aquí se fija el
orden de magnitud (horas, no días) y el porqué.

## 5. Destino de la copia

El mismo criterio que las otras tres bases: un directorio existente, fuera del disco donde vive el
proceso del sidecar, bajo un nombre canónico. **El destino remoto real es una decisión de negocio
pendiente** (`docs/STATUS.md`); este contrato no lo fija, y ningún valor de ejemplo de esta página
debe leerse como una elección ya tomada. Los tests de esta tarea, y los de la etapa A-3, simulan
"fuera del disco" con un segundo directorio local.

## 6. Qué queda fuera de este contrato, a propósito

* El mecanismo de transporte concreto entre el núcleo y el sidecar (socket, tubería, protocolo de
  serialización): decisión de `adr-0011-whatsmeow-sidecar-e-ipc.md`, todavía por escribir.
* Quién dispara la orden en producción y con qué periodicidad exacta: decisión de la etapa A-3 y de
  la etapa A-6 (empaquetado y planificación).
* La ejecución real de este contrato contra un sidecar desplegado: diferida explícitamente a la
  etapa A-3. En el commit de esta tarea no existe ningún cliente ni servidor que lo hable.
* El destino remoto real fuera del servidor: decisión de negocio pendiente en `docs/STATUS.md`.

> **Nota posterior, 2026-07-31 (no altera este contrato ni su versión).** El primer punto de esta
> lista ya tiene respuesta: el mecanismo de transporte y de serialización lo fija
> `docs/protocolo-ipc-nucleo-sidecar.md`, versión 1.0, redactado en la tarea 1 de la etapa A-3, y
> los campos de las secciones 1 y 3 de esta página encajan en él **sin cambio alguno**. Esta nota
> es una referencia hacia adelante y nada más: lo que este contrato fija —el mensaje, el
> responsable, la frecuencia y el destino— sigue exactamente igual, y `adr-0011` continúa siendo
> el ADR que registrará la decisión.

## Referencias

* `docs/adr/adr-0010-puerto-de-canal.md`, punto 7 (las cuatro bases del respaldo).
* `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md` (la decisión de esta tarea).
* `docs/runbook-restauracion-de-celula.md` (procedimiento de restauración, con la bifurcación antes
  de tocar el `sqlstore`).
* `docs/plan/fase-a-3-adaptador-whatsmeow.md` (ejecución real de este contrato).
* `docs/protocolo-ipc-nucleo-sidecar.md` (transporte y formato sobre los que viajan estos
  mensajes, sección 7 de ese documento).
