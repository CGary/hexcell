# Fase A · Etapa 3 — Adaptador whatsmeow: sidecar Go y puerto de canal

**Duración relativa:** Larga.

---

## Objetivo

El núcleo de la etapa A-2 sabe conversar, pero habla con un adaptador simulado. Esta etapa lo conecta
por primera vez a WhatsApp de verdad, y lo hace por la vía no oficial: la biblioteca **whatsmeow**,
que implementa el protocolo de WhatsApp Web sobre un **websocket saliente**. No hay webhook, no hay
IP pública, no hay certificado que emitir ni puerto que abrir en el router. El servidor local se
conecta hacia fuera.

whatsmeow es una biblioteca Go y no existe equivalente maduro en Rust, de modo que el adaptador vive
en un **proceso separado** —un sidecar— que acompaña al núcleo dentro de la misma célula. Los dos
contenedores comparten red local y volumen, y se comunican por IPC sobre un socket local. Del lado
Rust, ese IPC se envuelve en una implementación del trait `ChannelAdapter`, de modo que el núcleo
sigue sin enterarse de que existe whatsmeow.

Hay dos problemas que esta etapa tiene que resolver bien o el piloto no sobrevive a su primera
semana. El primero es la **persistencia de sesión**: whatsmeow se empareja escaneando un QR o
introduciendo un código, y si las credenciales no se persisten, cada reinicio del contenedor exige que
alguien vuelva a coger el teléfono del cliente. Es inaceptable en operación. El segundo es el
**calentamiento anti-ban**: un número nuevo que empieza a emitir mensajes automatizados a ritmo de
máquina es un número que WhatsApp desactiva. La disciplina de calentamiento no es una optimización,
es la condición de supervivencia del canal.

---

## Alcance

### Qué entra

* Sidecar Go con la sesión whatsmeow: conexión, mantenimiento del websocket y traducción de los
  eventos del protocolo al formato canónico del puerto de canal.
* **Emparejamiento** por código QR y por *pairing code* (código de ocho caracteres introducido en el
  teléfono), con una interfaz de operación que no obligue a exponer el terminal al cliente.
* **Persistencia de sesión** en el `sqlstore` de whatsmeow sobre el volumen de la célula, de modo que
  un reinicio del contenedor reanude la sesión sin re-escanear el QR.
* **Reconexión automática con retroceso exponencial** ante caídas del websocket, con límite superior
  de espera y registro de cada intento.
* Detección de **desvinculación del dispositivo** (logout desde el teléfono, expiración de sesión o
  ban) y señalización explícita al núcleo, distinguiendo entre una desconexión transitoria y una que
  exige re-emparejamiento humano.
* **Outbox durable en el sidecar (entrega *at-least-once*).** La **primera acción** del sidecar tras
  recibir un evento del websocket —antes de traducirlo, antes de entregarlo al núcleo, antes de
  cualquier otra cosa— es persistirlo con `fsync` en un outbox durable: una tabla SQLite sobre el
  volumen compartido. La entrega al núcleo se marca procesada **solo** con la confirmación explícita
  del núcleo. Al arrancar cualquiera de los dos procesos, todo lo no confirmado se reentrega. El
  núcleo deduplica por el identificador de deduplicación de FR-12, de modo que la reentrega sea
  inofensiva.

  > **Nota honesta sobre el límite de esta garantía.** El acuse de protocolo hacia WhatsApp lo emite
  > la biblioteca de forma automática al recibir el mensaje y **no se puede diferir** hasta que
  > nuestro outbox haya hecho `fsync`. Existe por tanto una ventana real —de milisegundos— en la que
  > un corte de corriente entre el acuse de protocolo y el `fsync` pierde el evento sin que WhatsApp
  > lo reenvíe. El outbox no elimina esa ventana: la reduce de "todo lo que hubiera en memoria" a
  > "el evento en vuelo". Se documenta porque prometer entrega exactamente-una-vez sobre este canal
  > sería mentir.
* **Protocolo IPC local** entre sidecar y núcleo: definición del formato de mensaje, del socket, de la
  semántica de reconexión y de la política de reintento y confirmación, apoyada en el outbox durable,
  de modo que ni un evento entrante se pierda si uno de los dos procesos se reinicia antes que el
  otro. El protocolo expone además el **estado de la sesión whatsmeow** (activa / reconectando /
  desvinculada); el núcleo incorpora ese estado a su `GET /health/ready` —contrato declarado en la
  etapa A-2, donde el trait `ChannelAdapter` ya reserva el campo y el simulado lo reportaba siempre
  activo—, de modo que la célula no se declara lista mientras la sesión del canal no esté activa.
