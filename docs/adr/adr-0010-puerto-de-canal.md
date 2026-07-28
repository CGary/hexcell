# ADR-0010 — Puerto de canal `ChannelAdapter` como frontera entre el núcleo y el transporte

* **Estado:** Vigente desde el 2026-07-28.
* **Supersede a:** nada. Formaliza una decisión tomada el 2026-07-26 y registrada hasta ahora solo en
  el PRD (FR-12) y en el índice de ADR.
* **Etapa:** A-1 (declaración), A-2 (lado consumidor), A-3 (primera implementación completa).
* **Requisitos tocados:** FR-01, FR-05, FR-12.

---

## Contexto

El producto tiene que hablar por WhatsApp, y hay dos maneras de hacerlo que no se parecen en nada.
El **canal propio** usa whatsmeow sobre un websocket saliente: sin ventanas de servicio, sin
plantillas aprobadas, con emparejamiento por QR y con credenciales de sesión que hay que persistir.
El **canal oficial** usa la Meta Cloud API sobre webhooks entrantes: con una ventana de servicio de
24 horas que se cierra, con plantillas obligatorias fuera de ella y sin nada que emparejar. Desde
`adr-0014` los dos son permanentes y **conviven**: el canal oficial no sustituye al propio, se suma a
él en células distintas del mismo servidor.

La forma barata de construir esto es la que se descarta aquí: escribir el núcleo contra whatsmeow
porque es lo primero que existe, y ya se verá cómo entra la Cloud API cuando haga falta. Barata
durante seis semanas y ruinosa después, porque el día en que aparezca el primer cliente que justifique
el canal oficial, "añadir un canal" no sería escribir un adaptador sino reescribir el producto sobre
datos conversacionales de clientes reales que ya están en producción.

Hay además una tentación más silenciosa y más cara de deshacer: dejar que el identificador de
transporte —el JID de whatsmeow, el `wa_id` de Meta— se filtre al núcleo "solo para depurar". Un
identificador de transporte persistido en `sessions.db` no es una fealdad estética: convierte
cualquier cambio de canal en una migración de datos históricos de clientes de pago.

## Decisión

1. **El núcleo Rust no conoce ningún transporte de WhatsApp.** Toda integración de canal se
   implementa detrás del trait `ChannelAdapter`, que el núcleo consume sin saber qué hay debajo.
   Añadir un canal es escribir un adaptador; no se toca el dominio.
2. **El puerto es una frontera de coexistencia, no de migración.** Dos adaptadores están vivos a la
   vez, en células distintas del mismo servidor. Esta es la lectura vigente desde `adr-0014` y
   sustituye a la anterior, que lo entendía como el paso de un canal a otro.
3. **El puerto se abstrae hacia el caso más restrictivo, que es la Cloud API**, con una distinción
   que hace viable la convivencia: **el TIPO admite el resultado restrictivo; la POLÍTICA de cada
   adaptador decide si lo produce.** Que `send()` pueda devolver `FueraDeVentana` obliga al núcleo a
   saber reaccionar, pero no obliga al adaptador del canal propio a imponer una ventana de 24 horas
   artificial: ese adaptador nunca produce ese resultado, porque su transporte no lo impone.
4. **`sessions.db` nunca almacena identificadores de transporte crudos.** La regla tiene el alcance
   estrecho que le da el PRD y se enuncia sin ampliarla: prohíbe esa base, no prohíbe que el
   identificador exista. Dentro del adaptador existe por necesidad, y ahí es donde debe quedarse.
5. **El mapeo entre el identificador de transporte y el identificador interno pertenece al
   adaptador.** El núcleo recibe el identificador interno ya traducido y lo trata como **opaco**: no
   lo deriva de ningún dato de transporte, no lo interpreta y no lo invierte. Asignarle al núcleo una
   "traducción estable y reversible" sería responsabilidad duplicada: si el adaptador ya entrega el
   identificador interno, esa traducción del núcleo es la función identidad.
6. **El mapeo persiste en un almacén propio del adaptador, sobre el volumen de la célula y separado
   de las credenciales de sesión del transporte** —separado, por tanto, del `sqlstore` de whatsmeow—.
   El motivo es concreto y no estético: la rama `LoggedOut` con `device_removed` obliga a
   **descartar** el `sqlstore`, porque el dispositivo ya no existe en el servidor de WhatsApp y la
   única salida es el re-emparejamiento. El mapeo tiene que **sobrevivir** a ese re-emparejamiento
   para que cada contacto siga cayendo en el hilo que ya tenía. Guardarlo dentro del `sqlstore` lo
   destruiría exactamente en el único escenario en el que se necesita que aguante.
7. **Ese almacén entra en el respaldo por célula, que pasa de tres bases a cuatro:** `sessions.db`,
   `knowledge_live.db`, el almacén de identidad del adaptador y el `sqlstore` del sidecar. La **lista
   de exclusión (STOP)** de la etapa A-3 vive en ese mismo almacén del adaptador, por la misma razón
   y con la misma consecuencia: un contacto que pidió no recibir nada no puede volver a la lista de
   destinatarios porque alguien haya tenido que re-emparejar la célula.

El puerto normaliza siete elementos —evento entrante canónico, envío tipado, resultado tipado, estado
de la ventana de servicio, identidad de conversación, acuses normalizados y ciclo de vida de sesión
como sub-trait opcional—, enumerados en detalle en `docs/PRD.md` (FR-12), que es la fuente normativa.
Este ADR registra el porqué y las consecuencias, no vuelve a describirlos.

## Consecuencias

### Positivas

* **Sumar un canal es escribir un adaptador.** El coste de la segunda etapa deja de ser una reescritura
  y pasa a ser una implementación acotada, con una batería de tests de contrato ya escrita que el
  adaptador nuevo tiene que pasar sin que se toque una línea del núcleo.
