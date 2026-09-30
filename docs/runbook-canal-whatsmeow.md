# Runbook: procedimiento ante rotura de protocolo y política de actualización de whatsmeow

* **Fecha de esta versión:** 2026-08-12.
* **Etapa que lo redacta:** A-3 (tarea 17 de `docs/plan/fase-a-3-adaptador-whatsmeow.md`).
* **Alcance de esta versión:** procedimiento operativo paso a paso ante roturas de protocolo de WhatsApp Web en el canal propio (`whatsmeow`), política de fijación de dependencia por commit y mecanismo de ventana de actualización. Este documento cubre exclusivamente roturas de protocolo; el re-emparejamiento operativo con `PairPhone()` es alcance de la tarea 16 (`docs/runbook-canal-fase-a.md`), el respaldo del `sqlstore` por IPC es alcance de la tarea 18, y la respuesta ante baneos de cuenta (sustitución de número con `cell rebind` y gestión de SIM de reserva) es alcance de la etapa A-7. La convivencia permanente con el canal oficial (Fase B) sigue lo fijado en `adr-0014`.

---

## 1. Política de fijación de dependencia por commit

La biblioteca `whatsmeow` implementa el protocolo no oficial de WhatsApp Web. Para garantizar la reproducibilidad de las imágenes de producción y evitar cambios no probados, la dependencia se fija **por commit** (`[precautorio]`, `adr-0015` ítem 14).

* **Commit fijado actualmente:** commit `e9a033b24933` (pseudotasa de versión `v0.0.0-20260722203353-e9a033b24933` en `sidecar/go.mod`).
* **Regla de fijación:** nunca se emplean versiones flotantes, rangos ni la etiqueta `latest`. Cualquier actualización de la biblioteca se efectúa de forma explícita mediante un commit concreto y validado.
* **Aislamiento:** la dependencia de whatsmeow vive exclusivamente en el módulo Go del sidecar (`sidecar/go.mod`). Ni el núcleo Rust ni el protocolo IPC conocen la biblioteca ni cambian cuando el commit se actualiza.

---

## 2. Mecanismo de la ventana de actualización

Correr una versión atrasada de la biblioteca introduce un doble riesgo (`adr-0015` ítem 14 `[precautorio]`):
1. **Desconexión por protocolo:** WhatsApp bloquea clientes con versiones obsoletas mediante el error recurrente `Client outdated (405)`.
2. **Señal anómala:** declarar una versión de cliente Web atípica o desfasada frente a los clientes oficiales activos constituye una señal de automatización detectable por los sistemas de Meta.

### Mecanismo de control

* **Revisión técnica:** el equipo revisa periódicamente los cambios aguas arriba en el repositorio de `tulir/whatsmeow` (nuevos commits, avisos de roturas y actualizaciones de versión de cliente de WhatsApp Web).
* **Puerta de paso (gate):** la incorporación de un nuevo commit requiere que la batería de pruebas automatizadas del sidecar (`go test ./...`) y las pruebas de integración del workspace pasen en verde antes de considerar la versión como candidata.
* **Cadencia de actualización:** la frecuencia numérica regular con la que se evalúan y aplican actualizaciones ordinarias queda declarada **a calibrar** como decisión de negocio pendiente en `docs/STATUS.md`.
* **Despliegue escalonado en cartera:** el procedimiento operativo completo —canario de 72 horas en la célula centinela, escalonado por lotes con parada ante incidencias y la prohibición de actualizar toda la cartera el mismo día— se describe en la sección 7, con su respaldo mecánico en el guardia de CI `deploy/verificar_despliegue_escalonado.sh` (trabajo `guardas-despliegue`).

---

## 3. Procedimiento ante rotura de protocolo

