# ADR-0014 — Canal propio permanente y canal oficial pospuesto a segunda etapa

* **Estado:** Vigente desde el 2026-07-28.
* **Supersede a:** `adr-0008-estrategia-canal-dos-fases.md` (estrategia de canal en dos fases con
  compuerta en el tercer cliente).
* **Etapa:** A-1.
* **Requisitos tocados:** FR-01, FR-12, NFR-01.

---

## Contexto

`adr-0008` fijaba una estrategia de dos fases con una compuerta explícita: la Fase A validaba el
negocio sobre canal no oficial con exactamente dos células piloto y **el tercer cliente la cerraba**,
abriendo la Fase B sobre Meta Cloud API. La regla que sostenía el conjunto era "no se comercializa
sobre canal no oficial": el riesgo frente a los Términos de Servicio de WhatsApp se aceptaba
**temporalmente y solo como riesgo de validación**.

Dos hechos revisados el 2026-07-28 invalidan la premisa económica de esa compuerta.

**1. El coste de gestión comercial por cliente.** Llevar a cada microempresa al canal oficial no es
una tarea de integración: exige convencer a un negocio de tres empleados de que monte una WABA
(cuenta de WhatsApp Business), verifique su empresa ante Meta y delegue las gestiones en el
proveedor, que acaba haciéndolas por ella. Ese esfuerzo no se paga en servidores ni en líneas de
código: **recae íntegramente sobre el tiempo del fundador, que es el recurso más escaso del
proyecto**. No aparece en ningún diagrama de arquitectura, en ninguna estimación de memoria y en
ningún presupuesto de infraestructura, y por eso mismo se venía subestimando. Multiplicado por cada
alta, es el factor que decide si el producto escala o se ahoga en trámites.

**2. El coste de transporte del canal oficial ha dejado de ser cero.** El 2026-07-01 Meta anunció
que **desde el 2026-10-01 cobrará también los mensajes de servicio**, es decir, las respuestas
enviadas dentro de la ventana de 24 horas, con las tarifas publicables hasta el 2026-09-01. Esto
invalida directamente la decisión registrada en `docs/STATUS.md` el 2026-07-27, según la cual, al
nacer el canal oficial como canal **solo-respuesta**, su transporte costaba aproximadamente cero.
El producto es solo-respuesta por diseño: precisamente el tráfico que iba a ser gratuito es el que
pasa a facturarse. *Estado de la evidencia: confirmado por múltiples BSPs (proveedores de soluciones
de negocio), **todavía no reflejado en la página oficial de precios de Meta**. Se documenta con ese
matiz y no como hecho cerrado; si Meta lo desmintiera, este motivo decaería, pero el motivo 1 se
sostiene solo.*

Un tercer punto se registra como pendiente conocido y no como bloqueo: **la pérdida de la bandeja de
entrada del móvil no se considera un problema, al menos por ahora**, por decisión explícita del
dueño.

Con la premisa económica caída, mantener la compuerta significaría frenar el crecimiento en el
tercer cliente para financiar una migración que ahora cuesta tiempo de fundador **y** dinero de
transporte, a cambio de eliminar un riesgo que el propio proyecto ya sabe cómo acotar.

## Decisión

1. **whatsmeow pasa a ser el canal propio de producción, permanente y por defecto**, con **clientes
   de pago reales** encima. Deja de ser un adaptador temporal de validación. No hay límite de dos
   pilotos ni fecha de caducidad.
2. **El canal oficial (Meta Cloud API) se pospone a una segunda etapa y se incorporará como canal
   adicional que convive** con el propio. Se activará cuando aparezca un cliente que lo justifique
   —típicamente una empresa medianamente grande capaz de asumir el alta y el coste de transporte—,
   **no en una fecha ni al alcanzar un número de clientes**.
3. **Queda derogada la regla "no se comercializa sobre canal no oficial".** Es la inversión más
   importante de este cambio y se deja escrita como tal: el proyecto vende sobre canal propio.
4. **Queda derogada la compuerta del tercer cliente.** El tercer cliente ya no cierra nada; se suma
   a la cartera como cualquier otro. Lo que la sustituye está más abajo.

Las etiquetas **Fase A** y **Fase B** se conservan, junto con los nombres de archivo del plan; lo que
cambia es su significado. "Fase A" designa ahora el **canal propio en producción**; "Fase B", el
**canal oficial adicional**. El sidecar de whatsmeow es permanente en toda célula sobre canal propio.

## Consecuencias

### Positivas

* **Desaparece el trabajo de alta más caro.** Cada cliente nuevo se activa emparejando un número que
  ya existe, sin WABA, sin verificación de empresa ante Meta y sin trámites delegados. El tiempo del
  fundador deja de ser el cuello de botella del crecimiento.
