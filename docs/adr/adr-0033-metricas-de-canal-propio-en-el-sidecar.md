# adr-0033 — Métricas nativas del canal propio: productor de tres series acotadas en el sidecar

* **Estado:** Vigente (2026-09-12).
* **Etapa que lo produce:** A-6 (tarea 25-b del plan de la etapa A-6: `docs/plan/fase-a-6-empaquetado-cli.md`, HEX-072-b).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0024-metricas-internas-de-operacion.md`,
  que fijó el mecanismo de instantánea periódica en `key=value` para el lado Rust del núcleo. Este
  ADR aplica el mismo mecanismo, homólogo pero propio, al lado Go del sidecar. Se apoya en el
  sumidero en-proceso de `sidecar/internal/canal/acuses.go` (HEX-072-a, ver D-45).

## Contexto

`adr-0024` cubrió las métricas internas del **núcleo Rust** (admisión GCRA, concurrencia, saldo
financiero). El **sidecar Go**, que sostiene la sesión de whatsmeow, quedó fuera de ese alcance a
propósito: en 2026-08-27 el sidecar todavía no clasificaba acuses de entrega/lectura ni exponía
nada sobre la salud de la conexión de canal propio. HEX-072-a (2026-09-11) cerró la primera mitad
de esa brecha —el sumidero `canal.SumideroDeAcuses`, en proceso y sin salir jamás por IPC (D-45)—
mientras dejaba explícitamente para esta tarea, en su propio comentario de código, componer un
consumidor en `sidecar/main.go`.

El operador de una célula sobre canal propio necesita observar, del lado Go y sin depurador:

1. Si los mensajes salientes efectivamente llegan y se leen, **por contacto** —una cifra agregada
   escondería a un contacto concreto con entregas fallidas detrás del promedio general del resto—.
2. Con qué frecuencia se reconecta la sesión de whatsmeow, indicador temprano de degradación del
   canal o de un baneo en curso (`adr-0015`).
3. Cuánto tiempo lleva sin llegar ningún evento entrante, señal de una sesión colgada que superó
   silenciosamente su reconexión.

El diseño hereda los mismos límites que `adr-0024` fijó del lado Rust:

* Ningún endpoint HTTP ni socket nuevo.
* Ninguna tabla de historial ni escritura a `sqlstore.db` o `identidad.db`.
* Ningún mensaje IPC nuevo, ninguna subida de versión de cable: el protocolo IPC (versión 6,
  `adr-0032`) queda intacto.
* Huella de memoria despreciable y acotada por construcción.

A eso se suma un límite propio de este productor: **acuse de entrega/lectura no lleva ningún
identificador de contacto**. `canal.Acuse` (HEX-072-a) e `ipc.AcuseEnvio` solo llevan
`id_correlacion`, `estado` y una marca de tiempo. Segmentar por contacto exige entonces mantener,
además del estado por contacto, una unión transitoria `id_correlacion -> id_conversacion` mientras
el envío sigue sin confirmar.

## Decisión

**1. Paquete hoja `sidecar/internal/metricas`, un único archivo de producción:**
Toda la lógica del productor vive en `metricas.go`. El paquete importa solo la biblioteca estándar
más `internal/registro`: nunca `internal/canal`, `internal/outbox` ni `whatsmeow`. Los
observadores (`ObservarEnvio`, `ObservarAcuse`, `ObservarEstadoSesion`, `ObservarEntrante`) reciben
escalares —cadenas y no tipos concretos de esas costuras—, de modo que `sidecar/main.go`, la raíz
de composición, es quien adapta cada tipo a esta API. Esto evita cualquier riesgo de ciclo de
importación y mantiene el binario de pruebas del paquete libre de `whatsmeow`.

**2. Dos mapas acotados con desalojo determinista y un contador compartido de truncamiento:**
`contactos` (cota `MaximoContactos = 256`) es el estado de largo plazo por conversación; `correlaciones`
(cota `MaximoCorrelaciones = 1024`) es la unión transitoria que resuelve el acuse mientras el envío
está en vuelo, y se borra en cuanto `ObservarAcuse` la resuelve —libera cupo tan pronto cumple su
único propósito—. Superada cualquiera de las dos cotas, el desalojo es **siempre determinista**:
la entrada de actividad más antigua, con el id ascendente como desempate (doctrina D-08), nunca al
azar ni por el orden de iteración del mapa de Go. Ambas clases de desalojo incrementan el mismo
contador `contactos_omitidos`, para que el truncamiento sea observable y nunca silencioso.

**3. Emisión de una sola línea `key=value` cada 60 segundos:**
`Productor.Bucle` emite, con el mismo `IntervaloDeInstantanea` que `adr-0024` (60 s), una entrada
`sidecar.metricas_instantanea` con las claves siguientes en el campo `detalle`
(nunca JSON, la misma convención que `metricas_instantanea` del núcleo):

* `reconexiones_por_hora` — transiciones hacia el estado `activa` normalizadas por el tiempo
  transcurrido desde el arranque del productor; una reconexión repetida sin una desconexión
  intermedia no infla el conteo.
* `silencio_entrante_ms` — milisegundos desde el último evento entrante observado.
* `contactos_omitidos` — contador acumulado y compartido de ambos tipos de desalojo.
* `ack_ratio.<id_conversacion>` — una entrada por cada contacto conocido, en orden ascendente de
  id para que dos llamadas sobre el mismo estado produzcan el mismo texto byte a byte. **Nunca**
  se emite como una sola cifra agregada.

Estas claves son una interfaz publicada: la tarea 20 del plan (notificaciones, que depende de
25-b) las consume como condición de alerta y deben permanecer estables.

**4. Guarda de privacidad como hecho de tipo, no solo de convención:**
`ObservarEnvio` rechaza silenciosamente cualquier `id_conversacion` con forma de JID (contiene
`"@"`) como clave de contacto. Es una defensa en profundidad: además de que ninguna costura de
`main.go` debe pasar jamás un JID, el propio productor lo descarta si de todos modos llegara uno,
para que la frontera de `adr-0019` —`registro.Campos.IdConversacion`, "nunca un JID"— no dependa
solo de la disciplina del llamador.

**5. Cableado aditivo en `sidecar/main.go`, sin tocar las costuras existentes:**
`main.go` sigue siendo wiring puro. `transmisorObservado` decora `outbox.Transmisor` —la única
costura del sidecar donde `id_conversacion` e `id_correlacion` conviven a la vez— sin editar
`outbox/salida.go`; `recursos.Sesion.RegistrarManejadorDeAcuses` deja de ser código muerto y
alimenta `ObservarAcuse`; el sumidero de estado de sesión del supervisor y el sumidero de evento
entrante se decoran igual, y `Productor.Bucle` arranca en una goroutine junto a
`bucleDeDrenajeSalida`.

## Alternativas consideradas y descartadas

Ver `docs/bitacora-de-descartes.md`, entrada **D-46**, para las alternativas de cardinalidad y
mecanismo de emisión estudiadas y descartadas al diseñar este productor.

## Consecuencias

* El sidecar gana visibilidad operativa por contacto sobre la salud de entrega del canal propio,
  sin tocar el protocolo IPC ni ningún crate Rust.
* `sidecar/internal/canal/acuses.go` (HEX-072-a) deja de ser código sin consumidor: su comentario
  propio queda satisfecho.
* La huella en memoria queda acotada por construcción: 256 contactos más 1024 correlaciones caen en
  el orden de magnitud de ~110 KB, sin ninguna estructura sin cota.
* Las claves `reconexiones_por_hora`, `silencio_entrante_ms`, `contactos_omitidos` y
  `ack_ratio.<id_conversacion>` quedan documentadas como la interfaz estable que consume la tarea
  20 del plan (notificaciones).
* Latencia hasta el acuse (la cuarta serie prometida originalmente en `fase-a-3-adaptador-whatsmeow.md`)
  queda explícitamente diferida, no implementada por esta tarea.

## Referencias

* `docs/adr/adr-0024-metricas-internas-de-operacion.md` (mecanismo extendido).
* `docs/adr/adr-0019-registro-estructurado.md` (conjunto cerrado de campos y frontera de privacidad).
* `sidecar/internal/canal/acuses.go` (HEX-072-a, D-45).
* `sidecar/internal/metricas/metricas.go`, `sidecar/internal/metricas/metricas_test.go`.
* `docs/plan/fase-a-6-empaquetado-cli.md`, tarea 25-b.
* `docs/bitacora-de-descartes.md`, D-46.
