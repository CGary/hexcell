# Etapa 6 — Plano de control: Caddy y CLI de administración

**Duración relativa:** Media.

---

## Objetivo

Con la etapa 5 tenemos contenedores que funcionan, pero todavía no hay forma de gobernarlos. Esta
etapa construye el **plano de control**: el componente que decide qué subdominio apunta a qué
contenedor, cuándo un inquilino está activo, suspendido o eliminado, y cómo se hacen esas
transiciones sin que Meta llegue a notar nada.

El requisito que da forma a todo lo demás es NFR-02: **cero errores HTTP 502 o 503 expuestos hacia la
WAN de Meta durante suspensiones o reactivaciones**. Es una exigencia dura, porque la forma natural
de apagar un backend es apagarlo, y entonces el proxy inverso responde 502. La respuesta del PRD es
invertir el orden de las operaciones: primero se sustituye el proxy inverso por una respuesta
estática de `HTTP 200 OK` en Caddy (*blackholing*), y solo después se envía el `SIGTERM` al
contenedor. Mientras el contenedor se apaga, Caddy sigue absorbiendo los webhooks y confirmándolos a
Meta. Al reactivar se hace lo simétrico: se arranca el contenedor, se le interroga cada 100 ms hasta
que su `GET /health/ready` responde `200 OK`, y solo entonces se conmuta el tráfico real.

La CLI que implementa estas secuencias es también el lugar donde vive el requisito de eliminación
definitiva de un inquilino, que toca a la vez a Meta, a Docker, al disco y a Caddy, y que por tanto
necesita un orden de operaciones pensado para que un fallo a mitad de camino no deje el sistema en un
estado inconsistente.

---

## Alcance

### Qué entra

* Integración con la API de administración de Caddy: alta, modificación y baja de rutas por
  subdominio de forma programática, sin recargar la configuración global ni interrumpir a terceros.
* Configuración de TLS automático en Caddy, incluida la emisión bajo demanda y su restricción a los
  dominios legítimamente registrados.
* Cliente del socket Unix de Docker en `zeroclaw-admin`: arranque, parada con margen de gracia,
  inspección de estado y eliminación de contenedores y volúmenes.
* Comando `tenant pause`: blackholing en Caddy y después `SIGTERM` al contenedor con 30 segundos de
  gracia.
* Comando `tenant unpause`: arranque del contenedor, bucle de sondeo de disponibilidad cada 100 ms
  contra `GET /health/ready` y conmutación de la respuesta estática al proxy inverso solo tras la
  primera confirmación positiva.
* Comando `tenant terminate`: desasociación del webhook en la Meta Graph API, drenaje del
  contenedor, destrucción física de los volúmenes y purga de la ruta y de la caché de certificados
  en Caddy.

> **Nota de fuente.** El PRD cubre explícitamente la suspensión y la reactivación (FR-11 y las
> matrices de ciclo de vida de la sección 5), pero **no la eliminación definitiva**. El comando
> `tenant terminate` y su secuencia provienen del [README.md del proyecto](../../README.md),
> "Manual de Operación de la CLI de Administración", apartado 3. No es un requisito inventado por
> este plan, pero su rango es inferior al de los FR: ante conflicto, manda el PRD.
* Comandos auxiliares de operación: listado de inquilinos con su estado, y consulta del estado de uno
  concreto.
* Registro persistente del estado de cada inquilino en el plano de control, de modo que la CLI no
  dependa exclusivamente de inferir el estado a partir de Docker y Caddy.
* Idempotencia y recuperación: cada comando debe poder reejecutarse tras un fallo parcial y dejar el
  sistema en el estado pretendido.

### Qué NO entra

* El alta de un inquilino nuevo, incluido el handshake sintético contra Meta: etapa 7. Aquí se
  construyen las piezas que ese alta necesitará.
* La lógica interna del contenedor, que ya está terminada en las etapas 2 a 4.
* Cualquier interfaz gráfica de administración.

### Requisitos del PRD cubiertos

* **FR-03** — gestión de configuración dinámica de Caddy por subdominio sin interrumpir a terceros.
* **FR-11** — operaciones CLI de tráfico amortiguado con blackholing previo al `SIGTERM`.
* **NFR-02** — cero errores 502/503 hacia Meta durante suspensiones y reactivaciones.
* **NFR-04** — cifrado HTTPS con TLS 1.2/1.3 gestionado automáticamente por Caddy.

---

## Entregables

* `zeroclaw-admin` como binario funcional con los comandos `tenant pause`, `tenant unpause`,
  `tenant terminate`, `tenant list` y `tenant status`.
* Módulo cliente de la API de administración de Caddy.
* Módulo cliente del socket Unix de Docker.
* Configuración base de Caddy versionada en el repositorio, con la política de TLS.
* Almacén de estado del plano de control con su esquema y migraciones.
* `docs/adr/adr-0008-plano-de-control.md` con el orden de operaciones de cada secuencia y su
  justificación.
* `docs/runbook-operacion.md`: manual breve de operación con los comandos y sus efectos.
* Prueba de integración que mide códigos HTTP durante un ciclo completo de pausa y reactivación.

---

## Tareas

1. **Definir el modelo de estado del inquilino** (0,5 días). Estados posibles, transiciones válidas y
   esquema del almacén del plano de control.
2. **Implementar el cliente de la API de administración de Caddy** (1,5 días). Operaciones de alta,
   modificación y baja de ruta, con la granularidad necesaria para no reescribir la configuración
   completa.
