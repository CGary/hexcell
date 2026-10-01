# Runbook: operación de células con hexcell-admin

* **Fecha de esta versión:** 2026-09-30 (redactado el 2026-09-24; alineado el 2026-09-30 con el comportamiento que entregó HEX-087).
* **Tarea que lo redacta:** HEX-088 (tarea 21 de `docs/plan/fase-a-6-empaquetado-cli.md`); alineado con HEX-087 (tarea 15 del mismo plan).
* **Alcance de esta versión:** procedimiento de operación de los subcomandos `cell` y `config render` de `hexcell-admin`, más el procedimiento de respuesta ante `OOMKilled`. Los cuatro comandos de ciclo de vida (`cell pause`, `cell unpause`, `cell terminate`, `cell rebind`) son reejecutables: cada sección describe qué hace una reejecución, y la sección «Reejecución de un comando» las reúne.

---

## Qué es esto

Este runbook le dice al operador **qué comando ejecutar ante cada situación operativa** con una célula, **qué efecto tiene** ese comando sobre los contenedores, el almacén del plano de control y la sesión de canal, y **cómo verificar** que salió bien. Cubre los ocho comandos que `hexcell-admin` opera sobre una célula (`cell pause`, `cell unpause`, `cell terminate`, `cell rebind`, `cell list`, `cell status`, `reporte tokens`, `config render`) más el procedimiento de respuesta ante un contenedor muerto por límite de memoria (`OOMKilled`).

Lo que este runbook **no** cubre:

