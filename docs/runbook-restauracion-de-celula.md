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

**Lo que SÍ sobrevive a esta rama, y por qué:** el almacén de identidad del adaptador —el mapa entre
cada contacto y su identificador interno de conversación— **se restaura igual que las otras tres
bases**, exactamente porque vive separado del `sqlstore` desde `adr-0010`. Un contacto que ya tenía
hilo abierto antes de la pérdida vuelve a caer en el mismo hilo tras el re-emparejamiento, aunque el
dispositivo emparejado sea uno nuevo: es la propiedad que
`crates/hexcell/tests/continuidad_de_hilo.rs` ya prueba con `re_emparejar`, y que este mismo runbook
generaliza al caso de una restauración completa.

### Rama B — cualquier otra causa

**Situación:** corrupción del archivo, fallo de disco, cualquier otra desconexión que no sea
`LoggedOut` con `device_removed` (por ejemplo, una pérdida del propio servidor sin que la sesión de
WhatsApp se haya invalidado del otro lado).

**Decisión: el respaldo es válido. Se restaura el `sqlstore`.**

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
