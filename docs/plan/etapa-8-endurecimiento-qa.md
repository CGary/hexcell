# Etapa 8 — Endurecimiento, QA y operación

**Duración relativa:** Media.

---

## Objetivo

Un sistema no cumple un requisito no funcional porque su diseño lo contemple, sino porque alguien lo
mide y demuestra que lo cumple. Esta etapa cierra el plan sometiendo el sistema completo a los tres
criterios de aceptación de QA que fija el PRD y a los cinco requisitos no funcionales, y dejando
instalado lo necesario para operarlo día a día sin sorpresas.

Las etapas anteriores ya incluyeron pruebas propias, pero eran pruebas de componente ejecutadas en
condiciones controladas. Aquí las pruebas se ejecutan sobre el sistema desplegado, con varios
inquilinos reales conviviendo en el mismo servidor, que es la única configuración en la que las
mediciones significan algo. La densidad de inquilinos por servidor es, además, el número del que
depende directamente la viabilidad económica del producto: cuántos clientes caben en 8 GB de RAM.

La etapa incorpora también lo que hasta ahora se había pospuesto conscientemente: observabilidad
para operar, respaldo y restauración de los datos de cada inquilino, y una revisión de seguridad de
la superficie expuesta. Son trabajos que no añaden funcionalidad visible pero sin los cuales el
primer incidente en producción se resuelve a ciegas.

---

## Alcance

### Qué entra

* Ejecución formal y documentada de los tres criterios de aceptación de QA del PRD: prueba de carga
  de red, prueba de resiliencia del enlace TLS y prueba de consistencia en modo WAL.
* Verificación de los cinco NFR con mediciones registradas y reproducibles.
* Prueba de densidad: número máximo de inquilinos concurrentes que el servidor objetivo sostiene
  cumpliendo los NFR, con el dato documentado.
* Prueba de resistencia prolongada: ejecución sostenida durante varios días buscando fugas de
  memoria, crecimiento no acotado de descriptores de archivo o degradación progresiva.
* Observabilidad de operación: métricas agregadas por inquilino y por servidor, y alertas sobre las
  condiciones que anticipan un incidente.
* Procedimiento de respaldo y restauración por inquilino, con una restauración real ejecutada como
  prueba.
* Revisión de seguridad de la superficie expuesta: puertos, endpoints administrativos, manejo de
  secretos, dependencias con vulnerabilidades conocidas.
* Pruebas de caos acotadas: caída abrupta del contenedor, disco lleno, pérdida de conectividad con el
  proveedor de inferencia y con la API de Meta.
* Documentación operativa consolidada y criterios de salida a producción.

### Qué NO entra

* Nuevas funcionalidades. Si una prueba revela un defecto, se corrige; si revela una carencia de
  producto, se registra y se planifica aparte.
* Optimizaciones especulativas sin una medición que las justifique.

### Requisitos del PRD cubiertos

* **NFR-01** — verificación formal del techo de 50 MB por instancia en reposo, con varios inquilinos.
* **NFR-02** — verificación de tasa nula de errores 502/503 hacia Meta durante operaciones de ciclo
  de vida.
* **NFR-03** — verificación de la conmutación de conocimiento por debajo de 10 milisegundos.
* **NFR-04** — verificación del cifrado TLS 1.2/1.3 en todos los subdominios.
* **NFR-05** — verificación del aislamiento estricto de almacenamiento entre inquilinos.
* Verificación cruzada de **FR-02**, **FR-07** y **FR-08** en condiciones de sistema completo.

---

## Entregables

* `docs/qa/informe-aceptacion.md`: informe con el resultado de cada criterio de QA y de cada NFR, con
  los valores medidos y el procedimiento para reproducirlos.
* Suite de pruebas de sistema automatizada, ejecutable contra un entorno desplegado.
* Panel o exposición de métricas de operación por inquilino y por servidor.
* Configuración de alertas sobre saldo, memoria, tasa de rechazo GCRA y salud de contenedores.
* `docs/runbook-incidentes.md`: guía de diagnóstico y respuesta ante los incidentes previstos.
* Procedimiento de respaldo y restauración, con una restauración verificada.
* Informe de revisión de seguridad con los hallazgos y su tratamiento.
* Documento de criterios de salida a producción.

---

## Tareas

1. **Montar el entorno de pruebas de sistema** (1 día). Servidor equivalente al de destino con varios
   inquilinos dados de alta mediante el flujo real de la etapa 7.
2. **Ejecutar y documentar la prueba de carga de red** (1 día). 100 peticiones concurrentes simulando
   a Meta, midiendo códigos de respuesta, latencia y crecimiento de memoria residente.
3. **Ejecutar y documentar la prueba de resiliencia del enlace TLS** (0,5 días). Alta completa con el
   Hairpin NAT bloqueado, repetida sobre el sistema desplegado.
4. **Ejecutar y documentar la prueba de consistencia en modo WAL** (1 día). Conmutación de
   conocimiento bajo 20 lecturas RAG simultáneas, con inspección del sistema de archivos al terminar.