* **Restauración de una célula desde respaldo** — es el ámbito de [docs/runbook-restauracion-de-celula.md](docs/runbook-restauracion-de-celula.md). Este runbook opera células vivas; no repone datos perdidos.
* **Vigilancia externa de vida del anfitrión** (dead-man's switch) — es el ámbito de [docs/runbook-vigilancia-externa.md](docs/runbook-vigilancia-externa.md).
* **Operación del canal** en sí — emparejamiento inicial, reconexión de sesión, sustitución completa de número y respuesta ante baneo. Esos procedimientos viven en el plan de la etapa A-7 (`docs/plan/fase-a-7-pilotos.md`). La sustitución de número que se documenta aquí es exclusivamente el **cómo técnico** del comando `cell rebind`; el **cuándo procede** decidirlo es el runbook de baneo, que es un documento distinto y pendiente.

---

## Antes de empezar

* **`hexcell-admin` se ejecuta en el anfitrión**, no dentro de un contenedor. Se distribuye como binario nativo compilado desde `crates/hexcell-admin`.
* **Depende del socket Unix de Docker** (`/var/run/docker.sock`). El usuario que ejecuta `hexcell-admin` debe tener permiso de lectura y escritura sobre ese socket; sin él, los subcomandos `cell` fallan con código 1 antes de emitir ninguna petición. `config render` y `reporte tokens` no tocan Docker.
* **`HEXCELL_IMAGEN_SONDA`** — variable de entorno que nombra la imagen que los contenedores hermanos usan para operar dentro de la red de la célula: borrado de `sqlstore`, cierre de sesión y emparejamiento (`cell rebind`, `cell terminate`), y también la sonda de disponibilidad `GET /health/ready` de `cell unpause` y `cell status`. Su valor por omisión es `alpine:3` (documentado en la nota de cierre de HEX-085, 2026-09-23). No se instancia ninguna célula con esta imagen; es únicamente la herramienta con la que `hexcell-admin` opera sobre las ya instanciadas. Debe estar presente en Docker de antemano (`docker pull`): si falta, `cell unpause` falla con código 1.
* **Almacén del plano de control** — SQLite configurable con `HEXCELL_ADMIN_ALMACEN` (por omisión `/var/lib/hexcell-admin/plano_de_control.db`). Los subcomandos que persisten estado lo abren en lectura/escritura y migran; los de sólo lectura (`list`, `status`) usan el descriptor de sólo lectura de SQLite, sin crear el archivo ni migrar. Un `HEXCELL_ADMIN_ALMACEN` cuyo directorio padre no existe falla con diagnóstico en vez de crear una base nueva.
* **Sólo lectura por construcción:** `cell list` y `cell status` no reparan discrepancias. DISC-05 se reporta y la fila no se crea: el operador decide si es alta implícita legítima o inconsistencia.

---

## Situación → comando

| Situación | Comando a ejecutar |
| :--- | :--- |
| Falta de pago / pausa temporal de la actividad | `hexcell-admin cell pause --id <celula_id>` |
| Reactivación tras pausa | `hexcell-admin cell unpause --id <celula_id>` |
| Baja definitiva de un cliente | `hexcell-admin cell terminate --id <celula_id> --confirmar` (vale también para una célula pausada —p. ej. la ruta impago → pausa → baja definitiva—: `terminate` tolera contenedores detenidos, congelados o ausentes y no exige `cell unpause` antes) |
| Sustitución de número por baneo permanente o apelación fracasada | `hexcell-admin cell rebind --id <celula_id> --motivo "<motivo>" --confirmar [--metodo qr|codigo_de_vinculacion]` |
| Ver el estado de una célula | `hexcell-admin cell status --id <celula_id>` |
| Listar todas las células conocidas | `hexcell-admin cell list` |
| Reporte de consumo de unidades de presupuesto por conversación | `hexcell-admin reporte tokens --celula <celula_id> --copia <ruta.db> [--desde AAAA-MM-DD] [--hasta AAAA-MM-DD]` |
| Renderizar la configuración de una célula (defecto + superposición) | `hexcell-admin config render --defecto <ruta_defecto.env> --superposicion <ruta_superposicion.env> --salida <ruta_salida.env>` |

---

## 1. `cell pause` — suspender temporalmente una célula

**Cuándo:** falta de pago, mantenimiento acordado, o cualquier situación que exija detener la actividad de una célula sin destruirla.

**Comando:**

```bash
hexcell-admin cell pause --id <celula_id>
```

**Efecto:**

* Detiene el sidecar primero —cierra el websocket saliente y la entrada de mensajes cesa por construcción— y, una vez completamente detenido, señala el núcleo con `SIGTERM` y el margen de gracia de `deploy/cell.compose.yml` (`stop_grace_period: 30s`) hace el drenaje del WAL.
* Persiste el estado `Suspendida` en el almacén del plano de control, con alta implícita si la célula no tenía fila previa.
* La célula no emite ni recibe mensajes durante la pausa. Ninguna respuesta pendiente se entrega al reanudar.

**Verificación:**

```bash
hexcell-admin cell status --id <celula_id>
```

* Esperado: `estado: suspendida`, `docker nucleo: exited`, `docker sidecar: exited`.
* Los códigos **NO** deben aparecer: DISC-01 (almacén `en_ejecucion` con contenedor detenido), DISC-02 (almacén `suspendida` con contenedor corriendo), DISC-03, DISC-04, DISC-05.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito | — |
| 1 | Fallo de ejecución: el almacén no abrió, la transición no se validó, Docker no respondió, o (en una reejecución) falta el núcleo o el sidecar y el diagnóstico es «célula no encontrada» | Revisar diagnóstico en stderr; verificar que `HEXCELL_ADMIN_ALMACEN` apunta a un directorio existente y que el socket de Docker es accesible |
| 2 | Uso incorrecto: faltó `--id` o se aportó una opción no admitida | Revisar diagnóstico y texto de uso en stderr |

**Reejecución:** `cell pause` sobre una célula que el almacén ya tiene `suspendida` no se rechaza: inspecciona ambos contenedores y detiene sólo el que haya quedado corriendo (sidecar primero; uno congelado se descongela y se detiene). Si detuvo alguno, emite por stdout `cell pause completado para «<celula_id>»` con código 0; si ambos ya estaban detenidos, emite por stderr «sin cambios: la célula ya está suspendida» con código 0. En ningún caso registra una transición ni toca el almacén.

> **Nota:** el código 3 (`NoImplementadoTodavia`) está reservado y ningún subcomando `cell` lo devuelve hoy (ver la nota de estado del 2026-09-22 en README.md).

---

## 2. `cell unpause` — reactivar una célula

**Cuándo:** reactivación tras pausa por pago, fin del mantenimiento, o retorno acordado del servicio.

**Comando:**

```bash
hexcell-admin cell unpause --id <celula_id>
```

**Efecto:**

* Arranca ambos contenedores de la célula. El sidecar reanuda la sesión whatsmeow desde sus credenciales persistidas —sin re-escanear el QR— como condición previa de la readiness.
* La CLI sondea `GET /health/ready` del contenedor hermano cada 100 ms hasta la primera confirmación positiva, que exige pools SQLite operativos **y** sesión de canal activa.
* Persiste el estado `EnEjecucion` en el almacén del plano de control.

**Verificación:**

```bash
hexcell-admin cell status --id <celula_id>
```

* Esperado: `estado: en_ejecucion`, `docker nucleo: running`, `docker sidecar: running`, `salud: listo`.
* Ningún `DISC-0N` (ver sección 6 para la lista completa).

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito — la célula procesa mensajes | — |
| 1 | Fallo: contenedores arrancaron pero la sesión no se reanudó, la imagen de `HEXCELL_IMAGEN_SONDA` no existe en Docker, o se agotó el plazo de sondeo de `/health/ready` | Verificar credenciales del sidecar si la sesión no reanudó; si el diagnóstico nombra la imagen de sonda, ejecutar `docker pull` de esa imagen antes de reintentar; si se agotó el plazo, revisar la salud del contenedor hermano |
| 2 | Uso incorrecto: faltó `--id` | Revisar diagnóstico en stderr |

**Reejecución:** `cell unpause` sobre una célula que el almacén ya tiene `en ejecución` no se rechaza: inspecciona ambos contenedores, arranca sólo el que no esté corriendo y confirma la disponibilidad con la misma sonda de `/health/ready`. Si arrancó alguno, emite por stdout `cell unpause completado para «<celula_id>»`; si todo ya corría y la sonda confirma, emite por stderr «sin cambios: la célula ya está en ejecución». Ambos casos salen con código 0 y ninguno registra una transición. Si la sonda no confirma, falla igual que un `cell unpause` normal (código 1); si falta el núcleo o el sidecar, el diagnóstico es «célula no encontrada» (código 1).

---

## 3. `cell terminate` — eliminar definitivamente una célula

**Cuándo:** baja definitiva de un cliente o destrucción acordada de la célula. Operación **destructiva**: cierra la sesión de canal, destruye ambos contenedores y elimina el volumen de datos físicamente, incluidas las credenciales.

**Precondiciones:** ninguna sobre el estado de los contenedores. `cell terminate` ya no exige que el núcleo esté `running`: una célula pausada (`cell pause` previo, p. ej. la ruta impago → pausa → baja definitiva) se retira directamente, sin `cell unpause` previo. Tolera contenedores detenidos, congelados (`paused`) o ausentes. Con el núcleo detenido, o si sólo queda el sidecar, no hay sesión que cerrar: el cierre se omite con un aviso por stderr y la destrucción continúa. El único caso que falla por recursos ausentes es el de una célula **sin fila en el almacén y sin ningún contenedor** en Docker: el diagnóstico es «célula no encontrada» y no se toca nada.

**Comando:**

```bash
hexcell-admin cell terminate --id <celula_id> --confirmar
```

**Efecto:**

1. Inspecciona núcleo y sidecar. Si el núcleo existe, resuelve el nombre del volumen de su inspección (no del `--id`) y lo avisa por stderr, antes de cualquier parada, como «volumen de la célula: <nombre>».
2. Si el núcleo está en ejecución, cierra la sesión whatsmeow vía contenedor hermano (`POST /admin/sesion/cierre`), desvinculando el dispositivo. El cierre es **a mejor esfuerzo**: si el núcleo responde 502/504 o la sonda sale con código distinto de cero, la CLI escribe por stderr «aviso: el cierre de sesión devolvió código N; se continúa igual» y sigue con la destrucción. Si el núcleo no está en ejecución, escribe «aviso: el núcleo no está en ejecución; se omite el cierre de sesión»; si no existe, «aviso: el núcleo no existe; se omite el cierre de sesión». Un fallo de Docker en este paso (imagen de sonda ausente, demonio inalcanzable) sí aborta antes de destruir nada.
3. Detiene sidecar y núcleo con el margen de gracia de `deploy/cell.compose.yml` (un contenedor congelado se descongela antes de detenerse; uno ya detenido se omite).
4. Elimina ambos contenedores y el volumen (nombre leído de `docker inspect`). Un volumen que ya no existe no es un error.
5. Persiste `Retirada` con motivo `sesion_cerrada` tras el éxito de los pasos anteriores. El motivo es el mismo aunque el cierre se haya omitido o haya fallado.
6. Emite por stdout: `sesión cerrada` **sólo si el cierre llegó a completarse**, `contenedores eliminados` y `volumen <nombre> eliminado` (si el volumen se pudo resolver).

**Verificación:**

```bash
hexcell-admin cell status --id <celula_id>
```

* Esperado: `estado: retirada`, `docker nucleo: ausente`, `docker sidecar: ausente`.
* **`DISC-04` (el almacén tiene fila pero los contenedores no existen en Docker) es el resultado esperado y correcto, no una falla.** `cell status` lo sigue reportando y sale con código 1 por construcción: la fila persiste con `estado: retirada` mientras que los contenedores fueron eliminados, y esa combinación es exactamente la que dispara `DISC-04` (`comandos.rs:937-940`). Confirmar que la fila conserva `retirada` es la verificación real; el código de salida 1 de `cell status` en este caso no indica un problema.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito | — |
| 1 | «célula no encontrada» (sin fila y sin contenedores), fallo del almacén, o fallo de Docker en algún paso (imagen de sonda ausente, demonio inalcanzable, detención o eliminación) | Revisar diagnóstico en stderr. Un fallo de Docker deja la secuencia detenida en ese paso: reejecutar el mismo `cell terminate --confirmar` (ver «Fallos parciales y limpieza manual» y «Reejecución de un comando»). Un cierre de sesión fallido **no** es un fallo: sale como aviso y el comando termina en 0 |
| 2 | Uso incorrecto: faltó `--id` o `--confirmar` | Revisar diagnóstico y texto de uso |

> **Importante:** sin `--confirmar` el comando devuelve `UsoIncorrecto` (código 2) y no toca nada. Esta exigencia es la misma que aplica a `cell rebind`.

### Fallos parciales y limpieza manual

**Volumen huérfano.** El nombre del volumen de datos se resuelve **únicamente** de la inspección del núcleo (`Mounts[].Name` sobre `/var/lib/hexcell`); nunca se deriva por convención ni se lee del sidecar. Si el núcleo ya no existe, la CLI no puede resolverlo: escribe por stderr «aviso: no se pudo resolver el volumen de datos porque el núcleo ya no existe; si quedó, bórrelo a mano con docker volume rm <nombre>» y no borra ningún volumen. El operador lo borra a mano:

```bash
docker volume ls
docker volume rm <nombre>
```

El nombre es el valor de `HEXCELL_VOLUMEN_CELULA` de esa célula (ver `deploy/celula.defecto.env.ejemplo` y `deploy/celula.superposicion.env.ejemplo`); `docker volume ls` permite confirmar que existe antes de borrarlo. Cuando el núcleo sí existía al empezar, la CLI ya avisó el nombre («volumen de la célula: <nombre>») antes de la primera parada. Una reejecución de `cell terminate --confirmar` sobre esa célula ya `retirada` y sin contenedores responde «sin cambios: la célula ya está retirada» y **no** puede borrar el volumen: la limpieza manual es la única salida.

**Retiro parcial.** Si `cell terminate` quedó a medias —el almacén sigue en `en ejecución` o `suspendida` pero los contenedores ya no existen, están detenidos o congelados—, se reejecuta el mismo `hexcell-admin cell terminate --id <celula_id> --confirmar`. Con fila y sin ningún contenedor en Docker, termina en código 0 con dos avisos por stderr («aviso: ni el núcleo ni el sidecar existen en Docker; no queda nada que detener ni borrar» y el aviso del volumen) y persiste `Retirada`. Con contenedores detenidos o congelados, completa la destrucción y persiste `Retirada` igual que una primera ejecución (sin cierre de sesión si el núcleo no está en ejecución).

---

## 4. `cell rebind` — sustituir el número de una célula

**Cuándo:** la salida técnica de un baneo permanente o una apelación fracasada. **No** es un alta nueva: la célula, su conocimiento y la memoria del bot por contacto sobreviven a la sustitución. El **cuándo procede** decidirlo —y el **cuándo no**— es el runbook de baneo (`docs/plan/fase-a-7-pilotos.md`), que clasifica el incidente, define la prohibición de reconectar en bucle ante un baneo temporal, el guion de apelación, la plantilla de comunicación al cliente y el aviso a los contactos. Ese runbook **no existe todavía**: este documento sólo documenta **cómo** se ejecuta técnicamente el remplazo, no cuándo es procedente hacerlo.

**Comando (forma mínima, método por omisión `qr`):**

```bash
hexcell-admin cell rebind --id <celula_id> --motivo "<motivo>" --confirmar
```

El método de emparejamiento se elige con `--metodo`, omitido por omisión (`qr`). Para emparejamiento por código de vinculación en lugar de QR:

```bash
hexcell-admin cell rebind --id <celula_id> --motivo "<motivo>" --confirmar --metodo codigo_de_vinculacion
```

**Efecto (la secuencia de HEX-085, tarea 13):**

1. Abre el almacén y lee la fila: sin fila o `EnEjecucion` → secuencia completa; `Reemparejando` → reanuda en el paso 8 (ver «Reanudación» más abajo); `Suspendida` → falla con «ejecute cell unpause antes de cell rebind».
2. Inspecciona el núcleo para resolver los datos de la célula (red, puerto, volumen).
3. Pausa el envío de la célula (la célula **no** intenta responder sin sesión).
4. Cierre de sesión a mejor esfuerzo —un fallo se escribe por diagnóstico y la secuencia continúa, porque tras un baneo el cierre suele fallar—.
5. Persiste `Reemparejando` con el motivo aportado.
6. Detiene el sidecar y descarta su `sqlstore`.
7. Rearranca el sidecar con la pausa de envío reaplicada.
8. Solicita emparejamiento por QR o código de vinculación (omisión: `qr`). La cadena se emite por stdout con la nota de que el renderizado gráfico no está integrado; usar un renderizador QR externo.
9. Sondea la sesión hasta `activa`.
10. Reanuda el envío y persiste `EnEjecucion` con una fila en `sustituciones` (id de célula, motivo, fecha absoluta en ms).

**Conservado:** `sessions.db`, `knowledge_live.db`, el almacén de identidad del adaptador (identidad de conversación y lista STOP), `identidad.db` y `outbox.db` del sidecar. **Descartado:** sólo `sqlstore.db` (con `-wal` y `-shm`). **Reanudación:** una célula en `Reemparejando` se reanuda con el **mismo** `cell rebind`, que salta al paso 8 (no repite los pasos 2 a 7). Antes de emparejar, la reanudación consulta la sesión del núcleo (`GET /admin/sesion`):

* Si la sesión ya está `activa`, escribe por stderr «la sesión ya está activa: se omite el emparejamiento», omite los pasos 8 y 9 (la célula quedó en `Reemparejando` por un fallo posterior al emparejamiento) y continúa en el paso 10.
* Si la solicitud de emparejamiento responde `ya_emparejada` (la sesión del sidecar sobrevivió al fallo anterior), la reanudación descarta el `sqlstore`, rearranca el sidecar y reintenta el emparejamiento **exactamente una vez más**. Un segundo `ya_emparejada`, o cualquier otro motivo de rechazo, termina en fallo con la fila en `Reemparejando`, lista para otra reejecución.
* Si la consulta de la sesión falla, el comando aborta con fallo en vez de adivinar; la fila sigue en `Reemparejando`.

**Verificación:**

```bash
hexcell-admin cell status --id <celula_id>
```

* Esperado: `estado: en_ejecucion`, `docker nucleo: running`, `docker sidecar: running`, `salud: listo`, y al menos una fila en `sustituciones` con el motivo y la fecha.
* Ningún `DISC-0N`.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito | — |
| 1 | Fallo en algún paso de la secuencia | Revisar diagnóstico en stderr. Si quedó en `Reemparejando`, reejecutar el mismo comando: la reanudación consulta la sesión y recupera un `ya_emparejada` una sola vez (ver «Reanudación»). Si tras esa recuperación el emparejamiento vuelve a fallar, la fila sigue en `Reemparejando` y el comando puede reejecutarse |
| 2 | Uso incorrecto: faltó `--id`, `--motivo` o `--confirmar` | Revisar diagnóstico y texto de uso |

---

## 5. `cell list` — listar todas las células conocidas

**Cuándo:** inventario rápido, verificación de cuántas células existen antes de una operación, o auditoría de cobertura entre el almacén y Docker.

**Comando:**

```bash
hexcell-admin cell list
```

**Efecto:**

* No toca ningún contenedor ni persiste ningún estado.
* Imprime la unión de células del almacén del plano de control y de Docker: por cada una, su id, el estado almacenado (o `sin_fila` si sólo existe en Docker) y el estado Docker de núcleo y sidecar.
* No sonda la salud.

**Verificación:**

* Esperado: una línea por célula con el formato `<id> estado=<estado> nucleo=<estado_nucleo> sidecar=<estado_sidecar>`.
* No aplica la verificación de DISC-0N: este comando no cruza fuentes, sólo las une.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito — la lista se produjo | — |
| 1 | Fallo al abrir el almacén o al listar contenedores en Docker | Revisar diagnóstico en stderr |
| 2 | Uso incorrecto: `cell list` no admite `--id` ni ninguna opción de identificación de célula (el analizador de argumentos lo rechaza) | Revisar diagnóstico y texto de uso |

---

## 6. `cell status` — estado consolidado de una célula

**Cuándo:** diagnóstico operativo, verificación posterior a un comando, o investigación de una discrepancia reportada por las alertas.

**Comando:**

```bash
hexcell-admin cell status --id <celula_id>
```

**Efecto:**

* Cruza las tres fuentes (almacén, Docker, sonda de salud) y reporta el estado almacenado, el estado Docker de núcleo y sidecar, la salud (`listo`, `no_listo` o `inalcanzable`), el historial de sustituciones y los códigos de discrepancia detectados.
* Los cinco códigos de discrepancia:
  * **DISC-01:** el almacén indica `en_ejecucion` pero un contenedor no está corriendo.
  * **DISC-02:** el almacén indica `suspendida` pero un contenedor está corriendo.
  * **DISC-03:** los contenedores corren pero `/health/ready` no confirma disponibilidad.
  * **DISC-04:** el almacén tiene una fila pero los contenedores no existen en Docker.
  * **DISC-05:** los contenedores existen en Docker pero el almacén no tiene fila.
* Sale con código 0 si no hay discrepancias, o código 1 si hay al menos una.
* Una fuente que **falla** no es una discrepancia: si `docker inspect` devuelve un error que no sea «no encontrado», nombra la fuente Docker y el contenedor en el diagnóstico y sale con código 1 sin emitir ningún `DISC-0N`.

**Verificación:**

```bash
hexcell-admin cell status --id <celula_id>
```

* Para una célula sana en ejecución: `estado: en_ejecucion`, `docker nucleo: running`, `docker sidecar: running`, `salud: listo`. Ningún `DISC-0N`.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito — sin discrepancias | — |
| 1 | Al menos una discrepancia detectada (o fuente Docker fallada) | Inspeccionar los `DISC-0N` del diagnóstico; cada uno apunta a una acción concreta |
| 2 | Uso incorrecto: faltó `--id` | Revisar diagnóstico y texto de uso |

---

## 7. `reporte tokens` — consumo de unidades de presupuesto por conversación

**Cuándo:** facturación interna, verificación de consumo por cliente, o conciliación contable.

**Comando (sin periodo — toda la historia):**

```bash
hexcell-admin reporte tokens --celula <celula_id> --copia <ruta.db>
```

Con periodo opcional en UTC (`--desde` inclusivo, `--hasta` exclusivo):

```bash
hexcell-admin reporte tokens --celula <celula_id> --copia <ruta.db> --desde 2026-01-01 --hasta 2026-02-01
```

**Efecto:**

* **No abre `sessions.db` caliente.** `--copia` es una copia `VACUUM INTO` producida por el respaldo de A-2. Rechaza por nombre toda copia llamada `sessions.db` o terminada en `-wal`/`-shm`, con el mensaje `el reporte sólo lee copias VACUUM INTO, nunca sessions.db`.
* Periodo UTC por `resuelta_ms`: `--desde` inclusivo, `--hasta` exclusivo. Sin periodo, toda la historia.
* Agrega con la fórmula literal de `consumo_por_conversacion` (migración 0004): `monto_reservado` menos conciliación, sobre reservas conciliadas; las liberadas no cuentan.
* Salida: una línea `id_conversacion unidades` por conversación (ordenadas) más `TOTAL <celula> <desde|inicio> <hasta|fin> <unidades>`.

**Verificación:**

* Esperado: la línea `TOTAL` con la suma agregada. El número de líneas de conversación más la línea `TOTAL` es la salida completa.
* No aplica la verificación de DISC-0N.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito | — |
| 1 | Fallo de lectura de la copia (archivo dañado, ruta inexistente) | Verificar que la copia se produjo con `VACUUM INTO` y que la ruta es accesible |
| 2 | Uso incorrecto: faltó `--celula` o `--copia`, la copia se llama `sessions.db` o termina en `-wal`/`-shm`, o `--desde`/`--hasta` no son fechas `AAAA-MM-DD` válidas | Revisar diagnóstico en stderr |

---

## 8. `config render` — renderizar la configuración de una célula

**Cuándo:** generar el archivo de entorno que `deploy/cell.compose.yml` consumirá para una célula, combinando valores compartidos (defecto) y el *overlay* específico de la célula.

**Comando:**

```bash
hexcell-admin config render --defecto deploy/celula.defecto.env.ejemplo --superposicion deploy/celula.superposicion.env.ejemplo --salida celula.env
```

**Efecto:**

* Lee los dos archivos y produce la salida combinada: el archivo de defecto contiene los valores compartidos y el de superposición los valores específicos de la célula.
* Los archivos contienen **solo parámetros no secretos**; todo secreto sigue viajando por variables de entorno.
* Una clave desconocida o un valor inválido falla cerrado y no crea ni modifica la salida.
* No toca Docker ni el almacén del plano de control.

**Verificación:** código 0 y el archivo de salida generado con las claves combinadas. Para validar sin escribir: añadir `--simular` (reporta el número de claves sin crear la salida).

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito | — |
| 1 | Fallo al leer alguno de los archivos de entrada, o al escribir la salida | Revisar diagnóstico en stderr; verificar que las rutas de entrada existen y son legibles |
| 2 | Uso incorrecto: faltó `--defecto`, `--superposicion` o `--salida`, o alguna ruta es inválida | Revisar diagnóstico y texto de uso |

---

## Reejecución de un comando

Los cuatro comandos de ciclo de vida son reejecutables desde HEX-087 (tarea 15 del plan de A-6): un fallo parcial se reanuda con el **mismo comando**, sin pasos previos a mano (la única limpieza manual es la del volumen huérfano de `cell terminate`, sección 3). Reejecutar un comando sobre una célula que el almacén ya tiene en el estado objetivo no se rechaza: la reejecución **concilia contra el estado real de Docker**, completa sólo lo que falta y **no registra ninguna transición ni toca el almacén**. Cuando no hay nada que hacer, responde por stderr «sin cambios: la célula ya está <estado>» con código 0.

| Comando | Qué hace una reejecución |
| :--- | :--- |
| `cell pause` (fila `suspendida`) | Detiene el contenedor que haya quedado corriendo (ver sección 1). |
| `cell unpause` (fila `en ejecución`) | Arranca el contenedor que haya quedado detenido y confirma con `/health/ready` (ver sección 2). |
| `cell terminate` (fila `retirada`) | Elimina los restos que hayan quedado (contenedores y, si el núcleo sigue existiendo para resolverlo, el volumen), sin cierre de sesión. Sin restos: «sin cambios: la célula ya está retirada». |
| `cell terminate` (fila `en ejecución`, `suspendida` o `reemparejando`) | Completa el retiro y persiste `Retirada` (ver «Fallos parciales y limpieza manual» en la sección 3). |
| `cell rebind` (fila `reemparejando`) | Reanuda en el paso 8 de la secuencia (ver sección 4): primero consulta la sesión y, si ya está `activa`, omite el emparejamiento. |

La única excepción es la ausencia de recursos: `cell pause` y `cell unpause` sobre una célula a la que le falta el núcleo o el sidecar fallan con «célula no encontrada» (código 1), y `cell terminate` sin fila y sin contenedores falla igual.

## Arranque de la célula: barrido de reservas huérfanas de presupuesto

Desde HEX-092 (2026-09-30) cada arranque del núcleo —primer despliegue, `cell unpause` o cualquier otro reinicio del contenedor del núcleo— libera en una única transacción toda reserva de presupuesto en estado `'activa'` más antigua que el límite de drenaje (`HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS`, 20 s por omisión): el monto vuelve a `saldo.disponible` y queda un movimiento `'liberacion'`. El barrido corre tras abrir la persistencia y antes de que el HTTP acepte tráfico; ninguna acción del operador lo dispara ni lo evita.

Qué ver en el registro del núcleo (evento `reservas_huerfanas_liberadas`):

| Nivel | Detalle | Significado |
|---|---|---|
| `Info` | `recuento=N monto=M` | Se liberaron N reservas por M unidades: hubo una caída abrupta entre una reserva y su resolución. |
| `Info` | `sin cambios` | No había reservas huérfanas; es la línea esperada en un arranque normal. |
| `Aviso` | texto del error | El barrido falló y el arranque **continuó** (también sale por stderr «hexcell: no se pudo barrer las reservas huérfanas de presupuesto: …»). El saldo reservado sigue bloqueado hasta el próximo arranque; si se repite, revisar `sessions.db` con el procedimiento de la sección 7. |

Un `recuento` alto tras un `OOMKilled` no es un defecto del barrido: es la medida de lo que la caída dejó a medias. Regístrelo en el incidente junto con el procedimiento de la sección siguiente.

---

## Procedimiento: respuesta ante OOMKilled

Un contenedor muerto por `OOMKilled` es la señal de que el límite de memoria fijado en `deploy/cell.compose.yml` se queda corto bajo la carga real. Este procedimiento no cambia ese límite: documentar el incidente y escalar la decisión a quien corresponde.

### Detectar

1. `cell status` muestra `DISC-01` y el contenedor afectado aparece como `exited` (un contenedor muerto por `OOMKilled` sigue existiendo en Docker; `ausente` es el estado de `DISC-04`, no de esta situación).
2. Confirmar que la causa es OOM:
   ```bash
   docker inspect --format '{{.State.OOMKilled}}' <contenedor>
   ```
3. Registro del contenedor:
   ```bash
   docker logs --tail 50 <contenedor>
   ```

### Contener

1. Rearrancar **solo** el contenedor afectado (`docker start <contenedor>`). No usar `cell unpause`: arrancaría ambos.
2. Verificar con `cell status --id <celula_id>` — esperado: `salud: listo` (exige sesión activa).

### Registrar

Fecha absoluta, contenedor afectado y límite de memoria vigente. `deploy/cell.compose.yml` declara `mem_limit` como una referencia de variable (`${HEXCELL_NUCLEO_LIMITE_MEMORIA}` en la línea 106 para el núcleo, `${HEXCELL_SIDECAR_LIMITE_MEMORIA}` en la línea 174 para el sidecar), no un valor literal: ese archivo por sí solo no dice cuánta memoria tiene el contenedor. El valor efectivo es el que sustituye el archivo de entorno de la célula (referente `deploy/celula.env.ejemplo`) o, para el contenedor ya en ejecución, el que reporta:

```bash
docker inspect --format '{{.HostConfig.Memory}}' <contenedor>
```

No usar `docker stats` ni la memoria del anfitrión como fuente del límite.

### Escalar

Si el `OOMKilled` se repite en una misma célula en menos de 24 horas, la decisión de **revisar el límite** corresponde a la **tarea 6** y a `docs/STATUS.md`; los valores vigentes son provisionales hasta la medición de la tarea 16. **No se cambia el límite desde este runbook**; se escala con el registro del paso anterior.

---

## Referencias

* Tarea 21 del plan de la etapa A-6 (`docs/plan/fase-a-6-empaquetado-cli.md`) — esta tarea, que este runbook cierra.
* HEX-085 / tarea 13 del plan de A-6 (`docs/plan/fase-a-6-empaquetado-cli.md`) — `cell rebind` y su mecanismo de reanudación.
* HEX-087 / tarea 15 del plan de A-6 (`docs/plan/fase-a-6-empaquetado-cli.md`) — reejecución idempotente de `cell pause`, `cell unpause`, `cell terminate` y `cell rebind`, retiro parcial, cierre de sesión a mejor esfuerzo en `terminate` y recuperación de `ya_emparejada`; README.md, sección «9. Reejecución».
* `crates/hexcell-admin/src/comandos.rs` — servicio de aplicación, códigos de discrepancia DISC-01 a DISC-05, lógica de `ejecutar_estado` y `ejecutar_reemparejamiento`, y las reejecuciones `reejecutar_pausa`, `reejecutar_reanudacion` y `reejecutar_retiro`.
* `crates/hexcell-admin/src/ciclo_de_vida.rs` — secuencias sobre Docker: `retirar`, `completar_retiro`, `reconciliar_pausa` y `reconciliar_reanudacion`.
* `crates/hexcell-admin/src/codigo_de_salida.rs` — contrato de códigos de salida (0 éxito, 1 fallo, 2 uso incorrecto, 3 reservado).
* `deploy/cell.compose.yml` — plantilla de composición, límites de recursos y `stop_grace_period`.
* `docs/plan/fase-a-7-pilotos.md` — «Runbook de baneo» (tarea 5), que es donde se decide **si procede** sustituir el número; no existe todavía.
* `docs/runbook-restauracion-de-celula.md` — runbook de restauración desde respaldo.
* `docs/runbook-vigilancia-externa.md` — runbook del dead-man's switch.
