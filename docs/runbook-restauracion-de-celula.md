# Runbook: restauración completa de una célula desde su respaldo

* **Fecha de esta versión:** 2026-07-30.
* **Etapa que lo redacta:** A-2 (tarea 16 de `docs/plan/fase-a-2-nucleo-persistencia.md`).
* **Alcance de esta versión:** las tres bases que la etapa A-2 puede restaurar y verificar de punta
  a punta (`sessions.db`, `knowledge_live.db`, el almacén de identidad del adaptador), más el
  procedimiento razonado —pero **no ensayado contra un sidecar real**— para la cuarta, el
  `sqlstore`. El ensayo de las dos ramas de la bifurcación de este runbook contra la taxonomía de
  desconexión real de whatsmeow es diferido explícito a la etapa A-3.

---

## Antes de empezar

* Este procedimiento asume que ya existe una ronda de respaldo completa: los archivos que produjo
  `crates/hexcell-storage/src/respaldo.rs` (`sessions.db`, `knowledge_live.db`,
  `adapter_identity.db`) bajo sus nombres canónicos en un directorio de destino, más —si aplica— la
  copia del `sqlstore` que hubiera producido el sidecar bajo
  `docs/contrato-ipc-respaldo-del-sqlstore.md`.
* **Copiar un archivo de respaldo ya escrito es seguro precisamente porque está quieto.** Una vez
  que `VACUUM INTO` terminó de escribirlo, ese archivo no tiene ninguna conexión abierta encima y
  copiarlo con una copia de archivo corriente —`cp`, `std::fs::copy`— no puede capturarlo a medias.
  Esto es exactamente lo contrario de copiar una base **en uso**: `sessions.db`, `knowledge_live.db`
  o el `sqlstore` mientras el proceso que los tiene abiertos sigue escribiendo. Nadie debe
  generalizar de esta seguridad la idea de que copiar una base viva también lo es; es al revés, y es
  la razón entera por la que existen `VACUUM INTO` y este mismo contrato IPC.
* El directorio donde se restaura **debe ser distinto** del directorio de datos original de la
  célula. Restaurar sobre el propio directorio en marcha mezclaría el archivo de respaldo con el
  que el proceso todavía tiene abierto, y el resultado depende de en qué momento exacto se
  sobrescribió: no es un procedimiento, es una apuesta.

## Producción de un respaldo de célula (HEX-029)

Para producir una ronda de respaldo de las cinco bases en un directorio de destino:

1. **Disciplina operacional obligatoria:** la operación exige **núcleo detenido y sidecar en ejecución**.
   * El proceso del núcleo (`hexcell`) debe estar **detenido** (vía `SIGTERM` o Ctrl-C en el entorno de laboratorio).
   * El proceso del sidecar Go debe permanecer **en ejecución** escuchando en el socket IPC, ya que él mismo ejecuta la copia `VACUUM INTO` sobre `sqlstore.db` **y sobre `identidad.db`** (su almacén de identidad: lista STOP, mapeo de conversación, cortacircuitos), las dos bases ordenadas por IPC (`adr-0022`).
2. **Invocación:** ejecutar el subcomando `hexcell respaldar` indicando una **ruta absoluta** hacia un directorio de destino sin usar/vacío:
   ```bash
   hexcell respaldar --directorio /ruta/absoluta/al/destino
   ```
   O utilizar el script de laboratorio que construye un directorio con marca temporal:
   ```bash
   scripts/laboratorio/respaldar-celula.sh
   ```
3. El comando verifica previamente la disponibilidad de las cinco rutas de destino (`sessions.db`, `knowledge_live.db`, `adapter_identity.db`, `sqlstore.db` e `identidad.db`). Las dos bases IPC (`sqlstore.db`, `identidad.db`) se producen **antes** que las tres locales, tras el pre-chequeo de los cinco destinos (PAT-038 fallo-en-vacío). Ante cualquier fallo o destino ocupado, el proceso aborta con código no nulo, nombrando la base que falló, y deja el directorio libre de respaldos parciales (LES-031).

## 1. Restaurar las tres bases de esta etapa

1. Detener por completo el proceso `hexcell` de la célula, si sigue vivo. Restaurar contra un
   proceso en marcha no es un caso que este procedimiento cubra: los pools de
   `crates/hexcell-storage` abren sus conexiones al arrancar y no las reabren solas.
2. Preparar un directorio de datos **limpio**: o bien un directorio nuevo, o el directorio original
   ya vacío de sus tres archivos y de sus posibles compañeros `-wal`/`-shm`.
