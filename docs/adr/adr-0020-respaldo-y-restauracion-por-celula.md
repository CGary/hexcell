# ADR-0020 — Respaldo y restauración por célula

Decisión: `VACUUM INTO` como mecanismo de respaldo, el almacén de identidad del adaptador como
tercera base real, el contrato IPC del `sqlstore` y la bifurcación de restauración.

* **Estado:** Vigente desde el 2026-07-30.
* **Supersede a:** nada.
* **Etapa:** A-2 (respaldo de las tres bases alcanzables, restauración y ciclo probado de punta a
  punta). Diferido explícito a A-3: ejecución real del contrato IPC del `sqlstore` y ensayo de la
  bifurcación de restauración contra la taxonomía real de desconexión de whatsmeow.
* **Requisitos tocados:** FR-05, FR-12 (por la vía de `adr-0010`).

---

## Contexto

`adr-0010-puerto-de-canal.md` ya fijó, el 2026-07-28, que el respaldo por célula cubre **cuatro**
bases —`sessions.db`, `knowledge_live.db`, el almacén de identidad del adaptador y el `sqlstore` del
sidecar— y que el almacén de identidad vive separado del `sqlstore` precisamente para sobrevivir al
re-emparejamiento que sigue a una desvinculación con dispositivo retirado. Lo que esa decisión no
fijó, porque no era su alcance, es el **mecanismo** con el que las tres primeras bases se copian en
caliente sin interrumpir a la célula, ni el almacén de identidad como una base real —hasta esta
tarea era un mapa en memoria sin archivo detrás—, ni el procedimiento de restauración que confronta
esa bifurcación con un criterio de aceptación verificable.

## Decisión

1. **Las tres bases alcanzables desde esta etapa se respaldan con `VACUUM INTO`, ejecutado sobre
   conexiones de lectura que el proceso ya tiene abiertas.** `sessions.db` y `knowledge_live.db` a
   través de `con_lectura` de sus pools respectivos; el almacén de identidad, a través de su propia
   conexión de solo lectura. Nunca sobre `con_escritura`, y nunca abriendo una conexión nueva solo
   para el respaldo. Comprobado el 2026-07-30 contra `sqlite3` 3.53.4: `VACUUM INTO` funciona sobre
   una conexión de solo lectura y la copia resultante supera `integrity_check` —lo contrario de
   `PRAGMA wal_checkpoint`, que HEX-007 ya comprobó que falla ahí—, así que el respaldo nunca puede
   bloquear al escritor del camino caliente ni producir `SQLITE_BUSY` contra él.
2. **El almacén de identidad del adaptador se materializa como una base SQLite real,
   `adapter_identity.db`**, abierta con el mismo mecanismo que `sessions.db` (una conexión de
   escritura, una de lectura, los mismos parámetros de conexión), con su propia migración y su
   propio `PRAGMA user_version`. Antes de esta tarea era el campo `contactos` de `EstadoInterno` en
   `crates/hexcell-canal-simulado/src/adaptador.rs`, un `HashMap` sin archivo detrás: esto no amplía
   `adr-0010`, lo ejecuta, porque esa decisión ya exigía que el mapeo persistiera en un almacén
   propio del adaptador.
3. **`AlmacenDeIdentidad` guarda dos columnas de texto opaco** —`contacto`, `identificador_interno`—
   y no conoce el tipo `IdConversacion` del dominio: acuñar ese identificador sigue siendo
   responsabilidad exclusiva de `AdaptadorSimulado::inyectar_desde_contacto`, que ahora lo deriva
   del conteo de contactos ya registrados (`contactos_registrados()`) y no del nombre del contacto,
   para que el identificador dependa del orden de primera vista y una restauración se pueda probar
   sin ambigüedad.
4. **El contrato IPC del respaldo del `sqlstore` se redacta y se versiona como documento**
   (`docs/contrato-ipc-respaldo-del-sqlstore.md`), fijando el mensaje de disparo, que es **el propio
   sidecar** quien ejecuta `VACUUM INTO` sobre sus conexiones —nunca el núcleo, nunca un proceso
   externo leyendo el archivo—, la frecuencia (cada pocas horas, por la evolución continua de las
   credenciales del protocolo Signal) y el destino. Su ejecución real, contra un sidecar que todavía
   no existe con este contrato implementado, es explícitamente de la etapa A-3.
5. **El runbook de restauración** (`docs/runbook-restauracion-de-celula.md`) confronta, antes de
   tocar el `sqlstore`, sus dos únicas ramas: `LoggedOut` con `device_removed` no lo restaura y
   re-empareja por `PairPhone()`, porque el dispositivo ya no existe en el servidor de WhatsApp y
   restaurar sus credenciales sería restaurar la llave de una cerradura ya cambiada; cualquier otra
   causa restaura el respaldo, porque el dispositivo sigue existiendo del otro lado. El ensayo de
   estas dos ramas contra la taxonomía real de desconexión de whatsmeow es diferido a la etapa A-3.
