# ADR 0005: Contabilidad financiera en dos fases (Reserva previa y Conciliación posterior)

* **Estado**: Vigente (Fase 1: Reserva previa implementada en HEX-042; Fase 2: Conciliación posterior implementada en HEX-043; Fase 3: Modo degradado implementado en HEX-045)
* **Fecha**: 2026-08-26
* **Etapa**: A-4 (FR-10)

## Contexto

El producto HexCell ejecuta peticiones de inferencia sobre modelos de lenguaje externos. Cada llamada a un proveedor de inferencia tiene un coste computacional y presupuestario. Para evitar el sobregiro de saldo disponible o el consumo no contabilizado en ejecuciones no autorizadas, la célula requiere un mecanismo contable riguroso antes de invocar cualquier proveedor externo.

La persistencia del saldo y los movimientos se apoya en la migración `0002-saldo-y-movimientos.sql` sobre `sessions.db`, garantizando que el saldo disponible no se vuelva negativo mediante la restricción `CHECK (disponible >= 0)`.

## Decisión

Adoptar un esquema contable financiero en **dos fases** para el control del presupuesto de inferencia:

### Fase 1: Reserva previa atómica (Hold pre-ejecución)
1. Antes de invocar al proveedor de inferencia (`ProveedorDeInferencia`), se calcula una estimación determinista del coste basada en la longitud del prompt entrante (`estimar_coste` en `hexcell-core`), dividiendo los caracteres Unicode entre `CARACTERES_POR_UNIDAD_ESTIMADA` (4) y con un suelo de `UNIDADES_MINIMAS_POR_LLAMADA` (1).
2. Se ejecuta una transacción atómica SQLite en `hexcell-storage` (`reservar_presupuesto`) que:
   - Verifica la suficiencia de `saldo.disponible`.
   - Si es suficiente, inserta un registro con estado `'activa'` en la tabla `reservas`.
   - Decrementa `saldo.disponible` y aumenta `saldo.reservado`.
   - Registra un movimiento con clase `'reserva'` y monto negativo en la tabla `movimientos`.
3. Si el saldo es insuficiente, la reserva devuelve `VeredictoDeReserva::Rechazada`. El procesador de inferencia no llama al proveedor, emite un registro estructurado `presupuesto_rechazado` y retorna `None` (fail-closed) [Nota: Esta última cláusula de retorno `None` queda superada en la Fase 3 por la respuesta local en modo degradado].

### Fase 2: Conciliación o liberación posterior (HEX-043)
Una vez completada la llamada al proveedor de inferencia:
1. En caso de respuesta exitosa (`Ok(RespuestaDeInferencia)`), `ProcesadorDeInferencia` invoca `conciliar_presupuesto` en una única transacción atómica SQLite:
   - La reserva activa pasa a estado `'conciliada'`, fijando `resuelta_ms`.
   - Se reduce `saldo.reservado` en el monto originalmente retenido N.
   - Si la cantidad consumida real M es menor o igual a N, la diferencia N - M se acredita a `saldo.disponible`.
   - Si la cantidad consumida real M supera a N, el déficit M - N se debita de `saldo.disponible`, acotado al disponible existente para no violar `CHECK (disponible >= 0)`. El remanente no cubierto se reporta en `ResultadoDeResolucion::Resuelta.deficit_no_cubierto` y emite el registro estructurado `presupuesto_deficit_no_cubierto`. Este remanente no se inserta en `movimientos` para no violar el `CHECK` de clase de movimiento en la migración 0002.
   - Si el ajuste neto sobre disponible es cero (M == N o déficit sin saldo disponible), se omite la inserción en `movimientos` respetando la restricción `CHECK (monto <> 0)`.
2. En caso de fallo del proveedor (`Err`), `ProcesadorDeInferencia` invoca `liberar_presupuesto` en una única transacción atómica SQLite:
   - La reserva activa pasa a estado `'liberada'`, fijando `resuelta_ms`.
   - El monto originalmente retenido N se reintegra íntegramente a `saldo.disponible` y se reduce `saldo.reservado`.
   - Se inserta un movimiento de clase `'liberacion'` con monto +N.
3. La gestión de temporizadores (timeouts de red) queda diferida a la tarea 9 (cliente HTTP de inferencia real); cualquier fallo de transporte o timeout provocado por dicho cliente tomará la ruta de `liberar_presupuesto`.

### Fase 3: Respuesta local en modo degradado (HEX-045, 2026-08-27)
1. Si la reserva devuelve `VeredictoDeReserva::Rechazada` (saldo insuficiente), el procesador de inferencia no llama al proveedor de inferencia ni crea ninguna reserva o movimiento en el libro contable (coste de presupuesto cero).
2. En su lugar, el procesador emite el registro estructurado `modo_degradado` (además del existente `presupuesto_rechazado`) y genera una respuesta local provisional basada en reglas fijas con cero unidades de presupuesto consumidas (`unidades_consumidas` == 0).
3. Una vez restaurado el saldo de la célula mediante un aporte, el procesador retoma de forma automática la ruta ordinaria de inferencia en la siguiente petición.

## Consecuencias

* **Positivas**:
  - Evita llamadas no autorizadas o sin presupuesto a proveedores de inferencia externos.
  - Invariante de saldo no negativo (`disponible >= 0`) garantizado a nivel de base de datos e interfaz.
  - Cierre completo del ciclo de vida de las reservas: ninguna reserva creada por `reservar_presupuesto` permanece en estado `'activa'` tras concluir la llamada a la inferencia (éxito o fallo).
  - La contabilidad usa unidades enteras opacas sin presuponer precios ni monedas (monetización pendiente).
  - La política de fallo ante errores de almacenamiento es *fail-closed* en la ruta contable.
* **Negativas / Limitaciones**:
  - El déficit que supere el saldo disponible en el momento de conciliar se acota a cero disponible y el remanente no cubierto queda registrado únicamente en métricas/logs sin asiento contable negativo en el libro.
  - El texto de respuesta provisional enviado en el modo degradado es un marcador de posición técnico del mecanismo, cuya redacción final comercial queda pendiente de una decisión de producto.
