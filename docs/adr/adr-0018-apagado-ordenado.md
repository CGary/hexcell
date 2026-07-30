# ADR-0018 — Apagado ordenado del binario de la célula

* **Estado:** Vigente desde el 2026-07-30.
* **Supersede a:** nada.
* **Etapa:** A-2 (HEX-007).
* **Requisitos tocados:** NFR-01 (presupuesto de memoria y de arranque/parada), plazo de gracia
  del PRD.

---

## Contexto

Hasta esta tarea, el binario `hexcell` no capturaba ninguna señal: un `SIGTERM` del orquestador
(o de un `docker stop`) terminaba el proceso con la acción por defecto del sistema operativo,
cortando cualquier evento en curso a la mitad, sin drenar la cola ya recibida y sin consolidar el
WAL de `sessions.db`. El PRD fija un plazo de gracia de treinta segundos entre la señal y el
`SIGKILL` forzoso del orquestador; esta tarea tiene que aprovechar ese plazo para terminar de
forma que ningún evento en vuelo se pierda.

## Decisión

1. **`Apagado::instalar` registra `SIGTERM` y `SIGINT`** con `tokio::signal::unix::signal`, nada
   más analizar la configuración y antes de abrir la persistencia o vincular cualquier puerto: una
   señal que llegara durante el arranque queda capturada en vez de matar el proceso con la acción
   por defecto. `SIGINT` se añade porque quien lanza el binario a mano desde una terminal merece la
   misma salida ordenada que el orquestador, y cuesta tres líneas más.
2. **La señal se transporta con `tokio::sync::watch`**, no con `tokio-util::CancellationToken`
   (D-18, más abajo): `watch` ya está en la característica `sync` que este crate ya declaraba, y
   expresa exactamente lo que hace falta, un valor compartido que cambia una vez y que cualquier
   receptor observa. `SenalDeApagado` no guarda su propio emisor: la tarea de fondo que arranca
   `instalar` lo posee y se queda aparcada para siempre (`std::future::pending`), así que el emisor
   vive tanto como el proceso sin que nada externo tenga que retenerlo, y los seis sitios de
   prueba existentes que construyen un `Motor` sin apagado en marcha usan `SenalDeApagado::nunca()`
   sin que un receptor de emisor ya destruido dispare un apagado inmediato no deseado.
3. **`Motor::ejecutar` corre un `tokio::select!` con `biased` sobre exactamente dos ramas: la
   señal y `receptor_eventos.recv()`.** El trabajo de cada evento se espera **dentro** del cuerpo de
   la rama de `recv`, nunca como una rama más del propio `select!`, así que el `select!` nunca
   puede estar sondeando mientras un evento está a medias: no hay forma de cancelarlo a la mitad,
   estructuralmente, no por promesa.
4. **Al recibir la señal, el motor cierra `receptor_eventos` con `close()`.** A partir de ese
   instante ningún emisor puede encolar nada más, pero `recv()` sigue entregando lo que ya
   estuviera en la cola hasta vaciarla — exactamente la semántica que el spec pide: dejar de
   aceptar trabajo nuevo sin abandonar el que ya llegó.
5. **El drenaje que sigue comprueba el límite temporal (`HEXCELL_LIMITE_DE_DRENAJE_SEGUNDOS`, diez
   segundos por defecto) entre eventos, nunca envolviendo el drenaje entero en un temporizador de
   expiración global.** Un temporizador de ese tipo cortaría el futuro en curso en cualquier punto
   en que estuviera, posiblemente entre el envío y la anotación en el historial: precisamente el
   corte a medias que esta tarea existe para impedir. Diez segundos y no treinta, porque el plazo
   de gracia total del PRD es de treinta segundos y el punto de control del WAL más el resto de la
   salida tienen que caber en lo que quede tras el drenaje.
6. **Tras el drenaje, se ejecuta el punto de control del WAL sobre ambos pools
   (`GestorDePools::punto_de_control_de_wal`) y el proceso termina con `ExitCode::SUCCESS` siempre**,
   incluso si el punto de control falla: un WAL sin consolidar no es pérdida de datos, SQLite lo
   reproduce en la siguiente apertura, y reportar un fallo de salida al orquestador por eso sería
   una falsa alarma.
