# adr-0035 — Latencia hasta el acuse como cuarta clave del productor de métricas del sidecar

* **Estado:** Vigente (2026-09-13).
* **Etapa que lo produce:** A-6 (tarea 20 de `docs/plan/fase-a-6-empaquetado-cli.md`, HEX-077-c).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0033-metricas-de-canal-propio-en-el-sidecar.md`,
  que el 2026-09-12 declaró "explícitamente diferida" la cuarta serie de la promesa original de
  A-3. Este ADR cierra ese diferido entregando `latencia_hasta_acuse_ms` en la misma línea
  periódica `key=value` ya definida por adr-0033. adr-0033 sigue vigente tal cual; este registro
  solo amplía la lista de "cuatro claves agrupadas" a cinco. No introduce tipo IPC nuevo, no
  toca `docs/protocolo-ipc-nucleo-sidecar.md`, no sube la versión de cable (sigue en 6,
  `adr-0032`) y no añade dependencia Go.

## Contexto

`adr-0033` cerró la tarea 25-b de A-6 entregando tres series acotadas del productor de
métricas nativas del canal propio (`reconexiones_por_hora`, `silencio_entrante_ms`,
`contactos_omitidos` y la familia segmentada `ack_ratio.<id_conversacion>`), y dejó escrito en
su sección **Consecuencias**:

> "Latencia hasta el acuse (la cuarta serie de la promesa original de A-3) queda explícitamente
> diferida, no implementada por esta tarea."

La promesa venía de `docs/plan/fase-a-3-adaptador-whatsmeow.md:105-109` y se reiteraba en
`docs/plan/fase-a-6-empaquetado-cli.md:320-323` y en la nota de cierre de la tarea 25-b. HEX-077-c
existe para cerrar ese diferido, **dentro de la misma sub-tarea (per-cell-metrics) de la tarea
20** de A-6 y sin tocar los otros tres hijos de HEX-077 (puerto de notificación en `HEX-077-a`,
ocho condiciones de alerta en `HEX-077-b`, dead-man's switch en `HEX-077-d`).

El dato crudo necesario ya existe en el productor y **no requiere una nueva fuente**:

* `correlacionPendiente.creadaMs` se estampa en `ObservarEnvio` con el mismo reloj inyectado
  (`p.ahoraMs()`) que `Instantanea` ya usa para `silencio_entrante_ms` y para
  `reconexiones_por_hora`.
* El intervalo entre esa marca y el instante en que `ObservarAcuse` resuelve la correlación
  **se conoce** sin reloj nuevo: es `ahora - corr.creadaMs`, exactamente la misma operación
  que la línea de `silencio_entrante_ms` ya hace con `p.ultimoEntranteMs`.
* El cierre del intervalo se da **antes** de que `ObservarAcuse` borre la entrada del mapa
  `correlaciones`, así que el cálculo cabe sin alterar el orden de las instrucciones
  existentes en ese método.

No hay, por tanto, motivo para añadir señal nueva, productor nuevo, campo IPC nuevo, ni bump
de cable: el coste de borde de esta entrega es un campo `int64` y cinco líneas de cálculo.

## Decisión

**1. Una sola clave nueva: `latencia_hasta_acuse_ms`, en milisegundos enteros (`%d`).** Se añade
al payload de `Instantanea()` entre `silencio_entrante_ms` y `contactos_omitidos`, de modo que
las cinco claves agregadas vayan siempre juntas y la familia segmentada
`ack_ratio.<id_conversacion>` siga al final, sin alterar el orden que la tarea 20 del plan ya
documenta.

**2. Semántica de "última observada" —no promedio, no percentil, no segmento por contacto.**
El valor se actualiza en cada `ObservarAcuse` que resuelve una correlación conocida y
sobre-escribe el previo. Es la misma semántica que `ultimoEntranteMs` aplica a
`silencio_entrante_ms`: cada nuevo evento sustituye la marca, no se acumula con anteriores.
La métrica es un único entero por célula, comparable con las tres series agregadas de adr-0033
en cardinalidad y granularidad. La alternativa de promedio se estudió y se descartó en `D-50`
por dos razones: (a) introduce estado nuevo (contador y suma) que incrementa la huella y
exige disciplina de promoción a la vista, mientras que un entero único es trivialmente
correcto de leer y de probar por mutación; (b) el evento de interés para la alerta futura de
la tarea 20 no es la "tendencia" sino el **último caso**, y la media móvil sería un cambio
de contrato respecto a la tarea 20 que merece su propio ADR, no una re-implementación
silenciosa.

**3. Cálculo dentro de `ObservarAcuse`, después de la guarda de existencia y antes del
`delete`.** La secuencia es:

1. guardar la entrada resuelta del mapa `correlaciones` en `corr`;
2. comprobar que existe (la guarda contra contactos fantasma, ya probada por
   `TestAcuseDeCorrelacionDesconocidaSeIgnoraSinContactoFantasma`, sigue aplicando y no se
   relaja);
3. calcular `latenciaMs := ahora - corr.creadaMs`, con clamp a 0 si fuera negativo (mismo
   `if latenciaMs < 0 { latenciaMs = 0 }` que `Instantanea()` ya usa para `silencioMs`);
4. asignar `p.ultimaLatenciaAcuseMs = latenciaMs`;
5. continuar con el `c.acusados++` y el `delete` que ya estaban.

Este orden preserva la invariante de que un acuse sobre una correlación desconocida **no
altera** la métrica, exactamente igual que ya no crea un contacto fantasma ni mueve
`contactos_omitidos`.

**4. Formato de emisión: entero con signo, `latencia_hasta_acuse_ms=%d`.** Nunca `%.2f`, nunca
JSON, nunca una segunda línea de registro. Entero en milisegundos por tres razones: (a) la
unidad ya es la del resto de las series agregadas (`silencio_entrante_ms`, etc.), de modo que
el lector del log aplica una sola regla de parseo; (b) `int64` no admite precisión perdida,
así que la aserción de la prueba de mutación no se puede colapsar por un redondeo de `%.2f`;
(c) el contrato de la tarea 20 del plan (alertas) sigue siendo "un entero por célula,
comparables con `reconexiones_por_hora` y `silencio_entrante_ms`", y un float lo rompería.

**5. Cero cambios fuera del productor.** `sidecar/main.go` sigue siendo wiring puro: el
manejador que hoy cablea `RegistrarManejadorDeAcuses` a `productorMetricas.ObservarAcuse` ya
pasa los dos argumentos que la métrica necesita (`idCorrelacion` y `estado`); la marca de
tiempo de creación la estampa el propio productor con el reloj inyectado, no la trae
`canal.Acuse`. Ni `sidecar/internal/canal/acuses.go` ni `sidecar/internal/ipc/mensajes.go` ni
el documento del protocolo IPC se tocan. El conjunto cerrado de tipos del protocolo y su
versión de cable 6 (adr-0032) quedan intactos.

## Alternativas consideradas y descartadas

* **Promedio móvil de latencias sobre una ventana de N acuses** — descartado. Ver
  `docs/bitacora-de-descartes.md`, entrada **D-50**. Introduce estado nuevo para una
  funcionalidad que la promesa de A-3 y la tarea 20 no piden, y cambia la cardinalidad de la
  métrica agregada sin un ADR específico que lo justifique.
* **Emitir la métrica segmentada por `id_conversacion` (estilo `latencia_hasta_acuse.<id>`)**
  — descartada por contrato: la guarda `adr-0019` aplicaría igual que a `ack_ratio`, y el
  join `id_correlacion -> id_conversacion` se borra en cuanto el acuse se observa, así que
  el segmento por contacto solo estaría disponible en el instante del acuse —no en la
  siguiente instantánea—. Queda reservada, si la tarea 20 la pide, a un ADR futuro que la
  trate como cambio de contrato.
* **Calcular la latencia en `sidecar/main.go` y pasarla como parámetro a `ObservarAcuse`** —
  descartada por tamaño: añadir un parámetro a `ObservarAcuse` rompe su firma pública, y
  abre la puerta a que un llamador futuro calculase contra un reloj distinto del que
  `ObservarEnvio` usó para estampar `creadaMs`, contaminando la métrica con skew. El cálculo
  dentro del productor usa el mismo `p.ahoraMs()` que `ObservarEnvio`, así que skew por
  construcción es cero.

## Consecuencias

* La promesa original de la tarea 25 de A-3 queda **completa** en su sub-tarea
  per-cell-metrics (las cuatro series que el plan enumera: reconexiones por hora, silencio
  entrante, latencia hasta el acuse y ratio de acuse por contacto, este último ya entregado
  por HEX-072-b).
* `adr-0033` queda cerrado en su promesa: la frase "queda explícitamente diferida" deja de
  ser prospectiva y pasa a ser histórica. La sección "Consecuencias" de adr-0033 no se
  reescribe: este ADR la extiende, igual que adr-0033 extendió a su vez a adr-0024.
* El conjunto de claves estables que la tarea 20 del plan (alertas) consume como condición
  queda ampliado a cinco: `reconexiones_por_hora`, `silencio_entrante_ms`,
  `latencia_hasta_acuse_ms`, `contactos_omitidos` y la familia `ack_ratio.<id_conversacion>`.
  HEX-077-b puede consumir la nueva clave cuando se implemente —el contrato de este ADR no
  la precondición con ningún umbral ni ninguna notificación, esa es la frontera de
  HEX-077-b, no de este registro—.
* La huella en memoria del productor crece en exactamente 8 bytes (un `int64`), despreciable
  contra los ~110 KB ya documentados en adr-0033.
* Cuatro pruebas nuevas (`TestLatenciaHastaAcuseCalculadaSobreAcuseConocido`,
  `TestLatenciaHastaAcuseValeCeroAntesDeCualquierAcuse`,
  `TestLatenciaHastaAcuseReflejaElMasRecienteNoElPrimero`,
  `TestLatenciaHastaAcuseNoSeMuevePorAcuseHuerfano`) documentan por mutación cada uno de
  los cuatro frentes de fallo: no calcular, no inicializar a cero, enclavar tras el primer
  acuse y computar un elapsed espurio desde una correlación inexistente. La evidencia de
  mutación medida el 2026-09-13 es la siguiente, y se registra con precisión en vez de
  redondearse: forzar `latenciaMs := int64(0)` pone **tres** de las cuatro pruebas en rojo
  bajo `-count=1` (`...CalculadaSobreAcuseConocido` esperaba 1500, `...ReflejaElMasRecienteNoElPrimero`
  esperaba 4000, `...NoSeMuevePorAcuseHuerfano` esperaba 750 como precondición), con tres
  valores esperados distintos, lo que demuestra que discriminan y no asertan una constante.
  La cuarta, `...ValeCeroAntesDeCualquierAcuse`, documenta el valor por omisión y por
  construcción no puede ponerse roja con esa mutación: no se afirma que lo haga.

## Referencias

* `docs/adr/adr-0033-metricas-de-canal-propio-en-el-sidecar.md` (mecanismo extendido).
* `docs/adr/adr-0024-metricas-internas-de-operacion.md` (mecanismo homólogo del núcleo).
* `docs/adr/adr-0019-registro-estructurado.md` (frontera de privacidad, no se toca).
* `docs/adr/adr-0032-protocolo-ipc-version-de-cable-6.md` (versión de cable 6, no se sube).
* `docs/plan/fase-a-6-empaquetado-cli.md` líneas 304-350 (tarea 20, sub-tarea
  per-cell-metrics) y línea 375 (nota de cierre de la tarea 25-b).
* `sidecar/internal/metricas/metricas.go`, `sidecar/internal/metricas/metricas_test.go`.
* `sidecar/internal/canal/acuses.go` (HEX-072-a, sumidero en-proceso que este ADR no toca).
* `docs/bitacora-de-descartes.md`, **D-50** (alternativa de promedio descartada).