* **Los datos históricos son portables por construcción.** Como `sessions.db` solo contiene
  identificadores internos, mover un cliente de un canal a otro no obliga a migrar su historial.
* **El mapeo tiene un dueño único y verificable.** Una sola pieza traduce, en un solo sitio, y hay un
  criterio de aceptación que comprueba que el identificador de transporte no cruza la frontera. Las
  responsabilidades duplicadas no se detectan con pruebas: se detectan cuando divergen, y para
  entonces ya hay datos escritos por las dos.
* **La continuidad del hilo sobrevive a la recuperación.** El cliente que sufre una desvinculación y
  un re-emparejamiento —el peor momento posible para que además parezca que el bot tiene amnesia—
  recupera sus conversaciones donde estaban.
* **La lista de exclusión sobrevive a la recuperación por el mismo mecanismo**, sin necesidad de una
  decisión de diseño aparte.

### Negativas

Se enuncian sin atenuación, porque una decisión cuyo coste se maquilla no se puede revisar después.

* **Hay una cuarta base que respaldar y restaurar de forma consistente con las otras tres.** No es
  solo un archivo más en un script: es un punto más donde la copia puede quedar desincronizada
  respecto de `sessions.db`. Si el mapeo se restaura de un momento distinto que el historial, aparecen
  hilos huérfanos o contactos apuntando a conversaciones que no son la suya. El procedimiento de
  respaldo tiene que tratar las cuatro copias como un conjunto, y la verificación de integridad tiene
  que cubrirlas todas.
* **La lista de exclusión (STOP) hereda ese riesgo, y es el que más duele.** Un almacén restaurado de
  un momento anterior devuelve a la lista de destinatarios a alguien que pidió la baja. Es una
  violación de la promesa más explícita que el producto hace a un usuario final, y llega por un
  camino —una restauración— en el que nadie está mirando eso.
* **El puerto obliga al núcleo a manejar casos que sobre canal propio no ocurren nunca.** La política
  ante `FueraDeVentana` se diseña, se implementa y se prueba aunque en la Fase A no se dispare jamás.
  Es trabajo real pagado por adelantado a cambio de que la segunda etapa no sea una reescritura.
* **La abstracción se paga en indirección.** Depurar un problema de canal exige cruzar la frontera del
  puerto y leer dos piezas en lugar de una, y la tentación de "mirar el JID desde el núcleo" volverá
  cada vez que haya una incidencia en producción. Por eso la prohibición es criterio de aceptación con
  prueba automatizada y no una convención de estilo.

## Alternativas consideradas y descartadas

### A. Modelar el puerto sobre las libertades de whatsmeow

Es la opción cómoda mientras el canal propio sea el único en producción: enviar lo que sea, a quien
sea, cuando sea, sin ventanas ni plantillas. Se descarta porque un puerto así **no podría albergar
después al adaptador oficial**, que es exactamente lo que FR-12 existe para evitar. La abstracción se
hace hacia el caso restrictivo o no sirve de nada.

### B. Que el núcleo mantenga su propia traducción de identidad

Era el reparto que la etapa A-2 asignaba antes de este ADR. Se descarta por responsabilidad
duplicada: si el adaptador ya entrega el identificador interno, la traducción del núcleo es la
función identidad, y dos piezas que traducen lo mismo acaban divergiendo sin que nadie lo note hasta
que hay datos escritos por ambas.

### C. Guardar el mapeo dentro del `sqlstore` del sidecar

Es el sitio que parece natural, porque "todo lo de whatsmeow vive ahí". Se descarta por la rama
`device_removed`: descartar el `sqlstore` es obligatorio en ese caso, de modo que el mapeo y la lista
STOP se destruirían en el único escenario en el que se necesita que sobrevivan. Queda registrado en
la bitácora de descartes como **D-15**.

### D. Guardar el identificador de transporte en `sessions.db`

Ahorra el almacén separado y simplifica las consultas. Lo prohíbe el PRD, y el motivo es económico
antes que estético: contamina datos históricos de clientes de pago y convierte cualquier cambio de
canal en una migración. Queda registrado en la bitácora como **D-16**.

## Referencias

* `docs/PRD.md`, FR-12 (enumeración normativa de los siete elementos del puerto) y FR-05
  (persistencia dual), sección 6 (Prueba de Recuperación de Sesión, sobre las cuatro bases).
* `adr-0009-whatsmeow-adaptador-fase-a.md` (elección de biblioteca) y
  `adr-0011-whatsmeow-sidecar-e-ipc.md` (arquitectura de sidecar e IPC).
* `adr-0014-canal-propio-permanente.md`: fija la lectura del puerto como **frontera de coexistencia**
  y no de migración.
* `adr-0015-politica-de-convivencia-con-el-baneo.md`: continuidad del hilo tras el re-emparejamiento.
* `docs/plan/fase-a-1-fundaciones.md` (declaración del trait),
  `docs/plan/fase-a-2-nucleo-persistencia.md` (lado consumidor, identificador interno opaco y diseño
  del respaldo de las cuatro bases), `docs/plan/fase-a-3-adaptador-whatsmeow.md` (mapeo JID, almacén
  de identidad, lista STOP y ejecución del respaldo),
  `docs/plan/fase-b-1-canal-oficial.md` (segundo adaptador: si exige tocar el núcleo, la etapa no se
  acepta y este ADR se revisa).
* `docs/bitacora-de-descartes.md`: D-09, D-10, D-15 y D-16.
* `docs/STATUS.md`: dueño y ubicación del mapeo, y respaldo de cuatro bases (2026-07-28).