5. **Ejecutar la prueba de densidad** (1 día). Incremento progresivo del número de inquilinos hasta
   encontrar el límite del servidor cumpliendo los NFR; documentar el número.
6. **Ejecutar la prueba de resistencia prolongada** (1 día de trabajo, varios días de ejecución).
   Vigilancia de memoria, descriptores de archivo y latencia a lo largo del tiempo.
7. **Instrumentar las métricas de operación** (1,5 días). Agregación por inquilino y por servidor de
   memoria, admisiones y rechazos, saldo, latencia y estado de contenedores.
8. **Configurar las alertas** (0,5 días). Umbrales sobre saldo bajo, memoria cerca del límite, tasa
   de rechazo anómala y contenedores no listos.
9. **Implementar y probar el respaldo y la restauración** (1,5 días). Copia consistente de las bases
   de un inquilino, restauración completa sobre un entorno limpio y verificación de que el bot vuelve
   a funcionar con su historial y su conocimiento.
10. **Ejecutar las pruebas de caos** (1,5 días). Caída abrupta del contenedor, disco lleno y pérdida
    de conectividad con Meta y con el proveedor de inferencia; verificar que ninguna produce
    corrupción de datos ni errores hacia Meta.
11. **Realizar la revisión de seguridad** (1,5 días). Superficie expuesta, endpoints
    administrativos, manejo de secretos y auditoría de dependencias.
12. **Redactar el informe de aceptación y el runbook de incidentes** (1 día). Consolidar resultados,
    hallazgos y criterios de salida a producción.

---

## Criterios de aceptación

* **Prueba de Carga de Red (PRD):** con 100 peticiones concurrentes, el middleware GCRA se activa, el
  exceso recibe `HTTP 200 OK` rápido y la memoria residente no crece más de un 15 % sobre la base.
* **Prueba de Resiliencia del Enlace TLS (PRD):** con el Hairpin NAT bloqueado, el onboarding se
  completa con éxito mediante la resolución forzada del socket.
* **Prueba de Consistencia en Modo WAL (PRD):** el intercambio de conocimiento con 20 lecturas RAG
  simultáneas no arroja `SQLITE_BUSY` ni deja archivos `.db-wal` o `.db-shm` huérfanos.
* **NFR-01:** cada instancia en reposo consume menos de 50 MB de RAM, medido con la densidad máxima
  de inquilinos alcanzada.
* **NFR-02:** durante toda la campaña de pruebas de ciclo de vida no se registra ni un solo `502` ni
  `503` hacia la red pública.
* **NFR-03:** la conmutación de conocimiento se mantiene por debajo de 10 milisegundos en todas las
  mediciones.
* **NFR-04:** todos los subdominios negocian TLS 1.2 o 1.3 y rechazan protocolos anteriores.
* **NFR-05:** ningún contenedor puede acceder al volumen de otro, verificado con la densidad máxima.
* La ejecución prolongada no muestra crecimiento sostenido de memoria ni de descriptores de archivo.
* Una restauración desde respaldo devuelve un inquilino a un estado funcional verificado.
* Los hallazgos de seguridad de severidad alta están corregidos o tienen una mitigación aceptada por
  escrito.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Una medición incumple un NFR y obliga a rediseñar. | Alto: retrabajo tardío y costoso. | Las etapas 2, 3, 4 y 5 miden su parte anticipadamente; esta etapa confirma, no descubre por primera vez. |
| La densidad real de inquilinos es muy inferior a la esperada. | Muy alto: afecta directamente a la viabilidad económica. | Medirla explícitamente y con antelación; si el número es bajo, es una entrada obligatoria para la decisión de monetización, no un detalle técnico. |
| Fugas lentas que solo se manifiestan tras días de ejecución. | Alto: caídas en producción semanas después del lanzamiento. | Prueba de resistencia prolongada con vigilancia de memoria y descriptores. |
| Las pruebas se ejecutan una vez y nunca más. | Medio: la garantía caduca con el primer cambio. | Automatizar la suite y ejecutarla de forma periódica desde la CI. |
| **Manejo de excepciones comerciales sin definir.** | Medio: no se sabe qué debe ocurrir ante impago, abuso o cancelación. | Se documentan los mecanismos técnicos disponibles; la política queda como decisión de producto pendiente. |

---

## Dependencias

* **De otras etapas:** etapa 7 completa. Las pruebas requieren inquilinos dados de alta mediante el
  flujo real.
* **Externas:** acceso al servidor de destino o a uno equivalente, capacidad de bloquear el Hairpin
  NAT del router para la prueba de TLS, y cuota suficiente en los proveedores externos para sostener
  la campaña de pruebas.
* **Decisiones de producto pendientes:** el **manejo de excepciones comerciales** condiciona las
  alertas y los procedimientos de respuesta ante impago o abuso; el **modelo de monetización**
  necesita el dato de densidad que esta etapa produce.
