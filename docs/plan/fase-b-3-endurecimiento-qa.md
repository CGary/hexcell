# Fase B · Etapa 3 — Endurecimiento, QA y operación comercial

**Duración relativa:** sin estimar. **La Fase B permanece sin planificar hasta que aparezca un
cliente que justifique el canal oficial**; sus etapas se describen en alcance y dependencias, no en
días de trabajo.

---

## Objetivo

Un sistema no cumple un requisito no funcional porque su diseño lo contemple, sino porque alguien lo
mide y demuestra que lo cumple. Esta etapa cierra el plan sometiendo el sistema completo a los
criterios de aceptación de QA que fija el PRD y a los requisitos no funcionales, y dejando instalado
lo necesario para operarlo día a día con clientes de pago.

Las etapas anteriores ya incluyeron pruebas propias, pero eran pruebas de componente ejecutadas en
condiciones controladas. Aquí las pruebas se ejecutan sobre el sistema desplegado, con varias células
reales conviviendo en el mismo servidor, que es la única configuración en la que las mediciones
significan algo. La densidad de células por servidor es, además, el número del que depende
directamente la viabilidad económica del producto: cuántos clientes caben en 8 GB de RAM.

Una diferencia importante frente al plan anterior: **el respaldo y la restauración ya no viven aquí**.
Se adelantaron a la etapa A-2 porque con pilotos reales operando desde la Fase A no podían esperar al
final. Lo que esta etapa hace con ellos es verificarlos a escala comercial, no construirlos.

La etapa incorpora lo que hasta ahora se había pospuesto conscientemente: observabilidad para operar,
una revisión de seguridad de la superficie expuesta —que en la Fase B es mucho mayor, porque hay
puertos entrantes y certificados— y las pruebas de caos.

---

## Alcance

### Qué entra

* Ejecución formal y documentada de los criterios de aceptación de QA del PRD: prueba de carga del
  canal, prueba de resiliencia del enlace TLS y prueba de consistencia en modo WAL.
* Verificación de los cinco NFR con mediciones registradas y reproducibles, contra el presupuesto de
  memoria que corresponda a cada célula según su canal: **< 50 MB en las células sobre canal oficial,
  sin sidecar; ≤ 80 MB en las células sobre canal propio, que lo conservan de forma permanente**. Con
  ambos canales conviviendo, un único presupuesto agregado no significa nada.
* Prueba de densidad: número máximo de células concurrentes que el servidor objetivo sostiene
  cumpliendo los NFR, con el dato documentado.
* Prueba de resistencia prolongada: ejecución sostenida durante varios días buscando fugas de
  memoria, crecimiento no acotado de descriptores de archivo o degradación progresiva.
* Observabilidad de operación: métricas agregadas por célula y por servidor, y alertas sobre las
  condiciones que anticipan un incidente.
* **Verificación a escala del respaldo y la restauración construidos en la etapa A-2**: comportamiento
  con varias células, coste en tiempo y espacio, y una restauración real ejecutada como prueba.
* Revisión de seguridad de la superficie expuesta: puertos, endpoints administrativos, manejo de
  secretos, dependencias con vulnerabilidades conocidas.
* Pruebas de caos acotadas: caída abrupta del contenedor, disco lleno, pérdida de conectividad con el
  proveedor de inferencia y con la API de Meta.
* Documentación operativa consolidada y criterios de salida a producción comercial.

### Qué NO entra

* La construcción del mecanismo de respaldo y restauración. Ya existe desde la etapa A-2; aquí se
  verifica a escala.
* Nuevas funcionalidades. Si una prueba revela un defecto, se corrige; si revela una carencia de
  producto, se registra y se planifica aparte.
* Optimizaciones especulativas sin una medición que las justifique.

### Requisitos del PRD cubiertos

* **NFR-01** — verificación formal del techo de 50 MB por célula en reposo, con varias células.
* **NFR-02** — verificación de tasa nula de errores 502/503 hacia Meta durante operaciones de ciclo
  de vida.
* **NFR-03** — verificación de la conmutación de conocimiento por debajo de 10 milisegundos.
* **NFR-04** — verificación del cifrado TLS 1.2/1.3 en todos los subdominios, según la entrada pública
  elegida.
* **NFR-05** — verificación del aislamiento estricto de almacenamiento entre células.
* Verificación cruzada de **FR-02**, **FR-07**, **FR-08** y **FR-12** en condiciones de sistema
  completo.

---

## Entregables

* `docs/qa/informe-aceptacion.md`: informe con el resultado de cada criterio de QA y de cada NFR, con
  los valores medidos y el procedimiento para reproducirlos.
* Suite de pruebas de sistema automatizada, ejecutable contra un entorno desplegado.
* Panel o exposición de métricas de operación por célula y por servidor.
* Configuración de alertas sobre saldo, memoria, tasa de descarte GCRA y salud de contenedores.
* `docs/runbook-incidentes.md`: guía de diagnóstico y respuesta ante los incidentes previstos.
* Informe de verificación a escala del respaldo y la restauración.
* Informe de revisión de seguridad con los hallazgos y su tratamiento.
* Documento de criterios de salida a producción comercial.

---

## Tareas

*(Sin estimación: la Fase B no se dimensiona hasta que aparezca el cliente que la justifique.)*

1. **Montar el entorno de pruebas de sistema.** Servidor equivalente al de destino con varias células
   dadas de alta mediante el flujo real de la etapa B-2.
2. **Ejecutar y documentar la prueba de carga del canal.** 100 peticiones concurrentes simulando a
   Meta, midiendo códigos de respuesta, latencia y crecimiento de memoria residente.