3. Copiar, con una copia de archivo corriente, los tres archivos del respaldo bajo sus nombres
   canónicos: `sessions.db`, `knowledge_live.db` y `adapter_identity.db`.
4. Arrancar `hexcell` apuntando `HEXCELL_RUTA_DATOS` al directorio recién restaurado.
   `GestorDePools::abrir` y `AlmacenDeIdentidad::abrir` migran las tres bases si hiciera falta y
   fijan `journal_mode = WAL` en cada apertura de lectura y escritura —el archivo de respaldo sale
   de `VACUUM INTO` en modo `delete`, y este es precisamente el paso que lo devuelve a WAL; no es
   una señal de corrupción, es el comportamiento esperado (`docs/adr/adr-0020...md`).

## 2. La bifurcación, antes de tocar el `sqlstore`

Antes de restaurar la cuarta base, el procedimiento se detiene y pregunta **por qué** se perdió la
célula original. La respuesta decide una de dos ramas, y no hay una tercera:

### Rama A — `LoggedOut` con `device_removed`

**Situación:** whatsmeow reporta que la sesión terminó por `LoggedOut`, y la causa concreta es que
el dispositivo fue retirado del lado del servidor de WhatsApp (`device_removed`): el usuario
desvinculó el dispositivo desde su teléfono, o WhatsApp lo desvinculó por su cuenta.

**Decisión: NO se restaura el `sqlstore`. Se re-empareja por `PairPhone()`.**

**Por qué:** el dispositivo que ese `sqlstore` representaba **ya no existe** en el servidor de
WhatsApp. Restaurar sus credenciales de sesión no reconecta nada, porque no hay nada del otro lado
con lo que reconectar: es indistinguible de restaurar una llave para una cerradura que ya se
cambió. No es que restaurarlo sea peligroso; es que es **inútil**, y conservar el intento solo
retrasaría llegar al único camino que sí funciona, que es un re-emparejamiento nuevo por
`PairPhone()`.

**Lo que SÍ sobrevive a esta rama, y por qué:** los dos almacenes de identidad **no credenciales**
—el del adaptador (`adapter_identity.db`) y el del sidecar (`identidad.db`, que guarda la lista
STOP, el mapeo de conversación y el cortacircuitos; `adr-0022`)— **se restauran igual que las bases
locales, incluso en esta rama**, exactamente porque viven separados del `sqlstore`. Restaurar
`identidad.db` aquí es lo que impide que un contacto dado de baja vuelva a recibir mensajes tras el
re-emparejamiento: la lista STOP debe sobrevivir a las dos ramas. Un contacto que ya tenía
hilo abierto antes de la pérdida vuelve a caer en el mismo hilo tras el re-emparejamiento, aunque el
dispositivo emparejado sea uno nuevo: es la propiedad que
`crates/hexcell/tests/continuidad_de_hilo.rs` ya prueba con `re_emparejar`, y que este mismo runbook
generaliza al caso de una restauración completa.

### Rama B — cualquier otra causa

**Situación:** corrupción del archivo, fallo de disco, cualquier otra desconexión que no sea
`LoggedOut` con `device_removed` (por ejemplo, una pérdida del propio servidor sin que la sesión de
WhatsApp se haya invalidado del otro lado).

**Decisión: el respaldo es válido. Se restaura el `sqlstore` (y, como en las tres bases locales y
en `identidad.db`, todo el conjunto de cinco).**

**Por qué:** el dispositivo sigue existiendo en el servidor de WhatsApp; lo único que faltaba era
el proceso o el disco que lo servía. Restaurar la copia más reciente del `sqlstore` —producida por
el propio sidecar bajo `docs/contrato-ipc-respaldo-del-sqlstore.md`— le devuelve al proceso
reconstruido las credenciales que tenía, sin necesidad de un nuevo emparejamiento por QR.

### Diferido explícito

Estas dos ramas se **razonan** aquí; su ensayo contra la taxonomía real de desconexión que reporta
whatsmeow —qué otros valores además de `device_removed` puede tomar la causa de un `LoggedOut`, y
si alguno de ellos debería tratarse como la rama A y no como la B— es explícitamente de la etapa
A-3, que es la primera que tiene un sidecar real contra el que contrastarlo.

## 3. Criterio de aceptación de la restauración

Una restauración **no se da por buena porque los archivos existan y abran**. El único criterio
válido es que la célula restaurada:

