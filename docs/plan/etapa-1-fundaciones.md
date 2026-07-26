# Etapa 1 — Fundaciones del repositorio y contrato con Meta

**Duración relativa:** Corta.

---

## Objetivo

Esta etapa existe para que el proyecto deje de ser un conjunto de documentos y pase a ser un
repositorio de software con reglas. Hoy hay un repositorio git inicializado con un único commit de
documentación, pero no hay licencia, no hay workspace de Rust, no hay integración continua y, sobre
todo, hay un requisito funcional incompleto: FR-01 llegó truncado en el documento original y solo
sabemos por contexto que describía la recepción y verificación de los webhooks de la Meta Graph API.

Empezar a programar el receptor de webhooks sin haber reconstruido ese requisito sería edificar
sobre una suposición no verificada, y es precisamente el componente que queda expuesto a internet.
Por eso la etapa combina dos trabajos que parecen distintos pero comparten la misma naturaleza:
establecer el andamiaje del repositorio y **cerrar el contrato de integración con Meta por escrito**,
antes de que exista una sola línea de lógica que dependa de él.

El resultado es un repositorio en el que cualquier persona puede clonar, compilar, ejecutar las
comprobaciones automáticas y leer sin ambigüedad qué debe hacer el sistema cuando Meta llama a la
puerta.

---

## Alcance

### Qué entra

* Reconstrucción documental de **FR-01**. El texto original se perdió por truncado, de modo que lo
  que sigue es una **HIPÓTESIS DE RECONSTRUCCIÓN, no un contrato vigente**: es la propuesta de
  partida que esta etapa debe validar contra la documentación oficial de la Meta Graph API y someter
  a aprobación explícita antes de incorporarse al PRD. La hipótesis es que FR-01 cubría la
  verificación del webhook (el desafío `hub.challenge` que Meta envía al suscribir una URL), la
  validación de la firma criptográfica de cada entrega (`X-Hub-Signature-256`, HMAC-SHA256 sobre el
  cuerpo exacto de la petición) y la política de respuesta rápida con `HTTP 200 OK`. Cualquiera de
  estos tres elementos puede cambiar o caer tras la validación; el resto del plan asume el texto que
  se apruebe, no esta hipótesis.
* Elección y colocación del archivo `LICENSE`.
* Completar la configuración del repositorio git ya existente: convención de ramas, convención de
  mensajes de commit, plantilla de pull request y `.gitignore` adaptado a Rust.
* Scaffold del workspace Rust: `Cargo.toml` raíz de tipo *workspace* y la división en crates
  propuesta más abajo, cada uno compilando aunque su contenido sea todavía un esqueleto.
* Integración continua mínima: formato, análisis estático, compilación y ejecución de pruebas.
* Fijación de la versión mínima de Rust soportada y del perfil de compilación de *release*
  orientado a tamaño y consumo, coherente con NFR-01.

### Qué NO entra

* Cualquier lógica funcional real: no se implementa el receptor de webhooks, solo se especifica.
* Docker, Caddy, SQLite y la CLI de administración. Ninguno se toca en esta etapa.
* Decisiones de producto pendientes en STATUS.md. No se abordan aquí.

### Requisitos del PRD cubiertos

* **FR-01** — cubierto en su dimensión de **especificación**. La implementación corresponde a la
  etapa 2. Ninguna otra parte del plan asume un FR-01 distinto del que se fije aquí.

---

## Entregables

* `LICENSE` en la raíz del repositorio.
* `docs/PRD.md` actualizado: el texto de FR-01 sustituye al marcador de TODO.
* `docs/adr/adr-0001-licencia.md` y `docs/adr/adr-0002-estructura-workspace.md`,
  registros de decisión de arquitectura breves que dejan constancia del porqué de cada elección.
* `Cargo.toml` raíz declarando el workspace y las dependencias compartidas.
* Crates del workspace, todos compilando en vacío:
  * `zeroclaw-core` — tipos de dominio compartidos, errores, configuración y contratos de webhook.
  * `zeroclaw-tenant` — binario que se ejecuta dentro del contenedor de cada inquilino.
  * `zeroclaw-admin` — binario de la CLI central de administración.
  * `zeroclaw-storage` — capa de acceso a SQLite y gestión de pools.
  * `zeroclaw-meta` — cliente y tipos de la Meta Graph API, incluida la verificación de firma.
* `.github/workflows/ci.yml` (o el equivalente del proveedor elegido) con el pipeline mínimo.
* `rust-toolchain.toml`, `rustfmt.toml` y `clippy.toml`.
* `CONTRIBUTING.md` con las convenciones de rama y de commit.