* **El coste de transporte por conversación se mantiene en cero** cuando el del canal oficial deja
  de serlo el 2026-10-01. Sobre márgenes de microempresa, la diferencia es material.
* **El cliente conserva su bandeja de entrada en el móvil** y con ella su capacidad de intervenir a
  mano, sin que HexCell tenga que construir una interfaz de intervención humana para operar.
* **Un solo camino de producción, ejercitado a diario.** La ruta que se prueba es la que se vende.
* **El puerto de canal (FR-12) conserva íntegro su valor** y gana una razón adicional: ya no es solo
  la frontera de una migración futura, sino la frontera que permitirá que dos canales convivan en la
  misma base de código.

### Negativas

Se enuncian sin atenuación, porque una decisión cuyo coste se maquilla no se puede revisar después.

* **Se asume de forma permanente la violación de los Términos de Servicio de WhatsApp, y ahora con
  clientes de pago encima.** Lo que `adr-0008` aceptaba como riesgo temporal de validación pasa a ser
  la postura estable del producto. No hay fecha en la que este riesgo se extinga.
* **El riesgo deja de ser puntual y pasa a ser correlacionado de cartera.** Mientras hubiera dos
  pilotos, un baneo era un incidente aislado. Con N clientes sobre la misma biblioteca, **una ola de
  baneos o una rotura de protocolo golpea a todos a la vez**, y la reparación depende de un
  mantenedor voluntario único (ver *Evidencia*). No hay diversificación posible dentro del canal
  propio: el modo de fallo es común por construcción.
* **El sidecar y su presupuesto de memoria dejan de ser transitorios.** El objetivo de NFR-01 para el
  canal oficial (< 50 MB por célula, sin sidecar) deja de ser el estado final al que tiende el
  sistema: el estado normal es ≤ 80 MB con dos contenedores por célula. El coste de memoria del
  sidecar se paga indefinidamente sobre un servidor de 8 GB, y eso fija el techo físico de células
  por máquina.
* **El respaldo del `sqlstore` del sidecar cambia de naturaleza: pasa de ser respaldo de datos a ser
  respaldo de disponibilidad del canal.** Ya no protege un piloto reemplazable, sino la continuidad
  del servicio que un cliente paga. Su frecuencia, su verificación y su procedimiento de restauración
  suben de categoría en consecuencia, con el criterio de éxito ya vigente: **la restauración solo
  vale si el bot reconecta y responde**.
* **Desaparece un mecanismo de disciplina.** La compuerta no solo ordenaba el trabajo técnico:
  obligaba a parar y mirar. Sin ella, nada frena el crecimiento por sí mismo, y una cartera que crece
  sobre un riesgo correlacionado sin freno declarado es exactamente el escenario que peor termina. El
  freno hay que reponerlo de forma explícita.

## Qué sustituye a la compuerta derogada

La compuerta se sustituye por **compuertas de riesgo**, no por confianza. Ambas son decisiones de
disciplina de cartera y se detallan, con las demás medidas, en `adr-0015`:

* **Techo duro de cartera** mientras el canal propio sea el único canal en producción: un número
  máximo de células activas por encima del cual no se dan altas.
* **Umbral de incidentes que congela altas:** si la tasa de baneos supera un valor declarado, no se
  activa ninguna célula nueva hasta analizar la causa.

**Los valores numéricos de ambos umbrales quedan declarados como decisión de negocio pendiente**, en
`docs/STATUS.md`, y deben fijarse por escrito **antes del alta del primer cliente de pago**. Un techo
sin número no es un techo.

## Alternativas consideradas y descartadas

### A. Mantener la compuerta del tercer cliente

Conserva la promesa de eliminar el riesgo de ToS antes de vender y mantiene el crecimiento acotado
por construcción. Se descarta porque su premisa económica ya no se sostiene: la migración cuesta
tiempo de fundador por cada cliente **y**, desde el 2026-10-01, dinero de transporte por cada
respuesta. La compuerta pararía el negocio en su tercer cliente para pagar dos veces por un canal que
sirve el mismo producto. Su función de disciplina, que sí era valiosa, se recupera mediante el techo
de cartera y el umbral de incidentes.

### B. Migrar al canal oficial desde el principio

Elimina el riesgo de ToS y el riesgo correlacionado de cartera de raíz. Se descarta por los mismos
dos motivos económicos del contexto, agravados: pagarlos desde el cliente cero, antes de tener
ninguna evidencia de que el producto se vende.

