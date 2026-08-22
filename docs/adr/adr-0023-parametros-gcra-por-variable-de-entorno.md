# adr-0023 — Parametrización de límites de admisión GCRA por variables de entorno y justificación de parámetros por omisión

* **Estado:** Vigente (2026-08-22).
* **Etapa que lo produce:** A-4 (tarea 3 del plan de la etapa A-4: `docs/plan/fase-a-4-admision-presupuesto.md`).
* **Relación con otros ADR:** Cita el requerimiento **FR-08** de `docs/PRD.md`. No **SUPERSEDE** ni reescribe la entrada provisional de `adr-0004-gcra-y-parametros.md` (cuya fila permanece "Tomada en el PRD, por formalizar" para el ADR de arquitectura GCRA más amplio); se acota a la parametrización por entorno y a la justificación de los valores por omisión.

## Contexto

El control de admisión GCRA (módulo `crates/hexcell-core/src/admision.rs`) protege a la célula limitando la tasa de eventos por clave de límite (el identificador de conversación normalizado). Hasta esta tarea, los parámetros de tasa sostenida (0.5 req/s) y tolerancia a ráfaga (3) estaban prefijados como constantes en código.

Para cumplir con el requerimiento **FR-08** y la tarea 3 de la etapa A-4 (`docs/plan/fase-a-4-admision-presupuesto.md`), es necesario permitir que dichos parámetros puedan ser configurados al arrancar el proceso mediante variables de entorno, siguiendo las convenciones de nomenclatura en español de `crates/hexcell/src/configuracion.rs`. Si alguna variable contiene un valor inválido o no parseable, el proceso debe fallar en arranque (*fail-closed*) imprimiendo un mensaje en español que nombre la variable infractora.

Asimismo, la clave de límite de admisión se mantiene inalterada como el identificador de conversación normalizado (`RegistroDeAdmision` indexado por `conversacion.como_str()`), evaluándose de forma atómica antes de la deduplicación, la carga de contexto conversacional o la inferencia LLM.

## Decisión

**1. Parametrización por variables de entorno con fallback e invariante de fallo cerrado:**
Se definen dos nuevas variables de entorno opcionales:
* `HEXCELL_ADMISION_TASA_SOSTENIDA_POR_SEGUNDO` (tipo `f64`): tasa sostenida en peticiones por segundo.
* `HEXCELL_ADMISION_TOLERANCIA_RAFAGA` (tipo `u32`): número de peticiones permitidas en ráfaga.

En `Configuracion::desde_entorno()`, si las variables no están presentes se adopta el valor de `ConfiguracionGcra::default()`. Si están presentes pero contienen valores no parseables o semánticamente inválidos (tasa ≤ 0 o no finita), la función retorna un `ErrorDeConfiguracion::ValorInvalido` nombrando la variable específica y su formato esperado en español, provocando la terminación inmediata del proceso antes de vincular red o disco.

**2. Justificación de los valores por omisión (0.5 req/s, ráfaga 3):**
Se conservan los valores predeterminados (0.5 peticiones/segundo, 1 cada 2 segundos; ráfaga de 3 peticiones extra) respaldados por la evidencia de prueba `ac_3_perfil_conversacional_realista_cero_falsos_positivos` en `crates/hexcell-core/src/admision.rs` y las pruebas de integración en `crates/hexcell/tests/admision.rs`. Bajo un perfil de interacción conversacional humana realista (mensajes iniciales rápidos seguidos de pausas de lectura e inferencia), estos valores garantizan cero falsos positivos sin descartar tráfico legítimo.

**3. Inyección en el motor de mensajería sin mutar la firma existente:**
Se añade el método builder `Motor::con_configuracion_gcra(mut self, configuracion: ConfiguracionGcra) -> Self` en `crates/hexcell/src/motor.rs`. La firma de `Motor::nuevo` permanece intacta (utilizando `ConfiguracionGcra::default()`), evitando modificar las ~20 llamadas existentes en la suite de pruebas del proyecto. Las instancias de producción en `main.rs` encadenan `.con_configuracion_gcra(configuracion.configuracion_gcra.clone())`.

## Alternativas consideradas y descartadas

### (a) Modificar la firma de `Motor::nuevo` para exigir `ConfiguracionGcra` — **DESCARTADA**
Modificar `Motor::nuevo` obligaría a actualizar decenas de sitios de llamada en tests que no requieren personalizar la admisión. El patrón builder mantendrá la compatibilidad exacta con la suite existente (LES-039).

### (b) Reconfiguración dinámica en caliente sin reinicio — **DESCARTADA**
La reconfiguración dinámica añade complejidad de sincronización. Las variables de entorno leídas al arranque cumplen con el modelo de despliegue contenerizado previsto para la célula.

## Consecuencias

* Los parámetros de admisión GCRA son plenamente configurables mediante variables de entorno en despliegues.
* En ausencia de variables de entorno, se preservan de forma determinista los valores probados por omisión (0.5 req/s, ráfaga 3).
* Todo valor inválido en el entorno bloquea el arranque con un mensaje claro en español especificando la variable errónea.
* Ningún test previo fuera de la lista del contrato fue alterado.

## Referencias

* `docs/PRD.md` (requerimiento FR-08).
* `docs/plan/fase-a-4-admision-presupuesto.md` (Etapa A-4, tarea 3).
* `crates/hexcell-core/src/admision.rs` (`ConfiguracionGcra`, `ac_3`).
* `crates/hexcell/src/configuracion.rs` (`HEXCELL_ADMISION_*`).
* `crates/hexcell/src/motor.rs` (`con_configuracion_gcra`).
