# ADR-0036: Condiciones de alerta sobre señales existentes

**Fecha:** 2026-09-13  
**Última revisión:** 2026-09-14 (sección «Prioridad máxima del baneo: lo que se exige y lo que se entrega»)  
**Estado:** Vigente  
**Etapa:** A-6

## Decisión

Siete de las ocho condiciones de alerta de la tarea 20 del plan (`docs/plan/fase-a-6-empaquetado-cli.md`,
líneas 320-331) se implementan consumiendo señales que **ya existen** en el sidecar (etapa A-3) o en
el núcleo (etapa A-4), sin inventar ningún productor de señal nuevo. La octava condición —bucle de
reinicios de cualquiera de los dos contenedores— queda explícitamente fuera de esta tarea por
decisión humana del 2026-09-13: no existe ningún productor de señal para ella en el repositorio.

## Señales consumidas

| Condición | Código | Señal existente | Origen |
|:---|:---|:---|:---|
| Baneo temporal detectado (AC-2) | `baneo_temporal_detectado` | `expira_en_ms` del `estado_sesion` IPC | Sidecar (A-3) |
| Sesión desvinculada (AC-3) | `sesion_desvinculada` | `EstadoSesion::Desvinculada` | Sidecar (A-3) |
| Sidecar sin reconectar (AC-4) | `sidecar_sin_reconectar` | `EstadoSesion::Reconectando` sostenido | Sidecar (A-3) |
| Saldo LLM agotado (AC-6) | `saldo_llm_agotado_o_modo_degradado` | `InstantaneaDeMetricas.disponible` | Núcleo (A-4) |
| Tasa GCRA anómala (AC-7) | `tasa_descartes_gcra_anomala` | `descartados_admision / admitidos` | Núcleo (A-4) |
| Envío no solicitado (AC-8, proxy) | `descarte_envio_no_solicitado` | `rechazos_de_construccion()` delta | Núcleo (A-3) |
| Caída ratio acuses por contacto (AC-9) | `caida_anomala_ratio_acuses_por_contacto` | Contadores por `id_conversacion` | Adaptador (espejo Go) |

## Confinamiento deliberado cruzado

Dos señales ya llegaban al adaptador de whatsmeow pero se descartaban a propósito:

1. **`expira_en_ms` del `estado_sesion`**: el cable IPC lo transporta, pero el mapeo al dominio
   (`EstadoSesion` sin campos) lo descarta con el comentario «causa, codigo y expira_en_ms se quedan
   DENTRO de este crate». Esta tarea lo eleva por un `watch` que difunde el **par**
   `(EstadoSesion, Option<SystemTime>)`, sin tocar el puerto `ChannelAdapter`.

   **Por qué un solo `watch` de pares y no dos canales**: con dos canales independientes, un
   consumidor que leyera el estado de uno y la expiración del otro podría observar `Pausada` antes
   de que la expiración llegara a su canal, emitir la alerta de baneo sin fecha y dejarla enganchada
   por la regla de «exactamente una». El invariante 1 de la especificación exige que la alerta de
   baneo lleve **siempre** la fecha de expiración, así que la publicación conjunta lo vuelve
   estructuralmente imposible en vez de improbable. El `watch` de solo-estado se conserva para
   `suscribir_estado`, cuyos consumidores (emparejamiento) no necesitan la expiración.

   Si el sidecar declara un baneo sin fecha (`expira_en_ms <= 0`), la clave `expira_en` no
   desaparece del payload: se emite con el texto `expiracion_desconocida`.

2. **`AcuseEnvio`**: el adaptador lo consumía sin elevar la taxonomía al puerto. Esta tarea registra
   los acuses en contadores por `id_conversacion` (espejo del `Productor` Go de `adr-0033`), sin
   cambiar ningún tipo IPC ni subir la versión de cable.

   **El espejo cuenta solo la entrega**, como el `Productor` Go, que observa únicamente la ruta de
   `Receipt` (`entregado` / `leido`). El sidecar emite además `acuse_envio` con estado `enviado` en
   **cada** envío aceptado por el servidor; contar ese acuse igualaría `acusados` a `enviados` para
   todo mensaje que sale, el ratio quedaría clavado en 1,0 y la condición de caída (AC-9) sería
   inalcanzable en producción precisamente en el caso que la motiva: un contacto que bloquea y cuyos
   acuses de entrega cesan. El acuse `enviado` tampoco saca el mensaje de vuelo; un estado terminal
   que no es entrega (`fallido`) lo saca sin contarlo.

Ninguno de los dos cruces reabre el puerto de canal ni viola `crates/hexcell-canal-whatsmeow/tests/privacidad.rs`.