7. **El punto de control solo puede actuar de verdad sobre `sessions.db`.** Comprobado el
   2026-07-30: `PRAGMA wal_checkpoint` sobre una conexión abierta en modo de solo lectura falla con
   un error de E/S de disco, y todas las conexiones de `PoolDeConocimiento` son de solo lectura por
   construcción (FR-05, `adr-0003`). `punto_de_control_de_wal` visita los dos pools, pero solo
   ejecuta `PRAGMA wal_checkpoint(TRUNCATE)` sobre la conexión de escritura de `sessions.db`;
   `knowledge_live.db` se reporta como de solo lectura, sin nada que consolidar. Abrir una conexión
   de lectura y escritura sobre esa base solo para este momento del apagado violaría precisamente
   el invariante que FR-05 fija.

## Consecuencias

### Positivas

* Ningún evento en vuelo se corta a la mitad durante un apagado ordenado, verificado por un test
  de proceso real que espera la línea `inferencia_iniciada` antes de enviar la señal.
* El proceso termina siempre con código 0 tras una señal, salvo que el propio proceso tuviera un
  fallo no relacionado con el apagado.
* `sessions.db` queda con su WAL consolidado tras una parada ordenada, reduciendo el trabajo de
  recuperación en la siguiente apertura.

### Negativas

* **El límite de drenaje no acota un evento individual patológico** (por ejemplo, una llamada de
  red a un proveedor de inferencia real que se cuelga): se comprueba entre eventos, así que un
  evento cuya llamada al proveedor no retorne puede superar el límite de diez segundos y, en
  teoría, el plazo de gracia de treinta segundos del PRD, tras el cual el orquestador manda
  `SIGKILL`. Con el proveedor simulado de esta tarea el tiempo de procesamiento está acotado por
  construcción, así que esto no puede ocurrir todavía. **Aviso para revisitar:** la etapa A-4, que
  introduce un proveedor HTTP real, debe darle un tiempo máximo por llamada cómodamente menor que
  el límite de drenaje. Queda registrado como `Pendiente` en `docs/STATUS.md`.
* `tokio::signal::unix` es específico de Unix: la célula se despliega como contenedor Linux
  (etapa A-6) y la integración continua corre sobre `ubuntu-latest`, así que esto no es una
  regresión de portabilidad, pero se deja escrito para que nadie lo redescubra como sorpresa. No
  se añade ninguna rama `cfg(windows)`: sería código sin probar sustituyendo a una plataforma que
  este producto no dirige.

## Alternativas consideradas y descartadas

### A. `tokio-util::CancellationToken` en vez de `tokio::sync::watch` (D-18)

Se descartó porque duplica exactamente lo que `tokio::sync::watch` ya expresa, a cambio de una
dependencia nueva. `watch` ya estaba habilitado en la característica `sync` de este crate; añadir
`tokio-util` solo para un tipo que hace lo mismo no se justifica. Registrado como D-18 en
`docs/bitacora-de-descartes.md`.

### B. Envolver el drenaje entero en un `timeout`

Se descartó porque cortaría el futuro en curso en cualquier punto en que estuviera —posiblemente
entre el envío de la respuesta y su anotación en el historial— exactamente el corte a medias que
esta tarea existe para impedir. Se sustituyó por la comprobación del límite **entre** eventos, que
nunca interrumpe uno ya en curso.

## Referencias

* `crates/hexcell/src/apagado.rs`: `Apagado`, `SenalDeApagado`, `LIMITE_DE_DRENAJE_POR_DEFECTO`.
* `crates/hexcell/src/motor.rs`: el bucle `select!` con `biased` y el drenaje con límite.
* `crates/hexcell-storage/src/pools.rs`: `GestorDePools::punto_de_control_de_wal`.
* `docs/adr/adr-0003-persistencia-dual.md`: `knowledge_live.db` es de solo lectura en producción.
* `docs/bitacora-de-descartes.md`, D-18: rechazo de `tokio-util::CancellationToken`.
* `docs/STATUS.md`: entrada Pendiente sobre el tiempo máximo por llamada de un proveedor real.