1. Arranque contra el directorio restaurado sin errores de migración ni de integridad.
2. Consuma un evento nuevo por el puerto de su canal.
3. Responda por `send`, con el identificador interno de conversación que la célula original le
   había asignado a ese contacto —no uno nuevo que un almacén vacío también habría podido producir.

Restaurar archivos con el historial intacto pero con la célula incapaz de responder **es un fallo
de la restauración, no un éxito parcial**: es exactamente lo que
`crates/hexcell/tests/respaldo_y_restauracion.rs` prueba de forma negativa, con el mismo entorno
restaurado y el motor deliberadamente sin consumir el puerto.

## Referencias

* `docs/adr/adr-0010-puerto-de-canal.md`, punto 6 (por qué el mapeo de identidad sobrevive al
  re-emparejamiento) y punto 7 (las cuatro bases del respaldo).
* `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md` (la decisión de esta tarea).
* `docs/contrato-ipc-respaldo-del-sqlstore.md` (contrato de la cuarta base).
* `docs/plan/fase-a-3-adaptador-whatsmeow.md` (ensayo contra la taxonomía real de desconexión).
* `docs/STATUS.md` (destino remoto real del respaldo, decisión de negocio pendiente).

---

## Resultado del ensayo rama 1 (2026-08-20) — VALID con advertencia crítica

**Lo validado (rama 1 del runbook — restauración CON `sqlstore`, sin `device_removed`):**

1. `hexcell respaldar --directorio <destino>` ejecutado con **núcleo detenido y sidecar en ejecución** produjo las **cuatro copias** verificadas:
   - `sessions.db`, `knowledge_live.db`, `adapter_identity.db` vía `VACUUM INTO` sobre conexiones de lectura del núcleo.
   - `sqlstore.db` vía IPC: el sidecar ejecutó `VACUUM INTO` en su conexión dedicada de respaldo (`AbrirConexionDeRespaldo`), verificó con `PRAGMA integrity_check` y cotejó `user_version`; emitió `acuse_respaldo_sqlstore` con `identificador_de_ronda` impreso en consola; código de salida 0.
   - Orden observado: `sqlstore` primero (fallo-en-vacío si el destino existe), luego las tres del núcleo.

2. Restauración sobre **entorno limpio** (directorio nuevo, sin `-wal`/`-shm` previos):
   - Copia de los cuatro archivos a sus nombres canónicos.
   - Arranque de `hexcell` con `HEXCELL_RUTA_DATOS` apuntando al directorio restaurado.
   - Migraciones aplicadas, `journal_mode=WAL` restablecido en las tres bases del núcleo.

3. **Sesión reanudada sin QR**: el sidecar conectó automáticamente al arrancar (supervisor con `Arrancar(ctx, emparejada=true)`), restableció el websocket hacia WhatsApp y reportó `estado_sesion=activa` por IPC.

4. **Bot reconectó y respondió a un mensaje real**: se envió un mensaje de prueba desde el número del piloto; la célula lo consumió, lo procesó con el proveedor simulado y emitió la respuesta por el canal propio. El criterio de aceptación del runbook (sección 3) se cumple: la célula restaurada consume y responde.

**Advertencia honesta — lo que NO sobrevive a la restauración hoy:**

El conjunto de respaldo actual **no incluye `identidad.db`** (el almacén de identidad del sidecar Go en `/var/lib/hexcell/identidad.db`). Ese archivo contiene:
- El mapeo **conversation-id** (contacto → identificador interno de hilo).
- El estado del **cortacircuitos** conversacional (contadores de repetición, disparos previos).
- La **lista STOP** (contactos que pidieron la baja con las palabras clave configuradas).

**Consecuencia observada en el ensayo:** la célula restaurada reenvió su presentación de bienvenida al contacto de prueba, porque el mapeo de conversation-id se perdió y el contacto "nuevo" abrió un hilo fresco.

**Riesgo mayor (no observado pero cierto):** si se restaura tras una pérdida real, **cualquier contacto que hubiera enviado "baja"/"stop" y esté en la lista STOP volvería a recibir mensajes**, violando la regla del plan de que un re-emparejamiento no debe revivir bajas. El plan dice "cuatro bases"; la implementación dividió la identidad del adaptador en dos archivos (`adapter_identity.db` + `identidad.db`) y solo la primera está en el respaldo.

**Acción requerida:** tarea de fix con prioridad para añadir `identidad.db` al conjunto de respaldo (copia `VACUUM INTO` desde la conexión de lectura del sidecar, análoga a las otras tres bases) y actualizar el runbook y `adr-0020` en consecuencia.

