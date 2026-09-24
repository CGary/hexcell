# Runbook: operación de células con hexcell-admin

* **Fecha de esta versión:** 2026-09-24.
* **Tarea que lo redacta:** HEX-088 (tarea 21 de `docs/plan/fase-a-6-empaquetado-cli.md`).
* **Alcance de esta versión:** procedimiento de operación de los subcomandos `cell` y `config render` de `hexcell-admin`, más el procedimiento de respuesta ante `OOMKilled`. La reejecución idempotente de un comando está diferida a la tarea 15, que es la dueña de ese mecanismo.

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
| Baja definitiva de un cliente | `hexcell-admin cell terminate --id <celula_id> --confirmar` (si la célula está pausada —p. ej. la ruta impago → pausa → baja definitiva— ejecutar antes `cell unpause`: `terminate` exige el núcleo `running`) |
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
| 1 | Fallo de ejecución: el almacén no abrió, la transición no se validó, o Docker no respondió | Revisar diagnóstico en stderr; verificar que `HEXCELL_ADMIN_ALMACEN` apunta a un directorio existente y que el socket de Docker es accesible |
| 2 | Uso incorrecto: faltó `--id` o se aportó una opción no admitida | Revisar diagnóstico y texto de uso en stderr |

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

---

## 3. `cell terminate` — eliminar definitivamente una célula

**Cuándo:** baja definitiva de un cliente o destrucción acordada de la célula. Operación **destructiva**: cierra la sesión de canal, destruye ambos contenedores y elimina el volumen de datos físicamente, incluidas las credenciales.

**Precondición: la célula debe estar en ejecución.** `cell terminate` inspecciona el núcleo y exige que esté `running`; si la célula está pausada (`cell pause` previo, p. ej. la ruta impago → pausa → baja definitiva), el comando falla con código 1 y el diagnóstico «la célula está pausada: ejecute cell unpause antes de cell terminate» (`ciclo_de_vida.rs:176,531-533`), sin tocar nada. Ejecutar primero `hexcell-admin cell unpause --id <celula_id>` y luego `cell terminate`.

**Comando:**

```bash
hexcell-admin cell terminate --id <celula_id> --confirmar
```

**Efecto:**

1. Inspecciona núcleo y sidecar.
2. Cierra la sesión whatsmeow vía contenedor hermano (`POST /admin/sesion/cierre`), desvinculando el dispositivo.
3. Detiene sidecar y núcleo con el margen de gracia de `deploy/cell.compose.yml`.
4. Elimina ambos contenedores y el volumen (nombre leído de `docker inspect`, no del `--id`).
5. Persiste `Retirada` con motivo `sesion_cerrada` —solo tras éxito—.
6. Emite por stdout: `sesión cerrada`, `contenedores eliminados`, `volumen <tamaño> eliminado`.

**Verificación:**

```bash
hexcell-admin cell status --id <celula_id>
```

* Esperado: `estado: retirada`, `docker nucleo: ausente`, `docker sidecar: ausente`.
* **`DISC-04` (el almacén tiene fila pero los contenedores no existen en Docker) es el resultado esperado y correcto, no una falla.** `cell status` lo sigue reportando y sale con código 1 por construcción: la fila persiste con `estado: retirada` mientras que los contenedores fueron eliminados, y esa combinación es exactamente la que dispara `DISC-04` (`comandos.rs:720-723`). Confirmar que la fila conserva `retirada` es la verificación real; el código de salida 1 de `cell status` en este caso no indica un problema.

**Fallos comunes por código de salida:**

| Código | Significado | Remediación |
| :--- | :--- | :--- |
| 0 | Éxito | — |
| 1 | La célula está pausada, o fallo en algún paso de la secuencia destructiva (cierre de sesión, detención, eliminación) | Si el diagnóstico dice «la célula está pausada», ejecutar `cell unpause` primero. Para los demás fallos, revisar diagnóstico en stderr; la secuencia se detiene en el primer paso que falla y no continúa — ver «Reejecución de un comando» más abajo |
| 2 | Uso incorrecto: faltó `--id` o `--confirmar` | Revisar diagnóstico y texto de uso |