**Hallazgo registrado durante esta evaluación — el modo coexistencia de Meta.** Existe un modo
oficial de coexistencia
([documentación de Meta](https://developers.facebook.com/docs/whatsapp/embedded-signup/custom-flows/onboarding-business-app-users/))
en el que **un mismo número funciona a la vez en la app de WhatsApp Business del móvil y en la Cloud
API**: sincroniza 180 días de historial y los contactos, y el integrador recibe por webhook
(`smb_message_echoes`) lo que el dueño del negocio responde a mano desde su propia app. Requiere
Embedded Signup a través de un Solution Partner o Tech Provider; **no hay ruta de Cloud API directa**.
Limitaciones conocidas: 20 mensajes por segundo fijos, sin grupos, sin mensajes efímeros, sin vista
única, sin ubicación en vivo, sin listas de difusión y sin catálogo ni pedidos por API.

Este hallazgo **desmonta uno de los argumentos históricos a favor del canal propio**: es falso que
adoptar el canal oficial obligue al cliente a perder la bandeja de entrada de su teléfono. Y resuelve
de paso el pendiente de la interfaz de intervención humana registrado en `docs/STATUS.md`, porque la
intervención a mano vuelve a ocurrir en la app del propio dueño y el sistema se entera de ella.

**Aun así no cambia la decisión, y conviene ser honesto sobre por qué:** el argumento de la bandeja
móvil era un argumento de comodidad, no de coste. Los dos motivos económicos —el tiempo de fundador
por alta y el cobro de mensajes de servicio desde el 2026-10-01— **se sostienen solos**, y la
coexistencia no alivia ninguno: sigue exigiendo Embedded Signup con WABA verificada (mismo trámite,
misma persona haciéndolo) y sigue facturando el transporte. Lo que sí queda escrito es el mandato:
**la segunda etapa debe evaluar el modo coexistencia como su opción preferente**, por delante de una
migración limpia a Cloud API, y contrastar sus limitaciones contra el alcance real del producto antes
de comprometerse.

## Evidencia que respalda el riesgo asumido

El riesgo de baneo es **en buena medida estructural**: Meta detecta la biblioteca por su huella de
protocolo, y ninguna medida de comportamiento lo elimina. Los tres incidentes de referencia en
`tulir/whatsmeow` documentan baneos y avisos de *"unauthorized tools"* sobre cuentas de **bajo volumen
y solo-respuesta**, es decir, sobre el mismo perfil de uso que tiene este producto:

* **Issue #810** y **#807** (mayo de 2025, concentrados en Brasil): oleada de baneos y avisos de
  herramientas no autorizadas.
* **Issue #989** (noviembre de 2025): suspensiones de 24 horas con código de enforcement
  `BULK_MESSAGING` **pese a enviar pocos mensajes y con pausas de 5 segundos entre ellos**.

Ninguno de los tres identificó un patrón accionable y los tres se cerraron como *not planned*. Meta
banea del orden de 2 millones de cuentas al mes, alrededor del 75 % por decisión automática, y **puede
hacerlo sin aviso previo**.

A esto se suma el **riesgo de mantenimiento**: whatsmeow tiene **bus factor 1** —prácticamente la
totalidad de sus ~1.620 commits son de un único mantenedor, con actividad casi diaria en junio y
julio de 2026—, y su patrón de rotura recurrente es `Client outdated (405)` (issues #415 y #1031)
cuando WhatsApp sube la versión mínima de cliente. El arreglo es siempre actualizar, pero **no se
puede comprometer ningún tiempo de recuperación que dependa de un tercero voluntario**.

Consecuencia de diseño que hereda `adr-0015`: **el baneo se trata como evento esperado, no como
fallo**, y las medidas que reducen el daño valen más que las que reducen la probabilidad.

## Referencias

* Supersede: `adr-0008-estrategia-canal-dos-fases.md`.
* Continúa vigente: `adr-0009-whatsmeow-adaptador-fase-a.md` (elección de biblioteca),
  `adr-0010-puerto-de-canal.md` (FR-12), `adr-0011-whatsmeow-sidecar-e-ipc.md` (sidecar e IPC).
* Desarrolla las compuertas de riesgo: `adr-0015-politica-de-convivencia-con-el-baneo.md`.
* `adr-0013-entrada-publica-fase-b.md` deja de ser una decisión próxima y pasa a depender de la
  activación de la segunda etapa por demanda de un cliente.
* `docs/PRD.md`, sección "Estrategia de Canal por Fases" (FR-01, FR-12, NFR-01).
* `docs/STATUS.md`: invalida el "transporte de la Fase B cuesta ≈ 0" de la entrada del 2026-07-27
  sobre el canal solo-respuesta; el resto de esa entrada sigue vigente. Registra los umbrales del
  techo de cartera y del congelado de altas como decisión de negocio pendiente.
* `docs/plan/README.md` y las etapas A-1, A-3, A-6 y A-7.