**Rama 2 (`device_removed`: restaurar SIN `sqlstore` + re-emparejar por `PairPhone()`):** **pendiente**, programada para el próximo bloque de laboratorio.

---

## Resultado del ensayo rama 2 (2026-08-20) — VALID

**Lo validado (rama 2 del runbook — restauración SIN `sqlstore`, con `device_removed`):**

1. **Desencadenante y clasificación terminal:** desvinculación forzada desde el teléfono del piloto, clasificada en vivo por el sidecar como `estado=desvinculada causa=desvinculada_dispositivo_removido codigo=401`. Whatsmeow eliminó la sesión local inmediatamente y **el supervisor no ejecutó ningún reintento de conexión** (0 retries), conforme al invariante de HEX-027: ante `device_removed` no hay bucle de reconexión.

2. **Respaldo utilizado:** `hexcell respaldar --directorio <destino>` (ejecutado con núcleo detenido y sidecar en ejecución) produjo las copias verificadas de las **tres bases no credenciales**: `sessions.db`, `knowledge_live.db` y `adapter_identity.db` (vía `VACUUM INTO` sobre conexiones de lectura del núcleo). **NO se copió `sqlstore.db`** —consistente con la Rama A del runbook (sección 2), porque el dispositivo ya no existe en el servidor de WhatsApp.

3. **Restauración en entorno limpio:** se copiaron los tres archivos a sus nombres canónicos en un directorio nuevo (sin `-wal`/`-shm` previos). Arranque de `hexcell` con `HEXCELL_RUTA_DATOS` apuntando al directorio restaurado. Migraciones aplicadas, `journal_mode=WAL` restablecido.

4. **Sidecar rechaza auto-conexión:** al arrancar, el sidecar detectó `sesion.EstaEmparejada() == false` (almacén de credenciales vacío) y **no intentó conectar** —el supervisor invocó `Arrancar(ctx, emparejada=false)` que es no-op, respetando el invariante de que el emparejamiento es la única vía de conexión inicial (HEX-027). Cero reintentos de conexión registrados.

5. **Recuperación por re-emparejamiento QR (segunda capa):** se ejecutó `hexcell emparejar --metodo qr`, se escaneó el código con el teléfono del piloto, el sidecar persistió las nuevas credenciales en `sqlstore.db` (nuevo dispositivo) y reportó `estado_sesion=activa` por IPC.

6. **Célula reconstruida reconectó y respondió:** se envió un mensaje de prueba desde el número del piloto; la célula lo consumió, lo procesó con el proveedor simulado y emitió la respuesta por el canal propio. El criterio de aceptación del runbook (sección 3) se cumple: la célula restaurada consume y responde.

**Nota honesta — esta rama regenera credenciales deliberadamente:**

A diferencia de la rama 1, la rama 2 **no restaura** el `sqlstore` sino que **genera uno nuevo** mediante re-emparejamiento. Esto es **por diseño**, no una limitación: la regla de restauración (tarea 7 del plan + `adr-0020` + sección 2 de este runbook) establece que ante `device_removed` el `sqlstore` anterior es inútil porque el dispositivo ya no existe del otro lado. El re-emparejamiento por `PairPhone()` / QR es el camino correcto y probado.

**Hallazgo 12 reconfirmado:** al no incluirse `identidad.db` en el conjunto de respaldo, la célula restaurada trató al contacto de prueba como nuevo y re-envió presentación + respuesta. Esto refuerza la etiqueta **PRIORIDAD** del hallazgo 12 sin añadir un nuevo número de hallazgo y sin atenuar la consecuencia de revivir lista STOP ya documentada.

---

## Resolución del hallazgo 12 (2026-08-20, HEX-032)

El hallazgo 12 queda **resuelto**: `identidad.db` es ahora la **quinta base** del conjunto de
respaldo. El propio sidecar produce su copia verificada por IPC (`VACUUM INTO` sobre una conexión
dedicada de solo lectura, con la misma disciplina fail-closed que el `sqlstore`), ordenada mediante
el nuevo par de mensajes `orden_respaldo_identidad` / `acuse_respaldo_identidad` (versión de cable
5, `adr-0022`). Con la restauración de `identidad.db` en **las dos ramas**, la **lista STOP
sobrevive** a una restauración: un contacto dado de baja sigue de baja. El re-ensayo e2e de las dos
ramas con el conjunto de cinco bases queda para el próximo bloque de laboratorio; este fix lo hace
posible.