---

## Tareas

1. **Reconstruir FR-01 contra la documentación oficial de Meta** (1,5 días). Partir de la hipótesis
   de reconstrucción declarada en el alcance —que es una propuesta a validar, no un contrato— y
   contrastarla punto por punto con la documentación oficial vigente. Elementos a confirmar,
   corregir o descartar: flujo de verificación con `hub.mode`, `hub.verify_token` y `hub.challenge`;
   validación de firma HMAC sobre el cuerpo sin reserializar; ventana de respuesta esperada por
   Meta; semántica de reintentos y por qué cualquier código distinto de 200 los dispara; y garantía
   de idempotencia frente a entregas duplicadas. El requisito no se da por reconstruido hasta contar
   con revisión y aprobación explícita del responsable de producto.
2. **Incorporar FR-01 al PRD** (0,5 días). Edición mínima que sustituye el marcador de TODO y añade
   la referencia a la fuente consultada.
3. **Decidir la licencia y registrarla** (0,5 días). Contrastar al menos dos opciones frente al
   objetivo comercial del proyecto, escribir el ADR y colocar el archivo `LICENSE`.
4. **Completar la configuración del repositorio** (0,5 días). Convención de ramas, convención de
   mensajes de commit, plantilla de pull request, `.gitignore` para artefactos de Rust.
5. **Crear el workspace Cargo y los cinco crates** (1,5 días). Definir dependencias comunes en el
   workspace, fijar los límites entre crates y documentar en el ADR por qué la frontera está donde
   está.
6. **Fijar toolchain y perfil de compilación** (0,5 días). Versión mínima de Rust, `rustfmt`,
   `clippy` con negación de avisos, y perfil de *release* optimizado a tamaño de binario.
7. **Montar la CI mínima** (1 día). Ejecutar en cada push: comprobación de formato, análisis
   estático sin avisos, compilación de todo el workspace y ejecución de pruebas. La CI debe fallar
   ante cualquiera de los cuatro.
8. **Escribir el esqueleto de pruebas y un caso real** (0,5 días). Una prueba unitaria efectiva
   sobre la verificación de firma HMAC, para que la CI nazca con contenido y no con un pipeline vacío.

---

## Criterios de aceptación

* `docs/PRD.md` ya no contiene ningún marcador de TODO en FR-01, y el texto reconstruido ha sido
  aprobado explícitamente por el responsable de producto.
* Existe un archivo `LICENSE` en la raíz y un ADR que justifica la elección.
* `cargo build --workspace` y `cargo test --workspace` terminan sin errores desde un clon limpio.
* `cargo fmt --check` y `cargo clippy --workspace -- -D warnings` terminan sin hallazgos.
* La CI se ejecuta automáticamente en cada push y bloquea la fusión si alguna comprobación falla.
* Existe al menos una prueba que valida la firma HMAC-SHA256 de un webhook con un vector conocido,
  y que falla si se altera un solo byte del cuerpo.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| La reconstrucción de FR-01 no coincide con la intención original del autor del PRD. | Alto: todo el receptor de webhooks se construye sobre un contrato equivocado. | Redactarlo contra la documentación oficial de Meta, no de memoria, y exigir aprobación explícita antes de cerrar la etapa. |
| La elección de licencia se pospone "para más adelante". | Medio: cada commit posterior aumenta el coste de cambiar de licencia por la necesidad de consentimiento de los contribuyentes. | Es tarea bloqueante de la etapa: no se abre la etapa 2 sin `LICENSE` en la raíz. |
| La frontera entre crates se elige mal y obliga a refactorizaciones profundas. | Medio. | Mantener `zeroclaw-core` sin dependencias de infraestructura y revisar la división al cerrar la etapa 4, que es la que más presiona el diseño. |
| La CI se percibe como fricción y se acaba desactivando. | Medio: la deuda técnica se acumula en silencio. | Mantenerla estrictamente mínima en esta fase: cuatro comprobaciones rápidas, sin pasos lentos. |

---

## Dependencias

* **De otras etapas:** ninguna. Es la etapa inicial.
* **Externas:** acceso a la documentación oficial vigente de la Meta Graph API para reconstruir
  FR-01, y una decisión del responsable del proyecto sobre la licencia. Ambas son bloqueantes.
* **Decisiones de producto pendientes:** ninguna de las listadas en STATUS.md bloquea esta etapa.