* **Emisión de eventos de alerta** hacia el sistema de notificaciones push que construye la etapa
  A-6: sesión desvinculada, sidecar sin reconectar durante más de 5 minutos, bucle de reinicios y
  descarte de un envío no solicitado por el componente de envío. El sidecar produce las señales
  —incluido el contador expuesto de envíos rechazados por violar el invariante anti-ban, donde todo
  descarte anómalo es alerta—; la etapa A-6 las entrega.
* Implementación del trait `ChannelAdapter` en Rust sobre ese IPC, incluido el **sub-trait de ciclo de
  vida de sesión** (emparejamiento y persistencia de credenciales), que en la Fase B quedará sin
  implementar porque la Cloud API no lo necesita.
* Mapeo del **JID** de whatsmeow al identificador interno de conversación, dentro del adaptador. El
  JID no cruza la frontera del puerto.
* Traducción de los acuses del protocolo a los acuses normalizados `sent`/`delivered`/`read`/`failed`.
* **Calentamiento anti-ban** como política implementada, no como recomendación escrita: rampa de
  volumen diario configurable, retardos humanizados entre recepción y respuesta con dispersión
  aleatoria, y **política estricta de solo responder** — el bot nunca inicia una conversación con un
  desconocido.
* Procedimiento documentado de actualización de la dependencia whatsmeow ante una rotura de
  protocolo, con el *bump* de versión como operación de un solo paso.

### Qué NO entra

* Cualquier funcionalidad de envío masivo, difusión o contacto en frío. Es incompatible con la
  política anti-ban y con la naturaleza del producto.
* El adaptador de Cloud API: etapa B-1.
* El alta de las células piloto reales: etapa A-7. Aquí se prueba con un número de laboratorio propio,
  distinto de los números de los pilotos.
* Control de admisión y presupuesto: etapa A-4.

### Requisitos del PRD cubiertos

* **FR-01** — implementación de la variante de Fase A: recepción de mensajes por la sesión whatsmeow
  del sidecar, entregados al núcleo por el puerto de canal.
* **FR-12** — primera implementación completa del puerto, incluido el sub-trait de ciclo de vida de
  sesión.

---

## Entregables

* Binario del sidecar Go, con la dependencia de whatsmeow fijada a versión explícita.
* Implementación `WhatsmeowAdapter` del trait `ChannelAdapter` en el workspace Rust.
* Especificación escrita del protocolo IPC, versionada en el repositorio, incluida la semántica de
  confirmación y de reentrega.
* Esquema y implementación del **outbox durable** del sidecar, con su política de retención y purga.
* Runbook de **re-emparejamiento por `PairPhone()`** (ver tarea 13), como procedimiento de
  recuperación de primera clase.
* `docs/adr/adr-0011-whatsmeow-sidecar-e-ipc.md` con el porqué del proceso separado, la elección del
  mecanismo IPC y el diseño de persistencia de sesión, con la numeración que fija el
  [índice de ADR](../adr/README.md). Es distinto de `adr-0009`, que registra la **elección de la
  biblioteca**; este registra la **arquitectura de sidecar** que esa elección impone.
* `docs/runbook-canal-fase-a.md`: emparejamiento de una célula, diagnóstico de desconexión,
  re-emparejamiento y procedimiento de actualización ante rotura de protocolo.
* Política de calentamiento anti-ban implementada y con sus parámetros documentados.
* Pruebas: del adaptador contra un sidecar simulado, y del sidecar contra un número de laboratorio.

---

## Tareas

1. **Especificar el protocolo IPC** (1 día). Formato de mensaje, transporte (socket de dominio Unix
   sobre el volumen compartido), semántica de confirmación de entrega y comportamiento ante
   reconexión de cualquiera de los dos extremos. Se escribe antes de implementar nada.
2. **Construir el esqueleto del sidecar y la conexión whatsmeow** (1,5 días). Arranque, conexión del
   websocket, recepción de eventos crudos y registro estructurado.
3. **Implementar el outbox durable** (1,5 días). Tabla SQLite sobre el volumen compartido, escritura
   con `fsync` como primera acción tras recibir del websocket, marcado de procesado solo contra
   confirmación del núcleo, reentrega de lo no confirmado al arrancar, y política de retención y
   purga de lo ya confirmado. Se implementa **antes** que la traducción de eventos: si el outbox
   llega después, el orden "persistir primero" se convierte en una intención en lugar de una
   propiedad del código.
