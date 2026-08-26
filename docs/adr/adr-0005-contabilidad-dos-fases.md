# ADR 0005: Contabilidad financiera en dos fases (Reserva previa y Conciliación posterior)

* **Estado**: Vigente (Fase 1: Reserva previa implementada en HEX-042; Fase 2: Conciliación posterior pendiente)
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
3. Si el saldo es insuficiente, la reserva devuelve `VeredictoDeReserva::Rechazada`. El procesador de inferencia no llama al proveedor, emite un registro estructurado `presupuesto_rechazado` y retorna `None` (fail-closed).

### Fase 2: Conciliación o liberación posterior (Pendiente de implementación)
Una vez completada la llamada al proveedor de inferencia:
- Si la respuesta es exitosa o ajustada al consumo real, la reserva activa pasa a estado `'conciliada'`, ajustando el saldo reservado y registrando un movimiento de `'conciliacion'`.
- Si la llamada falla o se cancela, la reserva activa pasa a estado `'liberada'`, devolviendo el monto retenido al saldo disponible y registrando un movimiento de `'liberacion'`.
- *Nota*: La Fase 2 está fuera del alcance de la tarea HEX-042 y se implementará en la tarea posterior de conciliación.

## Consecuencias

* **Positivas**:
  - Evita llamadas no autorizadas o sin presupuesto a proveedores de inferencia externos.
  - Invariante de saldo no negativo (`disponible >= 0`) garantizado a nivel de base de datos e interfaz.
  - La contabilidad usa unidades enteras opacas sin presuponer precios ni monedas (monetización pendiente).
  - La política de fallo ante errores de almacenamiento es *fail-closed* en la ruta contable.
* **Negativas / Limitaciones**:
  - Hasta que se implemente la Fase 2 (conciliación), las reservas activas permanecen en estado `'activa'` y el saldo reservado no se libera automáticamente.
