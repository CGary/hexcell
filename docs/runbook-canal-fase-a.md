# Runbook: re-emparejamiento de célula mediante PairPhone()

* **Fecha de esta versión:** 2026-08-12.
* **Etapa que lo redacta:** A-3 (tarea 16 de `docs/plan/fase-a-3-adaptador-whatsmeow.md`).
* **Alcance de esta versión:** procedimiento operativo paso a paso para el re-emparejamiento de una célula con el canal propio (`whatsmeow`) utilizando el código de ocho caracteres (`PairPhone()`). Este documento cubre el re-emparejamiento como segundo nivel de defensa ante pérdidas de sesión; la restauración ordinaria del `sqlstore` se detalla en `docs/runbook-restauracion-de-celula.md`, las roturas de protocolo de whatsmeow se cubren en `docs/runbook-canal-whatsmeow.md`, y la respuesta ante baneos permanentes/temporales (con `cell rebind` y SIM de reserva) es alcance de la etapa A-7.

---

## 1. Disparadores del re-emparejamiento

El procedimiento de re-emparejamiento por código telefónico se activa exclusivamente en dos situaciones:
1. **Fallo o desfase del respaldo:** cuando el respaldo del `sqlstore` es insuficiente, está corrupto o llega obsoleto con credenciales de Signal invalidadas que impiden la reconexión automática.
2. **Bifurcación por desvinculación (Rama A):** cuando la Rama A (`device_removed`) del procedimiento de restauración en `docs/runbook-restauracion-de-celula.md` ordena explícitamente no restaurar el `sqlstore` obsoleto y en su lugar generar una nueva vinculación mediante `PairPhone()`.

---

## 2. El re-emparejamiento como defensa de primera clase

El re-emparejamiento no es un último recurso improvisado, sino un procedimiento de recuperación de primera clase diseñado como la segunda capa de defensa del canal propio. 

Presenta una ventaja operativa fundamental: **no requiere tener el teléfono físico del piloto en la mano del operador ni realizar desplazamientos**. Dado que el código se introduce directamente en el dispositivo del cliente, el piloto puede realizar la vinculación de forma remota en su propio teléfono siguiendo las indicaciones del operador, cumpliendo con lo estipulado en la tarea 16 del plan A-3.

---

## 3. Procedimiento del operador

El operador solicita el código de vinculación utilizando la superficie existente del sidecar:

1. **Invocación interna:** el sidecar ejecuta la función `SolicitarCodigoDeVinculacion` (en `sidecar/internal/canal/emparejamiento.go`), la cual envuelve la API `PairPhone()` de `whatsmeow`.
2. **Higiene de datos:** 
   * La función obtiene el número de teléfono directamente desde la configuración de la célula. Nunca se transmite como un campo del protocolo IPC para respetar la guardia de identificadores de transporte de `mensajes_test.go`.
   * El código de vinculación generado nunca se escribe en el registro estructurado de logs a ningún nivel, asegurando la privacidad conforme a `adr-0019`.
3. **Brecha de interfaz de operador (Pendiente):**
   * *Advertencia:* Actualmente `SolicitarCodigoDeVinculacion` no posee una superficie expuesta directamente al operador (carece de subcomando CLI o de mensaje IPC cableado desde el núcleo Rust), siendo ejercitada únicamente por las pruebas del paquete Go. El operador debe documentar este vacío como una tarea pendiente en `docs/STATUS.md` y no inventar rutas inexistentes. Una vez desarrollada la CLI de administración en la etapa A-6, este paso se ejecutará con el comando correspondiente.

---

## 4. Pasos en el teléfono del piloto

Una vez que el operador obtiene el código de vinculación de ocho caracteres, se lo transmite al piloto (por ejemplo, vía llamada telefónica o canal alternativo). El piloto debe realizar lo siguiente en su propio teléfono:

1. Abrir **WhatsApp**.
2. Acceder al menú de configuración e ir a **Dispositivos vinculados**.
3. Seleccionar la opción **Vincular un dispositivo**.
4. Seleccionar la opción **Vincular con el número de teléfono** en la parte inferior de la pantalla de escaneo QR.
5. Introducir el código de ocho caracteres provisto por el operador.

---

## 5. Comprobación de salud y supervivencia de la identidad

### Criterio de aceptación de salud
El re-emparejamiento se da por completado con éxito solo cuando se verifica que la célula está sana:
1. El estado de la sesión reportado por el sidecar cambia a activo.
2. El bot responde correctamente a un mensaje entrante real en un chat de prueba.

### Supervivencia de la identidad
De acuerdo con `adr-0010`, **el mapeo de identidad (JID a ID interno) y la lista de exclusión (STOP) sobreviven al re-emparejamiento**. Dado que este almacén (`adapter_identity.db`) vive de forma independiente al `sqlstore` del sidecar, no se borra al cambiar de dispositivo. Los chats de los clientes continuarán cayendo en sus mismos hilos históricos sin interrupciones, respetando la sección "Lo que SÍ sobrevive a esta rama" del runbook de restauración.

---

## 6. Requisito de ensayo y aplazamiento

Un procedimiento de recuperación que nunca se ha ejecutado no es un procedimiento, sino una suposición. Por lo tanto, se establece el siguiente requisito de control:

* **Ensayo obligatorio:** el procedimiento de re-emparejamiento debe ser ensayado y cronometrado al menos una vez con `piloto-01` **antes** de proceder al onboarding de `piloto-02`.
* **Aplazamiento explícito:** el ensayo queda explícitamente aplazado y no se inventan fechas ni números de cliente para simularlo. Requiere una célula emparejada real y acceso al piloto, lo cual depende de la resolución de la tarea 15 (número de laboratorio) y del alta de `piloto-01`.

---

## Referencias

* `docs/runbook-restauracion-de-celula.md` (procedimiento de restauración y bifurcación de ramas).
* `docs/runbook-canal-whatsmeow.md` (política de actualización y rotura de whatsmeow).
* `docs/adr/adr-0020-respaldo-y-restauracion-por-celula.md` (decisión de respaldo dual).
* `docs/adr/adr-0010-puerto-de-canal.md` (abstracción del puerto de canal y persistencia de identidad).
* `docs/plan/fase-a-3-adaptador-whatsmeow.md` (planificación de la etapa A-3).
* `docs/PRD.md` (requisito de recuperación y control).
* `docs/STATUS.md` (estado de tareas pendientes y decisiones de negocio pendientes).
* `docs/bitacora-de-descartes.md` (descartes de comportamiento y proxies).
