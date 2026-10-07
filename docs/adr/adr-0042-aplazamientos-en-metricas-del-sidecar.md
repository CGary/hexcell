# adr-0042 — Aplazamientos por horario y por rampa visibles en la línea de métricas del sidecar

* **Estado:** Vigente (2026-10-07).
* **Etapa que lo produce:** A-6 (cierre del Hallazgo 9 de `docs/STATUS.md:578`, HEX-096).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0033-metricas-de-canal-propio-en-el-sidecar.md`,
  que el 2026-09-12 fijó el productor `sidecar/internal/metricas` y su línea periódica `key=value`, y
  `adr-0035-latencia-hasta-el-acuse-en-metricas-del-sidecar.md`, que el 2026-09-13 añadió la cuarta
  clave agregada. Este ADR añade dos claves más —`aplazadas_por_horario` y `aplazadas_por_rampa`— a
  esa misma línea, sin tipo IPC nuevo, sin subir la versión de cable (sigue en 7, `adr-0040`), sin
  tocar ningún crate Rust y sin añadir dependencia Go. La lista de claves agregadas pasa de cuatro a
  seis.

## Contexto

La sesión de laboratorio del 2026-08-20 registró el **Hallazgo 9** (`docs/STATUS.md:578`): los
aplazamientos por ventana de atención y por rampa de volumen eran invisibles. Los contadores
existían en memoria (`ContadorAplazadasPorHorario`, `ContadorAplazadasPorRampa` en
`sidecar/internal/outbox/disciplina.go`), pero no se exponían en ningún endpoint ni métrica; costó
aproximadamente una hora de diagnóstico en vivo entender por qué los mensajes no salían, y la única
visibilidad era añadir `log.Printf` temporal al código.

`adr-0033` y `adr-0035` entregaron la línea periódica `key=value` con cuatro claves agregadas
(`reconexiones_por_hora`, `silencio_entrante_ms`, `latencia_hasta_acuse_ms`, `contactos_omitidos`) más
la familia segmentada `ack_ratio.<id_conversacion>`. Los dos contadores de aplazamiento quedaron
fuera: no existía una costura por la que el paquete hoja `internal/metricas` pudiera leerlos sin
importar `internal/outbox`, lo que habría roto su disciplina de hoja.

Lo que faltaba, por tanto, era **una fuente inyectada**: el productor recibe los dos enteros a través
de una función, y es `sidecar/main.go` —la raíz de composición— quien los lee de los contadores
existentes. Así `internal/metricas` sigue sin conocer `outbox` y los contadores no se renombran ni se
mueven.

## Decisión

**1. Una fuente inyectada en el productor: `ObservarAplazamientos(func() (int64, int64))`.** El
método almacena la función bajo `p.mu` en el campo nuevo `aplazamientosFuente`. Devuelve `(horario,
rampa)`: el total acumulado de aplazamientos por ventana de atención y por rampa de volumen,
respectivamente. La firma de `NuevoProductor(reg, ahoraMs)` no cambia.

**2. Dos claves nuevas acumuladas, siempre presentes.** `Instantanea()` consulta la fuente una sola
vez por llamada, dentro del crítico, y emite `aplazadas_por_horario=%d` y `aplazadas_por_rampa=%d`
inmediatamente después de `contactos_omitidos` y antes del bucle de `ack_ratio.<id_conversacion>`.
Sin fuente inyectada ambos valores son 0, el cero de Go. Son **totales acumulados desde el arranque
del proceso**, no deltas por tick ni medias: la misma semántica acumulada que `contactos_omitidos`
y coherente con la decisión D-50 (descartado el promedio móvil).

**3. La hoja sigue siendo hoja.** La lectura de los contadores atómicos vive en `sidecar/main.go`:

```go
productorMetricas.ObservarAplazamientos(func() (int64, int64) {
	return outbox.ContadorAplazadasPorHorario.Load(), outbox.ContadorAplazadasPorRampa.Load()
})
```

`internal/metricas` no gana ningún import de `internal/outbox`, `internal/ipc` ni `internal/canal`.
El orden de lectura del cierre —horario antes que rampa— es el mismo que el de las claves emitidas;
como ninguna prueba del paquete `main` cubre ese cierre, la guarda estática
`guarda-hex-096.sh` comprueba ese orden.

**4. Formato intacto.** La línea sigue siendo un único texto plano `key=value` en el campo `detalle`,
nunca JSON ni una segunda línea; los enteros se emiten con `%d`. Ninguna clave previa cambia de
nombre ni de orden.

**5. Fuera de alcance declarado.** `ContadorAplazadasPorLatencia` y
`ContadorAplazadasPorErrorCortacircuitos` **no** se exponen: la tarea que cierra el Hallazgo 9 pide
solo horario y rampa, y ambos quedan nombrados aquí como exclusión explícita, no como descarte. La
alternativa de exponer además la latencia mínima o el error de cortacircuitos se aplaza a una tarea
que los necesite.

## Alternativas consideradas y descartadas

* **Que `internal/metricas` importara `internal/outbox` y leyera los contadores directamente** —
  descartada de raíz: rompería la disciplina de paquete hoja que `adr-0033` fijó como invariante y
  arrastraría la costura de salida entera al binario de pruebas del productor. La fuente inyectada
  preserva la frontera y es el patrón que el propio paquete ya usa para el reloj
  (`ahoraMs func() int64`).
* **Emitir los aplazamientos como deltas por tick en lugar de acumulados** — descartada: introduciría
  estado de «última lectura» que el lector del log tendría que sumar, y rompería la coherencia con
  `contactos_omitidos` y con D-50. El acumulado desde el arranque es trivialmente correcto de leer y
  de probar.
* **Colocar las claves nuevas al final de la línea, tras la familia `ack_ratio`** — descartada: la
  familia segmentada crece con cada contacto y debe quedar siempre al final para que el prefijo de
  claves agregadas sea estable y parseable. Las dos claves nuevas van con las demás agregadas.

## Consecuencias

* El Hallazgo 9 de `docs/STATUS.md:578` queda cerrado: los aplazamientos por ventana y por rampa son
  visibles en la misma línea periódica que el operador ya consume.
* El conjunto de claves agregadas estables que la tarea 20 del plan (alertas) puede consumir pasa de
  cuatro a seis. Este ADR no precondiciona ninguna alerta ni umbral: eso es frontera de la tarea 20.
* La huella en memoria del productor crece en exactamente el tamaño de un puntero de función más los
  dos contadores atómicos globales que ya existían; no se introduce estructura nueva sin cota.
* Dos pruebas nuevas (`TestAplazamientosVisiblesConFuenteInyectada` y
  `TestAplazamientosValenCeroSinFuenteInyectada`) documentan por mutación la presencia, el orden y el
  valor por omisión. La mutación de intercambio de `horario` y `rampa` pone en rojo la aserción de
  valor; la eliminación de una clave, la de presencia/orden. La eliminación literal de un `Sprintf`
  además deja de compilar por variable sin usar, un rojo aún más temprano, y la guarda estática
  `FALLA[metricas-orden]` discrimina ese caso.
* No se toca `sidecar/go.mod` ni `sidecar/go.sum`, ni `registro.Campos`, ni ninguna clave previa.

## Referencias

* `docs/adr/adr-0033-metricas-de-canal-propio-en-el-sidecar.md` (mecanismo extendido).
* `docs/adr/adr-0035-latencia-hasta-el-acuse-en-metricas-del-sidecar.md` (última extensión, cuarta clave).
* `docs/adr/adr-0024-metricas-internas-de-operacion.md` (mecanismo homólogo del núcleo).
* `docs/bitacora-de-descartes.md`, **D-61** (alternativas de esta entrega).
* `docs/STATUS.md:578` (Hallazgo 9, cerrado por este registro).
* `sidecar/internal/metricas/metricas.go`, `sidecar/internal/metricas/metricas_test.go`.
* `sidecar/internal/outbox/disciplina.go` (contadores `ContadorAplazadasPorHorario` y
  `ContadorAplazadasPorRampa`, no renombrados).
* `sidecar/main.go` (raíz de composición que inyecta la fuente).
