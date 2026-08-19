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
* **Despliegue escalonado en cartera (diferido a etapa A-6):** el despliegue de una versión candidata no se aplica a toda la cartera simultáneamente. Siguiendo `adr-0015` (Capa 3, canary de biblioteca), la actualización se ejecuta primero sobre una célula centinela con número propio durante 72 horas antes de escalonar progresivamente al resto de las células. La automatización de este escalonado pertenece a la etapa A-6.

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