## AC-4 es una condición temporal, no de transición

El sidecar emite `reconectando` **una sola vez por desconexión** (`Supervisor.procesarDesconexion`
en `sidecar/internal/canal/reconexion.go`) y después solo `activa` cuando reconecta. Un evaluador
invocado únicamente al cambiar el estado vería `Reconectando` en el instante cero —duración cero,
silencio— y no volvería a invocarse nunca: la ventana configurada jamás se cruzaría y la alerta
sería inalcanzable en el escenario que el criterio nombra.

Por eso el observador del núcleo **reevalúa el último estado observado también por reloj**, con el
mismo tick de 60 s que ya usan las métricas, además de reaccionar a los cambios del `watch`.
Reevaluar una condición ya alertada no produce una segunda notificación: la regla de «exactamente
una» vive en el evaluador, no en la frecuencia de las llamadas.

## Exactamente una notificación por ocurrencia

Cada criterio de aceptación exige «exactamente una notificación» por ocurrencia. Un canal `watch`
re-entrega en cada observación y el tick de 60 s re-evalúa indefinidamente, así que el evaluador
mantiene estado por condición: si la condición persiste, la segunda evaluación no produce nada.
Cuando la condición se despeja, el estado se reinicia.

## Umbrales como parámetros

Ningún umbral (ventana de reconexión, suelo de balance, límite de tasa de descartes, límite de
caída de ratio de acuses) se afirma como correcto. Todos son parámetros de configuración con
valores de respaldo para que la célula arranque sin configuración. La calibración definitiva se
hará contra datos reales de producción.

## Lo que esto NO hace

- **No reduce la probabilidad de baneo.** El riesgo del canal propio es estructural: Meta detecta
  la biblioteca por su huella de protocolo. El alertado acorta el tiempo de reacción, no evita el
  baneo. No se introduce ningún folclore de bulk-sender (jitter, warm-up), proxies, VPNs ni
  rotación de IP.
- **No observa cuántos usuarios han reportado el número.** Esa señal no existe por ninguna vía.
- **No implementa la condición de bucle de reinicios.** Queda para una tarea futura.

## Prioridad máxima del baneo: lo que se exige y lo que se entrega

**Lo que exige el invariante.** La especificación de esta tarea pide que la alerta de baneo temporal
se trate como **prioridad máxima** y que ninguna otra condición comparta su comportamiento de canal.

**Lo que se entrega hoy.** La alerta de baneo se distingue **únicamente por su propio código de
alerta** (`baneo_temporal_detectado`). El puerto `SumideroDeNotificaciones` no transporta severidad,
ni política de reintento, ni ventana de deduplicación: es una omisión deliberada de HEX-077-a,
registrada como **D-09** en `docs/bitacora-de-descartes.md` («escribir esas firmas antes de que
exista un consumidor real»). Esta tarea **no** ensancha el puerto —su contrato lo prohíbe
expresamente— así que hoy no existe ningún mecanismo por el que el sumidero de Telegram dé a la
alerta de baneo un trato de canal distinto del de las otras seis. La limitación se acepta a
sabiendas por decisión humana del **2026-09-14** y se deja escrita aquí en lugar de quedar implícita
en el código.

**Condición de reapertura.** La severidad se lleva al puerto en una tarea propia cuando exista un
consumidor real que la necesite para **comportarse distinto**: por ejemplo, un sumidero que deba
enrutar la alerta de baneo a un destino aparte, repetirla hasta acuse humano, o saltarse una
ventana de silencio que sí se aplique a las demás. Mientras el único consumidor sea un chat de
Telegram donde las siete alertas se leen igual, el código de alerta basta y añadir el campo sería
exactamente el descarte D-09.

## Alternativas descartadas

- **Leer el evento discreto `VeredictoDeReserva::Rechazada` para AC-6**: más preciso que la
  instantánea, pero obligaría a pasar un `Arc` a través de `ProcesadorDeInferencia::nuevo` y
  ripple en todos los sitios de construcción. La instantánea basta y se evalúa en el tick de 60 s
  ya existente.
- **Agregar un tipo IPC nuevo para los acuses por contacto**: violaría `adr-0033` (protocolo
  estable en versión 6, `adr-0032`). El espejo en Rust resuelve la unión `id_mensaje →
  id_conversacion` del mismo modo que el `Productor` Go resuelve `id_correlacion →
  id_conversacion`.

## Extiende

- `adr-0024-metricas-internas-de-operacion.md` (la instantánea como fuente de señales)
- `adr-0033-metricas-de-canal-propio-en-el-sidecar.md` (el Productor Go como espejo)