6. **Ninguna operación de respaldo tiene disparador de producción en esta tarea.** Ni un
   planificador, ni una ruta HTTP, ni un subcomando de CLI: la especificación de esta tarea no lo
   pide, el apagado ordenado es de HEX-007 y sus metas descartadas prohíben reabrirlo, y el
   empaquetado y la planificación son de la etapa A-6. `respaldar_celula` es una operación de
   biblioteca cuyos únicos llamantes de este diff son los tests de integración; queda anotado como
   decisión, no como hueco, también en `docs/STATUS.md`.
7. **El destino real de respaldo remoto, fuera del servidor, sigue sin decidirse.** Se simula en los
   tests con un segundo directorio local, y queda como decisión de negocio pendiente en
   `docs/STATUS.md`.

## Consecuencias

### Positivas

* **El respaldo no compite nunca con el camino caliente.** Bajo WAL, una lectura nunca bloquea al
  escritor, y el motor entero escribe a través de `con_escritura`: el respaldo no puede introducir
  `SQLITE_BUSY` ni latencia perceptible en el procesamiento de eventos.
* **Un destino ya ocupado o inalcanzable falla antes de la primera copia**, porque `VACUUM INTO`
  rechaza sobrescribir: no puede quedar una ronda de respaldo a medias por un descuido de rutas.
* **La restauración tiene un criterio verificable y no vacío**, porque el identificador que se
  acuña depende del orden de primera vista: una restauración real y un almacén vacío no pueden
  producir la misma respuesta para el segundo contacto en adelante.
* **La continuidad del hilo tras un re-emparejamiento, ya probada por HEX-007 con el mapa en
  memoria, se generaliza al caso de una restauración completa** sin ningún cambio de diseño nuevo:
  el mismo mecanismo —clave por contacto, nunca por dispositivo— es lo que hace ambas cosas
  ciertas a la vez.

### Negativas

* **El almacén de identidad es ahora una cuarta base que puede desincronizarse de las otras tres**
  si alguien restaura solo un subconjunto de los archivos de una ronda. El runbook lo trata como un
  conjunto y no como archivos sueltos, precisamente por esto.
* **`crates/hexcell-canal-simulado` gana una dependencia de `hexcell-storage`** que no tenía. Es el
  precio de que la acuñación de identidad —que `adr-0010` ya le asigna al adaptador— pueda persistir
  de verdad; la alternativa, dejar el almacén fuera del adaptador, habría vuelto a poner la
  traducción de identidad en una capa que `adr-0010` ya descartó por responsabilidad duplicada.
* **Sin disparador de producción, `respaldar_celula` es código que un revisor desprevenido puede
  leer como muerto.** Queda anotado explícitamente aquí, en el runbook y en `docs/STATUS.md` para
  que se lea como una frontera de alcance deliberada.

## Alternativas consideradas y descartadas

### A. La API de respaldo en línea de `rusqlite` (`Connection::backup`)

Reinicia su copia cada vez que un escritor confirma una transacción, así que bajo un escritor activo
puede no terminar nunca. `VACUUM INTO` toma una única instantánea de lectura, no necesita ninguna
característica adicional de `rusqlite` y produce un archivo defragmentado. Descartada; registrada
como **D-19** en la bitácora de descartes.

### B. Un planificador de respaldo dentro del propio proceso de la célula

La planificación pertenece al empaquetado de la etapa A-6; un temporizador por célula duplicaría el
trabajo de un futuro orquestador sobre un presupuesto de memoria de 80 MB por célula. Descartada;
registrada como **D-20**.

### C. Guardar el mapeo de identidad dentro del `sqlstore`

Ya descartada por `adr-0010` como alternativa C (registrada allí como **D-15**); esta tarea no la
reabre, solo ejecuta la decisión ya tomada de mantenerlo separado.

## Referencias

* `docs/adr/adr-0010-puerto-de-canal.md`, puntos 5, 6 y 7.
* `docs/adr/adr-0003-persistencia-dual.md` (parámetros de conexión que este almacén reutiliza).
* `docs/adr/adr-0018-apagado-ordenado.md` (por qué el respaldo no toca el punto de control del WAL
  de apagado, que sigue siendo de esa tarea).
* `docs/contrato-ipc-respaldo-del-sqlstore.md`, `docs/runbook-restauracion-de-celula.md`.
* `docs/plan/fase-a-2-nucleo-persistencia.md` (tareas 13, 14, 16).
* `docs/plan/fase-a-3-adaptador-whatsmeow.md` (ejecución real diferida).
* `docs/bitacora-de-descartes.md`: D-15, D-19, D-20.
* `docs/STATUS.md`: destino remoto real del respaldo y ausencia de disparador de producción
  (2026-07-30).
