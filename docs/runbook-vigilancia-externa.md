# Runbook: vigilante externo de vida del anfitrión (dead-man's switch)

* **Fecha de esta versión:** 2026-09-13.
* **Tarea que lo redacta:** HEX-077-d.
* **Artefactos que ancla:** `deploy/ping_de_vigilancia_externa.sh` (el emisor) y
  `deploy/verificar_ping_de_vigilancia.sh` (el guardia mecánico con prueba de mutación).

---

## Qué es esto

Un **vigilante de vida invertido** (dead-man's switch): un script (`deploy/ping_de_vigilancia_externa.sh`)
que un cron del sistema operativo dispara cada 5 minutos en el anfitrión, y que emite exactamente un
`GET` saliente a una URL de un servicio compatible con healthchecks.io. Mientras el ping llega a
tiempo, el servicio externo permanece en silencio. Si el ping deja de llegar, el servicio detecta la
**ausencia** y dispara la notificación — **desde fuera del servidor**, porque un servidor muerto no
puede reportar por sí mismo que murió.

Esta inversión es el punto entero del diseño: en toda otra alarma de este proyecto, un evento
observado dispara la notificación. Aquí es al revés — el silencio es el evento.

## Qué prueba este vigilante (y solo esto)

Cuando el ping llega puntualmente, lo único que queda demostrado es:

1. El **anfitrión** está encendido, con un kernel vivo y el demonio `cron` corriendo.
2. La entrada de `crontab` está instalada y el script del emisor es ejecutable.
3. Hay **DNS, TLS y ruta de salida (egress)** funcionando desde ese anfitrión hasta el endpoint del
   servicio de vigilancia.

## Qué NO prueba este vigilante — léase esto antes de confiar en la alarma

**Este ping no prueba absolutamente nada sobre ninguna célula.** No prueba que el núcleo Rust esté
vivo, que el sidecar de whatsmeow tenga una sesión activa, que el canal de WhatsApp esté conectado,
ni que un solo mensaje esté fluyendo. Un anfitrión perfectamente sano, con **todas** sus células
caídas, deja este vigilante **en verde y en silencio** — por diseño, no por descuido.

Esto no es una omisión que quedó pendiente: alcanzar la salud de una célula desde un cron del
anfitrión exigiría publicar un puerto de salud al anfitrión o usar `docker exec` contra el contenedor,
y ambas opciones debilitarían el aislamiento por célula que anclan NFR-05 y la tarea 17 de la etapa
A-6 (`docs/plan/fase-a-6-empaquetado-cli.md`). Ver `docs/bitacora-de-descartes.md` (D-51) para el
razonamiento completo de por qué esa vía se descartó junto con las otras alternativas estudiadas.

**Sobre-prometer aquí sería peor que no tener el vigilante.** Un operador que cree que "el anfitrión
respondió" implica "el bot está respondiendo a los clientes" confiará en una señal que no cubre el
caso que más le importa. Esta es la razón por la que este runbook insiste en decirlo en estos
términos explícitos.

## Instalación — UNA entrada de cron por SERVIDOR, nunca por célula

Este vigilante se instala **una sola vez por servidor**, sin importar cuántas células corran en él.
Un cron por célula duplicaría pings idénticos hacia el mismo servicio externo sin agregar ninguna
cobertura, y es exactamente el error que este runbook busca prevenir. Por esa misma razón, la
variable de entorno de esta sección **nunca** entra en `deploy/celula.env.ejemplo` ni en
`deploy/cell.compose.yml`: ese archivo es per-célula, y este vigilante no lo es
(ver `docs/plantilla-celula.md`, sección "Qué queda fuera de esta plantilla").

1. Copiar `deploy/ping_de_vigilancia_externa.sh` a una ruta fija del anfitrión, por ejemplo
   `/opt/hexcell/ping_de_vigilancia_externa.sh`, y darle permiso de ejecución:

   ```bash
   install -m 0755 deploy/ping_de_vigilancia_externa.sh /opt/hexcell/ping_de_vigilancia_externa.sh
   ```

2. Dar de alta una entrada en el **crontab del usuario root** del anfitrión (`sudo crontab -e`) que
   defina la variable de entorno directamente en la línea del crontab, nunca en un archivo aparte
   que pudiera terminar versionado:

   ```cron
   */5 * * * * HEXCELL_URL_PING_VIGILANCIA=https://<servicio-de-vigilancia>/<id-del-chequeo> /opt/hexcell/ping_de_vigilancia_externa.sh
   ```

   La cadencia `*/5` (cada 5 minutos) es el único valor normativo que fija esta tarea. El período de
   gracia y el umbral de alerta son configuración del servicio externo (healthchecks.io o
   equivalente), no de este repositorio, y no se documentan aquí como constantes.

3. La URL real **no entra en git bajo ninguna forma**: ni en un archivo de configuración, ni en
   `deploy/celula.env.ejemplo`, ni en `deploy/cell.compose.yml`, ni en ningún `.env*`. Vive
   exclusivamente en esa línea del crontab del anfitrión, que por definición nunca se versiona.

## Verificación manual posterior a la instalación (obligatoria)

Ningún paso de CI puede demostrar que la entrada de crontab quedó instalada en un anfitrión real:
eso es estado del anfitrión, no del repositorio. Por eso, tras instalar la entrada, hay que
verificarlo a mano una vez:

1. Ejecutar el script manualmente con la misma variable que llevará el crontab:

   ```bash
   HEXCELL_URL_PING_VIGILANCIA=https://<servicio-de-vigilancia>/<id-del-chequeo> \
       /opt/hexcell/ping_de_vigilancia_externa.sh
   echo "código de salida: $?"
   ```

   Debe salir con código `0`.

2. Confirmar en el panel del servicio externo que el chequeo correspondiente pasa a estado "arriba"
   (`up`) tras ese ping manual.

3. Esperar al menos un ciclo de cron (5 minutos) y confirmar que el panel sigue en estado "arriba"
   sin intervención manual, lo que confirma que la entrada de crontab efectivamente se está
   ejecutando.

## Qué hacer si la alarma dispara

Un disparo de esta alarma significa: el anfitrión dejó de emitir el ping durante más del período de
gracia configurado en el servicio externo. Las causas típicas son un apagado o congelamiento del
anfitrión, una caída de red de salida, o que el demonio `cron` dejó de correr. **No** significa,
por sí solo, que una célula específica esté caída — para eso hacen falta las señales de salud
por-célula que están fuera del alcance de este vigilante (ver la sección anterior).
