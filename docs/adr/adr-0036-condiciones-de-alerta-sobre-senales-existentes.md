# ADR-0036: Condiciones de alerta sobre señales existentes

**Fecha:** 2026-09-13  
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
   DENTRO de este crate». Esta tarea lo eleva por un `watch::Receiver<Option<SystemTime>>` separado,
   sin tocar el puerto `ChannelAdapter`.

2. **`AcuseEnvio`**: el adaptador lo consumía sin elevar la taxonomía al puerto. Esta tarea registra
   los acuses en contadores por `id_conversacion` (espejo del `Productor` Go de `adr-0033`), sin
   cambiar ningún tipo IPC ni subir la versión de cable.

Ninguno de los dos cruces reabre el puerto de canal ni viola `crates/hexcell-canal-whatsmeow/tests/privacidad.rs`.

## Exactly-one

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