3. **Establecer la configuración base de Caddy y su política TLS** (1 día). TLS 1.2/1.3, emisión
   automática de certificados y restricción de la emisión bajo demanda a los dominios registrados en
   el plano de control.
4. **Implementar el cliente del socket Unix de Docker** (1,5 días). Arranque, parada con margen,
   inspección, eliminación de contenedor y de volúmenes, con manejo explícito de errores.
5. **Construir el esqueleto de la CLI** (0,5 días). Analizador de argumentos, salida legible,
   códigos de retorno significativos y modo de simulación que muestra lo que haría sin ejecutarlo.
6. **Implementar `tenant pause`** (1 día). Blackholing en Caddy, verificación de que la respuesta
   estática ya está activa, y solo entonces `SIGTERM` con 30 segundos de gracia.
7. **Implementar `tenant unpause`** (1,5 días). Arranque, bucle de sondeo cada 100 ms con límite
   temporal y mensaje de error claro si nunca llega a estar listo, y conmutación final al proxy
   inverso.
8. **Implementar `tenant terminate`** (1,5 días). Orden de operaciones definido: desuscripción en
   Meta, drenaje del contenedor, borrado físico de volúmenes, purga de ruta y de certificados;
   confirmación explícita requerida por tratarse de una operación destructiva.
9. **Implementar `tenant list` y `tenant status`** (0,5 días). Vista del estado consolidado
   cruzando el plano de control con la realidad de Docker y de Caddy, señalando discrepancias.
10. **Dotar de idempotencia y recuperación a los comandos** (1 día). Reejecución segura tras un fallo
    parcial, con detección del punto en que quedó la secuencia.
11. **Construir la prueba de integración de ciclo de vida** (1,5 días). Tráfico continuo contra el
    subdominio mientras se ejecuta pausa y reactivación, registrando todos los códigos de respuesta.
12. **Escribir el runbook de operación** (0,5 días). Qué comando usar en cada situación, qué efecto
    tiene y cómo verificar que salió bien.

---

## Criterios de aceptación

* Durante un ciclo completo de `tenant pause` seguido de `tenant unpause`, con tráfico continuo
  contra el subdominio, **el 100 % de las respuestas son `HTTP 200 OK`**: ni un solo 502 ni 503
  (NFR-02).
* `tenant pause` deja el contenedor detenido con código de salida 0 y la ruta de Caddy devolviendo
  una respuesta estática `200 OK` con cuerpo `{}`.
* `tenant unpause` no conmuta el tráfico al proxy inverso hasta que `GET /health/ready` ha respondido
  `200 OK` al menos una vez; forzar un backend que nunca esté listo produce un fallo explícito y el
  tráfico permanece absorbido por la respuesta estática.
* Alta y baja de una ruta en Caddy para un inquilino no interrumpen ni alteran el tráfico de los
  demás inquilinos activos, verificado con tráfico concurrente (FR-03).
* Todos los subdominios sirven exclusivamente sobre TLS 1.2 o 1.3, y una conexión con protocolos
  anteriores es rechazada (NFR-04).
* `tenant terminate` deja el sistema sin rastro del inquilino: sin contenedor, sin volúmenes en
  disco, sin ruta en Caddy y sin suscripción de webhook en Meta.
* Interrumpir cualquier comando a mitad y reejecutarlo lleva el sistema al estado pretendido sin
  intervención manual.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Enviar el `SIGTERM` antes de aplicar el blackholing. | Muy alto: se generan 502 hacia Meta, incumpliendo NFR-02 y activando reintentos. | El orden está fijado en el ADR y verificado por la prueba de ciclo de vida con tráfico continuo. |
| Una modificación de la configuración de Caddy afecta a rutas de otros inquilinos. | Muy alto: caída de clientes ajenos a la operación. | Usar operaciones de grano fino sobre la ruta concreta y probar siempre con varios inquilinos activos. |
| La API de administración de Caddy queda expuesta más allá del host local. | Muy alto: control total del enrutamiento para un atacante. | Vincularla exclusivamente a la interfaz de loopback y documentarlo en el runbook. |
| Fallo parcial de `tenant terminate` que deja datos en disco o una suscripción viva en Meta. | Alto: fuga de datos o tráfico entrante hacia un inquilino inexistente. | Orden de operaciones que desconecta primero y destruye después, con idempotencia y verificación final de cada paso. |
| Límites de tasa de la API Graph al desuscribir. | Medio. | Reintentos acotados y registro del estado pendiente para reejecución posterior. |
| **Criterio de suspensión por falta de pago sin definir** (monetización pendiente). | Medio: existe el mecanismo pero no la política que lo dispara. | La CLI se opera manualmente en esta etapa; la automatización de la suspensión queda bloqueada hasta que exista el modelo de monetización. |

---

## Dependencias

* **De otras etapas:** etapa 5 completa. La CLI necesita una imagen que arrancar, un volumen que
  destruir y un endpoint `GET /health/ready` al que sondear.
* **Externas:** un servidor con Caddy y Docker accesibles, un dominio propio con control de DNS, y
  credenciales de la Meta Graph API con permiso para desuscribir webhooks.
* **Decisiones de producto pendientes:** el **modelo de monetización** define cuándo se suspende a un
  cliente por falta de pago. El mecanismo se entrega aquí; la política que lo activa, no.