Cuando WhatsApp modifica el protocolo Web o eleva la versión mínima admitida, el patrón de fallo recurrente es `Client outdated (405)` (issues #415 y #1031 de `tulir/whatsmeow`).

> **Compromiso de recuperación:**
> whatsmeow es un proyecto mantenido por la comunidad con **bus factor 1** (prácticamente la totalidad de sus commits provienen de un único mantenedor voluntario). **No se puede comprometer ningún tiempo de recuperación que dependa de un tercero voluntario.** Esta limitación es una propiedad estructural del canal propio no oficial per `adr-0015`, no un defecto corregible del software. Con los clientes se pacta contractualmente la posibilidad de períodos de inoperatividad sin garantía de disponibilidad.

### Pasos operativos ante rotura

1. **Comprobar el estado del proyecto aguas arriba (upstream):**
   * Consultar el repositorio `tulir/whatsmeow` (issues recientes, pull requests y commits en la rama principal).
   * Identificar si la rotura ya fue reportada y si existe un commit disponible que actualice la versión de cliente o resuelva la incompatibilidad del protocolo.
2. **Actualizar el commit pinneado en `sidecar/go.mod`:**
   * En el directorio `sidecar/`, actualizar la dependencia apuntando al commit verificado:
     ```bash
     cd sidecar && go get go.mau.fi/whatsmeow@<nuevo_commit_hash> && go mod tidy
     ```
   * Verificar que `sidecar/go.mod` refleja el nuevo commit en su pseudotasa y que la compilación local (`go build ./...`) no presenta errores de tipos o API.
3. **Reconstruir la imagen del contenedor del sidecar:**
   * Ejecutar la suite de pruebas del sidecar:
     ```bash
     cd sidecar && go test ./...
     ```
   * Reconstruir la imagen Docker del sidecar para el entorno de despliegue.
4. **Redesplegar el sidecar en las células:**
   * Reiniciar y redesplegar los contenedores del sidecar con la nueva imagen.
   * Verificar en los registros estructurados que el websocket saliente reconecta satisfactoriamente, que no se emite error `405` y que el estado de sesión reportado transiciona a activo.

> **Aviso:** Nunca redesplegar todos los sidecars a la vez: la actualización pasa primero por el
> centinela y luego por lotes (criterio de la tarea 19 de A-6). Este procedimiento describe una sola
> célula.

---

## 4. Criterio de aceptación de la recuperación

Una recuperación ante rotura de protocolo **no se da por buena porque el contenedor arranque**. El criterio de éxito estricto exige que la célula:

1. Establezca la conexión websocket hacia WhatsApp sin errores de protocolo (`Client outdated (405)` u otros).
2. Reporte estado de sesión activo a través del IPC hacia el núcleo Rust (`GET /health/ready` responde listo).
3. Consuma un evento entrante real y emita la respuesta correspondiente por el canal.

---

## 5. Taxonomía de desconexión validada en laboratorio

Durante la sesión de laboratorio del **2026-08-18**, se validaron empíricamente las siguientes clasificaciones y rutas de recuperación ante desconexiones del canal propio:

* **Corte de transporte (`desconexion_de_transporte`):** Provocado por cortes de red. El sidecar inicia la reconexión autónoma aplicando la disciplina de retroceso (backoff) exponencial configurada hasta restablecer la conexión.
* **Desvinculación forzada (`desvinculada_dispositivo_removido`, código `401`):** Provocado al desvincular el dispositivo desde el cliente oficial. Se abortan inmediatamente los reintentos, whatsmeow elimina la sesión local y el restablecimiento requiere una intervención humana para re-emparejar (ver [runbook-canal-fase-a.md](runbook-canal-fase-a.md)).
* **Entorno del laboratorio:** Los ensayos se operaron sobre procesos directos mediante los scripts en `scripts/laboratorio/`, quedando pendiente el empaquetado del ciclo de vida de contenedores para la etapa A-6.

> [!NOTE]
> **Rutas no ejercitadas (pendientes):** El flujo de emparejamiento por código con `PairPhone()` contra un canal real de WhatsApp y el ensayo de restauración extrema a extrema (tarea 18) no se ejercitaron y permanecen explícitamente pendientes.

---

## 6. Métricas nativas y apagado ordenado (2026-09-13)

* **Métricas del canal propio**: el sidecar emite periódicamente una línea de registro estructurado con el
  ratio de acuses por contacto, las reconexiones por hora y la ventana de silencio entrante
  (`sidecar/internal/metricas`, HEX-072-b). No hay endpoint HTTP ni tipo IPC para ellas (`adr-0024`, D-45).
* **Apagado ordenado**: `docker stop` entrega `SIGTERM` a ambos contenedores con `stop_grace_period` de 30 s
  (`deploy/cell.compose.yml`); el núcleo drena en 20 s como máximo (`crates/hexcell/src/apagado.rs`).
  Guardas: `deploy/verificar_senales.sh` (en CI) y `deploy/verificar_apagado_ordenado.sh` (manual, con
  contenedores reales; HEX-075).

## 7. Despliegue escalonado en cartera

La actualización del commit fijado de `whatsmeow` (sección 1) no se aplica nunca a toda la cartera
a la vez. El criterio de aceptación de la tarea 19 de la etapa A-6
(`docs/plan/fase-a-6-empaquetado-cli.md`) exige que no exista ninguna vía —ni la CI, ni la CLI— que
actualice toda la cartera en un solo paso, y `adr-0015` (Capa 3, canary de biblioteca) fija la forma:
primero la célula centinela durante 72 horas y después, por lotes, el resto.

### Canario de 72 horas en la célula centinela

La **célula centinela** es una célula propia de HexCell, con número propio y sin ningún cliente
encima, que corre la versión candidata de `whatsmeow` durante **72 horas** antes de que la
actualización toque a cualquier célula de cliente. Su alta está pendiente de la decisión «Número
propio de WhatsApp para el centinela» de `docs/STATUS.md` (pendiente de pasar a `Definido`); hasta
entonces, la corrida real de 72 horas no puede ejecutarse. La centinela es además el sitio donde se
ensayan medidas cuya eficacia no está probada —el experimento con Meta Verified, entre ellas— porque
es el único número cuyo baneo no le cuesta el negocio a nadie; ninguna de esas medidas se documenta
como probada mientras no lo esté.

### Escalonado por lotes sobre la cartera

Superadas las 72 horas sin incidencias, la actualización avanza por lotes.

**Una vez por candidato, antes de la corrida de la centinela** (no dentro de cada lote): actualizar
el commit fijado en `sidecar/go.mod`, ejecutar `go test ./...` y construir la imagen del sidecar
(secciones 1 y 3). La misma imagen candidata que corrió 72 horas en la centinela es la que recibe
cada lote; no se reconstruye entre lotes.

**Por cada lote, célula por célula:**

1. **Pausar la célula:** `hexcell-admin cell pause --id <cell_id>`.
2. **Recrear el contenedor del sidecar de esa célula con la imagen candidata** (paso 4 de la sección
   3, una célula a la vez): con `docker compose` sobre el proyecto de **esa única célula**,
   apuntando `HEXCELL_IMAGEN_SIDECAR` de su archivo de entorno a la imagen candidata y ejecutando
   `docker compose -f deploy/cell.compose.yml --env-file <entorno_de_la_celula> create
   --force-recreate sidecar`. Nunca se ejecuta sobre las demás células ni sobre la cartera entera.
3. **Reanudar la célula:** `hexcell-admin cell unpause --id <cell_id>`. Este comando solo arranca
   los contenedores existentes de la célula: **por sí solo no cambia la imagen**; la imagen nueva
   llega únicamente por la recreación del paso 2.
4. **Comprobar** con `hexcell-admin cell status --id <cell_id>` que la célula queda en ejecución y
   que el websocket reconecta sin `Client outdated (405)`.
5. **Registrar el lote** en el registro de despliegue (tabla más abajo).

**Prohibición operativa literal: nunca actualizar todas las células el mismo día.**

**Condiciones de parada para el lote siguiente:** no se avanza al siguiente lote si el lote anterior
presenta baneos, desconexiones anómalas o `Client outdated (405)`: se detiene el escalonado y se
vuelve al procedimiento ante rotura de la sección 3.

**Tamaño de lote:** el tamaño de lote es un **parámetro** del procedimiento, no una constante. Su
valor por omisión es **una célula por lote**, el paso más pequeño que no es la cartera entera:
`adr-0015` fija el riesgo de que una versión candidata defectuosa se lleve por delante a todos los
clientes a la vez, y el techo de cartera es decisión de negocio pendiente, así que ningún documento
fija un número de células por lote. Mientras no se decida otro valor, un lote es una célula.

### Registro de despliegue

Cada lote se registra con **fecha** (absoluta), **células** del lote, **versión o commit** de
`whatsmeow` desplegado y **resultado** (OK o la condición de parada que detuvo el escalonado):

| Fecha | Células | Versión/commit de whatsmeow | Resultado |
| :--- | :--- | :--- | :--- |

### Comandos y respaldo mecánico

`hexcell-admin` **no tiene ningún comando de actualización ni de despliegue**: no existen `update`,
`deploy`, `actualizar` ni `desplegar`, y no se añadirá ninguno. Los únicos comandos que intervienen
en este procedimiento son los que ya existen: `cell pause`, `cell unpause` y `cell status`. El
guardia estático `deploy/verificar_despliegue_escalonado.sh` —ejecutado en la CI como trabajo
`guardas-despliegue`— comprueba que ningún trabajo distinto de `imagenes` ejecuta `docker compose
up`/`pull`/`restart` ni un script de despliegue, y que la CLI no expone subcomandos de actualización:
es el respaldo mecánico de la prohibición anterior.

## Referencias

* `docs/adr/adr-0015-politica-de-convivencia-con-el-baneo.md` (ítem 14 `[precautorio]`, Capa 3 canary de biblioteca).
* `docs/adr/adr-0014-canal-propio-permanente.md` (canal propio permanente y coexistencia con Fase B).
* `docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md` (arquitectura de sidecar e IPC).
* `docs/adr/adr-0009-whatsmeow-adaptador-fase-a.md` (elección de whatsmeow).
* `docs/plan/fase-a-3-adaptador-whatsmeow.md` (tarea 17).
* `docs/plan/fase-a-6-empaquetado-cli.md` (célula centinela y despliegue escalonado).
* `docs/STATUS.md` (registro de estado y decisiones de negocio pendientes).
* `docs/PRD.md` (FR-01, FR-12, NFR-01, NFR-05).
* `docs/bitacora-de-descartes.md` (D-07, D-08).
* `deploy/verificar_despliegue_escalonado.sh` (guardia estático del despliegue escalonado y su autoprueba de mutación; HEX-089).