> **Importante:** sin `--confirmar` el comando devuelve `UsoIncorrecto` (código 2) y no toca nada. Esta exigencia es la misma que aplica a `cell rebind`.

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

1. Abre el almacén y lee la fila: sin fila o `EnEjecucion` → secuencia completa; `Reemparejando` → reanuda en el paso 7; `Suspendida` → falla con «ejecute cell unpause antes de cell rebind».
2. Inspecciona el núcleo para resolver los datos de la célula (red, puerto, volumen).
3. Pausa el envío de la célula (la célula **no** intenta responder sin sesión).
4. Cierre de sesión a mejor esfuerzo —un fallo se escribe por diagnóstico y la secuencia continúa, porque tras un baneo el cierre suele fallar—.
5. Persiste `Reemparejando` con el motivo aportado.
6. Descarta el `sqlstore` del sidecar y rearranca el sidecar con la pausa reaplicada.
7. Solicita emparejamiento por QR o código de vinculación (omisión: `qr`). La cadena se emite por stdout con la nota de que el renderizado gráfico no está integrado; usar un renderizador QR externo.
8. Sondea la sesión hasta `activa`.
9. Reanuda el envío.
10. Persiste `EnEjecucion` con una fila en `sustituciones` (id de célula, motivo, fecha absoluta en ms).

**Conservado:** `sessions.db`, `knowledge_live.db`, el almacén de identidad del adaptador (identidad de conversación y lista STOP), `identidad.db` y `outbox.db` del sidecar. **Descartado:** sólo `sqlstore.db` (con `-wal` y `-shm`). **Reanudación:** una célula en `Reemparejando` se reanuda con el **mismo** `cell rebind`, que salta al paso 7.

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
| 1 | Fallo en algún paso de la secuencia | Revisar diagnóstico en stderr. Si quedó en `Reemparejando`, reejecutar el mismo comando. Limitación conocida (tarea 13, nota 2026-09-22): `ya_emparejada` termina en `Fallo` sin recuperación — pendiente de la tarea 15 |
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

El procedimiento definitivo de reejecución idempotente —detección del punto en que quedó la secuencia y recuperación automática para todos los comandos— es alcance de la **tarea 15** del plan de la etapa A-6, en progreso en paralelo. Hoy sólo `cell rebind` tiene una reanudación real: una célula que quedó en `Reemparejando` retoma en el paso 7 con el **mismo comando** (ver sección 4). `cell terminate` **no** tiene ese mecanismo: un fallo parcial no se reanuda, y la recuperación de esa secuencia es alcance de la tarea 15.

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
* HEX-085 / tarea 13 del plan de A-6 (`docs/plan/fase-a-6-empaquetado-cli.md`) — `cell rebind` y su mecanismo de reanudación; nota 2026-09-22 sobre la limitación de recuperación tras `ya_emparejada`.
* `crates/hexcell-admin/src/comandos.rs` — servicio de aplicación, códigos de discrepancia DISC-01 a DISC-05, lógica de `ejecutar_estado` y `ejecutar_reemparejamiento`.
* `crates/hexcell-admin/src/codigo_de_salida.rs` — contrato de códigos de salida (0 éxito, 1 fallo, 2 uso incorrecto, 3 reservado).
* `deploy/cell.compose.yml` — plantilla de composición, límites de recursos y `stop_grace_period`.
* `docs/plan/fase-a-7-pilotos.md` — «Runbook de baneo» (tarea 5), que es donde se decide **si procede** sustituir el número; no existe todavía.
* `docs/runbook-restauracion-de-celula.md` — runbook de restauración desde respaldo.
* `docs/runbook-vigilancia-externa.md` — runbook del dead-man's switch.