4. **Implementar el emparejamiento por QR y por código** (1,5 días). Generación y presentación del QR,
   solicitud del *pairing code*, y una superficie de operación que permita completar el alta sin
   acceso al terminal del servidor.
5. **Implementar la persistencia de sesión en `sqlstore`** (1 día). Almacenamiento de credenciales
   sobre el volumen de la célula, con los permisos del modelo de aislamiento, y reanudación
   automática al arrancar.
6. **Implementar la reconexión con retroceso exponencial** (1 día). Reintentos con espera creciente y
   techo, distinción entre error transitorio y sesión inválida, y registro de cada transición.
7. **Implementar la detección de desvinculación y re-emparejamiento** (1 día). Señalización explícita
   al núcleo cuando la sesión requiere intervención humana, con un estado observable desde la CLI.
8. **Traducir eventos y acuses al formato canónico** (1,5 días). Mensaje entrante a evento canónico
   con su identificador de deduplicación; acuses del protocolo a `sent`/`delivered`/`read`/`failed`.
9. **Implementar el mapeo JID → identificador interno** (1 día). Dentro del adaptador, con la garantía
   verificable de que el JID no cruza la frontera del puerto.
10. **Implementar `WhatsmeowAdapter` en Rust** (1,5 días). Cliente del IPC envuelto en el trait,
   incluido el sub-trait de ciclo de vida de sesión, con manejo de la caída del sidecar.
11. **Implementar la política de calentamiento anti-ban** (1,5 días). Rampa de volumen diario, retardo
    humanizado con dispersión aleatoria entre recepción y respuesta, y bloqueo por diseño de
    cualquier envío no solicitado. La política es parametrizable pero su desactivación no debe ser
    posible por configuración accidental.
12. **Probar contra un número de laboratorio** (1,5 días). Emparejamiento, conversación real, reinicio
    de contenedores con reanudación sin re-escaneo, corte de red con reconexión, y desvinculación
    forzada desde el teléfono.
13. **Escribir y ensayar el runbook de re-emparejamiento por `PairPhone()`** (1 día). El
    re-emparejamiento no es un último recurso improvisado sino un **procedimiento de recuperación de
    primera clase**: `PairPhone()` genera un código de ocho caracteres que el piloto teclea en su
    propio teléfono, de modo que **no hace falta tener su teléfono en la mano** ni desplazarse. Es la
    segunda capa de defensa cuando el respaldo del `sqlstore` no basta o llega desfasado. El
    procedimiento se **ensaya una vez con piloto-01 antes de dar de alta a piloto-02**: un
    procedimiento de recuperación nunca ejecutado no es un procedimiento, es una suposición.
14. **Redactar el runbook del canal y el procedimiento de actualización** (0,5 días). Incluye el paso
    a paso ante una rotura de protocolo: comprobar el proyecto de la biblioteca, subir la versión,
    reconstruir la imagen del sidecar y redesplegar.

---

## Criterios de aceptación

* Una célula recién creada se empareja con un número de WhatsApp mediante QR o código, y a partir de
  ese momento recibe y responde mensajes reales.
* **Reiniciar ambos contenedores de la célula reanuda la sesión sin re-escanear el QR.**
* **Todo evento recibido del protocolo está escrito en el outbox con `fsync` antes de cualquier otra
  acción**, verificado por inspección del orden de operaciones y por una prueba que interrumpe el
  proceso inmediatamente después de la recepción.
* **Tras un reinicio desacompasado de ambos procesos, en cualquiera de los dos órdenes y en cualquier
  punto del ciclo de entrega: cero eventos perdidos y cero eventos procesados por duplicado.** Es el
  criterio que sustituye al ambiguo "ningún mensaje acusado se pierde", que no decía qué medir ni
  cómo comprobarlo.
* El re-emparejamiento por `PairPhone()` se ha ejecutado con éxito al menos una vez sobre una célula
  real, con el código tecleado por el usuario en su propio teléfono y sin acceso físico al mismo.
* Un corte de red de varios minutos se recupera automáticamente por reconexión con retroceso, sin
  intervención manual y sin pérdida de eventos ya confirmados.
* Una desvinculación forzada desde el teléfono se detecta, se señaliza al núcleo y queda visible como
  estado consultable; no se disfraza de desconexión transitoria ni se reintenta indefinidamente.
* El identificador JID de whatsmeow **no aparece** en ninguna estructura del núcleo ni en
  `sessions.db`; solo vive dentro del adaptador.
* Los acuses del protocolo se reflejan en el núcleo exclusivamente como
  `sent`/`delivered`/`read`/`failed`.