3. **Ejecutar y documentar la prueba de resiliencia del enlace TLS**, o su equivalente de verificación
   del túnel según la entrada pública elegida.
4. **Ejecutar y documentar la prueba de consistencia en modo WAL.** Conmutación de conocimiento bajo
   20 lecturas RAG simultáneas, con inspección del sistema de archivos al terminar.
5. **Ejecutar la prueba de densidad.** Incremento progresivo del número de células hasta encontrar el
   límite del servidor cumpliendo los NFR; documentar el número.
6. **Ejecutar la prueba de resistencia prolongada.** Vigilancia de memoria, descriptores de archivo y
   latencia a lo largo de varios días.
7. **Instrumentar las métricas de operación.** Agregación por célula y por servidor de memoria,
   admisiones y descartes, saldo, latencia y estado de contenedores.
8. **Configurar las alertas.** Umbrales sobre saldo bajo, memoria cerca del límite, tasa de descarte
   anómala y contenedores no listos.
9. **Verificar el respaldo y la restauración a escala.** Comportamiento con varias células, coste en
   tiempo y espacio, y una restauración completa sobre un entorno limpio.
10. **Ejecutar las pruebas de caos.** Caída abrupta del contenedor, disco lleno y pérdida de
    conectividad con Meta y con el proveedor de inferencia; verificar que ninguna produce corrupción
    de datos ni errores hacia Meta.
11. **Realizar la revisión de seguridad.** Superficie expuesta, endpoints administrativos, manejo de
    secretos y auditoría de dependencias. En la Fase B la superficie es mayor que en la Fase A: hay
    puertos entrantes, certificados y credenciales de Meta.
12. **Redactar el informe de aceptación y el runbook de incidentes.** Consolidar resultados,
    hallazgos y criterios de salida a producción comercial.

---

## Criterios de aceptación

* **Prueba de Carga del Canal (PRD):** con 100 peticiones concurrentes, el control de admisión GCRA se
  activa, el exceso recibe `HTTP 200 OK` rápido y la memoria residente no crece más de un 15 % sobre
  la base.
* **Prueba de Resiliencia del Enlace TLS (PRD):** con el Hairpin NAT bloqueado, el onboarding se
  completa con éxito mediante la resolución forzada del socket; o, si el TLS termina en el edge, el
  túnel se restablece automáticamente tras una caída sin intervención manual.
* **Prueba de Consistencia en Modo WAL (PRD):** el intercambio de conocimiento con 20 lecturas RAG
  simultáneas no arroja `SQLITE_BUSY` ni deja archivos `.db-wal` o `.db-shm` huérfanos.
* **NFR-01:** cada célula en reposo consume menos de 50 MB de RAM, medido con la densidad máxima
  alcanzada.
* **NFR-02:** durante toda la campaña de pruebas de ciclo de vida no se registra ni un solo `502` ni
  `503` hacia la red pública.
* **NFR-03:** la conmutación de conocimiento se mantiene por debajo de 10 milisegundos en todas las
  mediciones.
* **NFR-04:** todos los subdominios negocian TLS 1.2 o 1.3 y rechazan protocolos anteriores.
* **NFR-05:** ninguna célula puede acceder al volumen de otra, verificado con la densidad máxima.
* La ejecución prolongada no muestra crecimiento sostenido de memoria ni de descriptores de archivo.
* Una restauración desde respaldo devuelve una célula a un estado funcional verificado, con el sistema
  operando a densidad comercial.
* Los hallazgos de seguridad de severidad alta están corregidos o tienen una mitigación aceptada por
  escrito.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Una medición incumple un NFR y obliga a rediseñar. | Alto: retrabajo tardío y costoso. | Las etapas A-2, A-4, A-5 y A-6 miden su parte anticipadamente, y los pilotos de la etapa A-7 aportan datos reales; esta etapa confirma, no descubre por primera vez. |
| La densidad real de células es muy inferior a la esperada. | Muy alto: afecta directamente a la viabilidad económica. | Medirla explícitamente y con antelación; si el número es bajo, es una entrada obligatoria para la decisión de monetización, no un detalle técnico. Los pilotos ya dan una primera señal. |
| Fugas lentas que solo se manifiestan tras días de ejecución. | Alto: caídas en producción semanas después del lanzamiento. | Prueba de resistencia prolongada con vigilancia de memoria y descriptores. |
| Las pruebas se ejecutan una vez y nunca más. | Medio: la garantía caduca con el primer cambio. | Automatizar la suite y ejecutarla de forma periódica desde la CI. |
| La superficie de seguridad de la Fase B es mayor y se revisa con el criterio de la Fase A. | Alto: puertos entrantes, certificados y credenciales de Meta expuestos. | Revisión de seguridad específica de la Fase B, no una repetición de la anterior. |
| **Manejo de excepciones comerciales sin definir.** | Medio: no se sabe qué debe ocurrir ante impago, abuso o cancelación. | Se documentan los mecanismos técnicos disponibles; la política queda como decisión de producto pendiente. |

---

## Dependencias

* **De otras etapas:** etapa B-2 completa. Las pruebas requieren células dadas de alta mediante el
  flujo comercial real.
* **Externas:** acceso al servidor de destino o a uno equivalente, capacidad de bloquear el Hairpin
  NAT del router para la prueba de TLS si aplica, y cuota suficiente en los proveedores externos para
  sostener la campaña de pruebas.
* **Decisiones de producto pendientes:** el **manejo de excepciones comerciales** condiciona las
  alertas y los procedimientos de respuesta ante impago o abuso; el **modelo de monetización**
  necesita el dato de densidad que esta etapa produce.
