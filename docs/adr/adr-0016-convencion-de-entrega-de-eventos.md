# ADR-0016 — Convención de entrega de eventos del puerto de canal

* **Estado:** Vigente desde el 2026-07-29.
* **Supersede a:** nada.
* **Etapa:** A-2 (HEX-004).
* **Requisitos tocados:** FR-01, FR-12.

---

## Contexto

El puerto `ChannelAdapter` (`crates/hexcell-core/src/canal.rs`, `adr-0010`) declara solo dos
métodos: `send` y `estado_ventana`. El propio archivo lo dice en prosa: «el mecanismo de
entrega —suscripción, flujo o retrollamada— no es uno de los siete elementos de FR-12 y se
decide en la etapa A-2». Esta tarea (HEX-004) es esa etapa, y el motor de mensajería que
introduce necesita, de alguna forma, recibir el `EventoEntrante` que cada adaptador produce.

La restricción que encuadra la decisión es doble. Primero, `hexcell-core` está cerrado por
HEX-002 y esta tarea no lo reabre: no se le añade un método `recv`/`subscribe` a
`ChannelAdapter`, ni una dependencia de `tokio`. Segundo, `ChannelAdapter` usa
`-> impl Future<Output = ...> + Send` en sus métodos (para evitar el aviso `async_fn_in_trait`
bajo `-D warnings`, según registra `adr-0002`), y esa forma de retorno hace que el trait **no**
sea compatible con objetos de trait: `Box<dyn ChannelAdapter>` no compila. La entrega de eventos,
sea cual sea el mecanismo elegido, tiene que vivir fuera del trait y fuera de `hexcell-core`.

## Decisión

1. **Cada adaptador crea y posee un canal `tokio::sync::mpsc::channel<EventoEntrante>`
   acotado**, no ilimitado. Al construirse, el adaptador se queda con el extremo emisor
   (`Sender`) y entrega el extremo receptor (`Receiver`) a quien lo construye, para que el
   `Motor` lo consuma con `receptor.recv().await` en su bucle principal.
2. **No se añade ningún método nuevo a `ChannelAdapter`.** La entrega de eventos no es uno de
   los siete elementos normativos de FR-12 y no se convierte en uno aquí: es una convención de
   construcción entre el binario `hexcell` y cualquier adaptador que quiera conectarse a él, no
   una obligación del tipo del puerto.
3. **El canal es acotado, no ilimitado, a propósito.** Una ráfaga de eventos entrantes debe
   aplicar contrapresión sobre el adaptador —que decide cómo reaccionar ante un canal lleno—
   en vez de crecer sin límite y presionar el presupuesto de memoria de NFR-01 (≤ 80 MB por
   célula sobre canal propio). La capacidad se configura con `HEXCELL_CAPACIDAD_COLA`.
4. **Todo adaptador que se conecte al binario `hexcell` debe adoptar esta misma convención.**
   El adaptador de whatsmeow de la etapa A-3, ya cerrada aparte de esta partición de A-2, queda
   obligado a entregar sus eventos por el mismo mecanismo si quiere conectarse al motor que esta
   tarea introduce; si no lo hace, la costura entre el adaptador y el motor se rompe. Esto es una
   consecuencia que se enuncia sin atenuarla: la convención vive fuera del compilador, así que
   nada impide a un futuro adaptador ignorarla salvo este mismo documento y la revisión del
   código que lo introduzca.

Nada de esto cambia la lectura de `ChannelAdapter` como frontera de coexistencia entre canales
(`adr-0010`): el canal propio y el canal oficial pueden seguir vivos a la vez en células
distintas, cada uno con su propio canal `mpsc` interno, sin que uno sepa nada del otro.

## Consecuencias

### Positivas

* `hexcell-core` permanece exactamente como lo cerró HEX-002: sin `tokio`, sin ningún runtime
  asíncrono, sin HTTP. La entrega de eventos es infraestructura del binario, no del dominio.
* El canal acotado da un punto de contrapresión explícito y configurable
  (`HEXCELL_CAPACIDAD_COLA`) en vez de una cola que crece sin límite bajo una ráfaga de tráfico.
* La decisión es reversible con un coste acotado: cambiar el mecanismo de entrega el día de
  mañana no exige tocar `ChannelAdapter` ni ningún consumidor del puerto, solo el punto de
  construcción de cada adaptador y del `Motor`.

### Negativas

* **La convención no está impuesta por el compilador.** Nada en el tipo `ChannelAdapter` obliga
  a un adaptador nuevo a exponer un `mpsc::Receiver<EventoEntrante>` de esta forma; solo este ADR
  y la revisión de código lo hacen cumplir. Un adaptador que decida entregar eventos de otra
  manera compilaría igual y rompería la costura con el `Motor` en tiempo de ejecución, no de
  compilación.
* **El adaptador de whatsmeow (etapa A-3) queda obligado a esta convención** aunque ya esté
  cerrado aparte de esta partición de A-2: si su forma de entregar eventos no encaja con un
  canal `mpsc` acotado poseído por el propio adaptador, esa costura tiene que revisarse contra
  este ADR, no inventarse de nuevo sobre la marcha.

## Alternativas consideradas y descartadas

### A. Añadir un método `recv`/`subscribe` a `ChannelAdapter`

Es la forma más directa de resolver la entrega desde dentro del puerto. Se descarta por dos
motivos: reescribiría un trait que HEX-002 cerró, violando la restricción explícita de esta
tarea de no tocar `hexcell-core`; y, aunque se permitiera, `ChannelAdapter` no es compatible con
objetos de trait (`-> impl Future`, `adr-0002`), así que un método de suscripción tendría el
mismo problema de despacho estático que ya tiene `send`, sin resolver nada que la convención
externa no resuelva ya.

### B. Registro de una función de retrollamada (`callback`)

El adaptador invocaría una función que el binario le pasa cada vez que llega un evento, en vez
de que el binario extraiga eventos de un canal. Se descarta porque invierte el control: el
adaptador pasaría a decidir cuándo y en qué contexto de ejecución se procesa cada evento, lo que
mete conocimiento del ejecutor (Tokio, en este caso) dentro del adaptador. El `Motor` es quien
debe decidir el ritmo de consumo, no el adaptador.

## Referencias

* `crates/hexcell-core/src/canal.rs`: declaración de `ChannelAdapter`, `EventoEntrante` y la
  prosa que difiere esta decisión a la etapa A-2.
* `adr-0002-estructura-workspace.md`: consecuencia de `-> impl Future` sobre la compatibilidad
  con objetos de trait.
* `adr-0010-puerto-de-canal.md`: el puerto como frontera de coexistencia, no de migración.
* `docs/plan/fase-a-2-nucleo-persistencia.md`: tareas 1 a 4, que esta tarea (HEX-004) implementa.
* `docs/STATUS.md`: entrada Definido de esta convención, fechada 2026-07-29.