* El bot **no emite ningún mensaje que no sea respuesta a un mensaje entrante**: no se verifica ya
  solo por inspección puntual del registro, sino como **invariante estructural continuo**. Todo
  envío saliente debe corresponder a una conversación entrante dentro de la ventana de servicio; el
  componente de envío **rechaza y registra, con un contador expuesto,** cualquier intento que lo
  viole, en producción y no solo en laboratorio —el riesgo de ban no puede depender de una
  inspección manual puntual—.
* Una prueba intenta deliberadamente un envío no solicitado y verifica que el componente de envío lo
  bloquea y que el intento queda registrado (contador incrementado y entrada en el registro).
* Los retardos de respuesta observados están dentro de la ventana humanizada configurada y presentan
  dispersión, no un valor constante.
* Actualizar la versión de whatsmeow y reconstruir la imagen del sidecar es una operación que no
  requiere tocar el núcleo Rust ni el protocolo IPC.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| **Ban permanente del número** por parte de WhatsApp. | Alto en la célula afectada. | Números nuevos y dedicados por célula, nunca el número principal de un negocio; calentamiento con rampa de volumen, retardos humanizados y política de solo responder. El coste de un ban se limita a un número desechable. |
| **Rotura del protocolo** por un cambio de WhatsApp. | Alto: el canal queda inoperativo hasta que la comunidad publique el arreglo. | Dependencia fijada a versión explícita y aislada en el sidecar, de modo que el arreglo sea un *bump* de una línea. Precedente: [la rotura de abril de 2026](https://github.com/lharries/whatsapp-mcp/issues/216) se resolvió en días, frente al [incidente equivalente en Baileys](https://github.com/WhiskeySockets/Baileys/issues/2488). Con los pilotos se pacta expresamente la posibilidad de semanas de silencio (etapa A-7). |
| Pérdida de las credenciales de sesión y re-emparejamiento forzoso. | Medio: sin una vía de recuperación acordada, obliga a coordinar con el piloto-02 en el peor momento. | **Dos capas.** Capa 1: el `sqlstore` entra en el respaldo de la etapa A-2 como tercera base, copiado por el propio sidecar vía `VACUUM INTO` sobre orden IPC y con frecuencia alta —esta etapa **expone la operación IPC** que lo hace posible, no la da por hecha—. Capa 2: re-emparejamiento por `PairPhone()`, con código de ocho caracteres que el piloto teclea en su propio teléfono, ensayado antes del alta de piloto-02. |
| Un fallo de corriente entre el acuse de protocolo y el `fsync` del outbox. | Bajo, pero real e imposible de eliminar: el acuse hacia WhatsApp es automático y no se puede diferir. | Se documenta explícitamente en el alcance en lugar de prometer entrega exactamente-una-vez. El outbox reduce la ventana de pérdida a milisegundos, de "todo lo que hubiera en memoria" a "el evento en vuelo". |
| El JID se filtra al núcleo por comodidad de depuración. | Alto: rompe la frontera de migración y contamina datos históricos. | Criterio de aceptación explícito y prueba automatizada. |
| Un fallo del IPC pierde eventos entrantes silenciosamente. | Alto: mensajes de clientes finales que nunca se responden, sin rastro. | Outbox durable con `fsync` como primera acción y confirmación explícita del núcleo; semántica de reentrega especificada por escrito antes de implementar; y prueba de reinicio desacompasado de ambos procesos en ambos órdenes. |
| La política anti-ban se relaja bajo la presión de "responder más rápido". | Muy alto: pérdida del número y del piloto. | Los parámetros son configurables, pero desactivar la política no es una opción de configuración; queda registrado en `adr-0011` como decisión, no como ajuste. |
| Violación de los Términos de Servicio de WhatsApp. | Asumido conscientemente. | Riesgo aceptado solo como validación, con dos pilotos controlados y sin comercialización. La Fase B lo elimina. |

---

## Dependencias

* **De otras etapas:** etapa A-2 completa. El adaptador sustituye al simulado en un núcleo que ya
  funciona, la deduplicación por identificador de FR-12 que hace inofensiva la reentrega del outbox ya
  existe, y el procedimiento de respaldo al que esta etapa aporta la operación IPC de copia del
  `sqlstore` ya está construido.
* **Externas:** un número de WhatsApp de laboratorio, distinto de los números de los pilotos, y un
  teléfono para el emparejamiento y las pruebas de desvinculación.
* **Decisiones de producto pendientes:** ninguna bloquea esta etapa.
