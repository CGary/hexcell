# Fase A · Etapa 1 — Fundaciones del repositorio

**Duración relativa:** Corta.

---

## Objetivo

Esta etapa existe para que el proyecto deje de ser un conjunto de documentos y pase a ser un
repositorio de software con reglas. Hoy hay un repositorio git inicializado con un único commit de
documentación, pero no hay licencia, no hay workspace de Rust y no hay integración continua.

A diferencia de la versión anterior de este plan, la etapa ya **no arrastra la reconstrucción de
FR-01 como hipótesis pendiente**. FR-01 está reconstruido y aprobado en el [PRD](../PRD.md), redactado
por fases: en la Fase A la recepción llega por la sesión whatsmeow del sidecar a través del puerto de
canal, y en la Fase B por webhooks verificados de la Meta Graph API. Lo que esta etapa debe hacer con
FR-01 es **traducirlo a tipos y contratos**, no averiguar qué decía.

La decisión de canal también está tomada y se registra aquí como tal: **whatsmeow** es el adaptador de
la Fase A. Esta etapa no la reabre; la documenta en un ADR para que quede trazable el porqué.

El resultado es un repositorio en el que cualquier persona puede clonar, compilar, ejecutar las
comprobaciones automáticas y leer sin ambigüedad qué frontera separa el dominio del transporte.

---

## Alcance

### Qué entra

* Elección y colocación del archivo `LICENSE`.
* Completar la configuración del repositorio git ya existente: convención de ramas, convención de
  mensajes de commit, plantilla de pull request y `.gitignore` adaptado a Rust y a Go.
* Scaffold del workspace Rust: `Cargo.toml` raíz de tipo *workspace* y la división en crates
  propuesta más abajo, cada uno compilando aunque su contenido sea todavía un esqueleto.
* **Declaración del trait `ChannelAdapter` (FR-12) como esqueleto de tipos**, sin implementación: el
  evento entrante canónico, el envío tipado con su resultado tipado, el estado de la ventana de
  servicio, el identificador interno de conversación, los acuses normalizados y el sub-trait opcional
  de ciclo de vida de sesión. Los tipos se abstraen **hacia el caso más restrictivo** —la Cloud
  API—, no hacia el más permisivo. Es la frontera que hace posible el salto de fase, y por eso nace
  en la primera etapa y no cuando haga falta.
* Registro documental de las decisiones ya tomadas: estrategia de dos fases y elección de whatsmeow
  como adaptador no oficial.
* Integración continua mínima: formato, análisis estático, compilación y ejecución de pruebas, tanto
  del workspace Rust como del módulo Go que albergará el sidecar.
* Fijación de la versión mínima de Rust soportada, de la versión de Go del sidecar y del perfil de
  compilación de *release* orientado a tamaño y consumo, coherente con NFR-01.

### Qué NO entra

* Cualquier lógica funcional real: no se implementa ningún adaptador, solo se declara el puerto.
* Docker, SQLite y la CLI de administración. Ninguno se toca en esta etapa.
* Decisiones de producto pendientes en STATUS.md. No se abordan aquí.

### Requisitos del PRD cubiertos

* **FR-12** — cubierto en su dimensión de **declaración del contrato**. Las implementaciones llegan en
  las etapas A-3 (whatsmeow) y B-1 (Cloud API).
* **FR-01** — se traduce a tipos el contrato ya aprobado en el PRD. La implementación de la variante
  de Fase A corresponde a la etapa A-3.

---

## Entregables

* `LICENSE` en la raíz del repositorio.
* `docs/adr/adr-0001-licencia.md` y `docs/adr/adr-0002-estructura-workspace.md`,
  registros de decisión de arquitectura breves que dejan constancia del porqué de cada elección.
* Las decisiones de canal registradas, con la numeración que fija el
  [índice de ADR](../adr/README.md):
  * `docs/adr/adr-0008-estrategia-canal-dos-fases.md` — dos fases con compuerta en el tercer cliente.
  * `docs/adr/adr-0009-whatsmeow-adaptador-fase-a.md` — elección de whatsmeow sobre las alternativas.
  * `docs/adr/adr-0010-puerto-de-canal.md` — el puerto de canal como frontera de migración.
* `Cargo.toml` raíz declarando el workspace y las dependencias compartidas.
* Crates del workspace, todos compilando en vacío:
  * `hexcell-core` — tipos de dominio compartidos, errores, configuración y **el trait
    `ChannelAdapter` con sus tipos canónicos**. Sin dependencias de infraestructura.
  * `hexcell` — binario que se ejecuta dentro del contenedor del núcleo de cada célula.
  * `hexcell-admin` — binario de la CLI central de administración.
  * `hexcell-storage` — capa de acceso a SQLite y gestión de pools.
  * `hexcell-meta` — cliente y tipos de la Meta Graph API, incluida la verificación de firma. Nace
    como esqueleto: su contenido real pertenece a la Fase B. En esta etapa queda como `lib.rs`
    **vacío, sin ningún tipo ni trait público**, no como un esqueleto con forma anticipada: la
    entrada pública de la Fase B es la decisión pendiente del ADR-0013 (Cloudflare Tunnel frente a
    VPS + WireGuard), y diseñar tipos antes de resolverla arriesga a condicionar esa decisión en
    lugar de esperarla.
* Módulo Go inicializado (`sidecar/`) que compila en vacío y que albergará el adaptador whatsmeow.
* `.github/workflows/ci.yml` (o el equivalente del proveedor elegido) con el pipeline mínimo.
* `rust-toolchain.toml`, `rustfmt.toml` y `clippy.toml`.
* `CONTRIBUTING.md` con las convenciones de rama y de commit.

---

## Tareas

1. **Declarar el puerto de canal en `hexcell-core`** (1,5 días). Tipos del evento entrante canónico
   (remitente, conversación, contenido, marca temporal, identificador de deduplicación), firma de
   `send(conversation_id, mensaje)` con el mensaje tipado como `RespuestaLibre` o
   `Plantilla { id, parámetros }`, **resultado tipado del envío** con `FueraDeVentana`,
   `PlantillaRequerida`, `LimiteDeTasa` y `DestinatarioInvalido`, **consulta del estado de la ventana
   de servicio de 24 h** por conversación, tipo del identificador interno de conversación, enumerado
   de acuses (`sent`/`delivered`/`read`/`failed`) y sub-trait opcional de ciclo de vida de sesión.
   Los tipos se diseñan hacia el caso restrictivo (FR-12): si el puerto solo sabe expresar lo que
   whatsmeow permite, la Fase B lo rehará entero. Que los tipos no mencionen ni `wa_id` ni JID es
   necesario pero no basta: es una prueba léxica, y un diseño puede ser semánticamente incorrecto sin
   violar esa regla textual. Por eso la tarea incluye, además, tests de compilación con `match`
   exhaustivo (sin brazo `_`) sobre `FueraDeVentana`, `PlantillaRequerida`, `LimiteDeTasa` y
   `DestinatarioInvalido`, de modo que añadir o quitar una variante de ese enumerado rompa la
   compilación de los propios tests; y el cotejo documentado de cada variante y del evento entrante
   canónico contra la documentación oficial de la Meta Cloud API —no solo contra el PRD, que podría
   arrastrar el mismo error de origen sin que nadie lo note.
2. **Registrar las decisiones de canal en ADR** (0,5 días). Tres registros breves, numerados según el
   [índice de ADR](../adr/README.md): `adr-0008` (estrategia de dos fases con la compuerta del tercer
   cliente), `adr-0009` (elección de whatsmeow frente a alternativas) y `adr-0010` (el puerto de canal
   como frontera de migración). Son decisiones ya tomadas: el ADR documenta el porqué y las
   consecuencias, no vuelve a deliberar.
3. **Decidir la licencia y registrarla** (0,5 días). Contrastar al menos dos opciones frente al
   objetivo comercial del proyecto, escribir el ADR y colocar el archivo `LICENSE`.
4. **Completar la configuración del repositorio** (0,5 días). Convención de ramas, convención de
   mensajes de commit, plantilla de pull request, `.gitignore` para artefactos de Rust y de Go.
5. **Crear el workspace Cargo y los cinco crates** (1,5 días). Definir dependencias comunes en el
   workspace, fijar los límites entre crates y documentar en el ADR por qué la frontera está donde
   está. `hexcell-core` no puede depender de ningún crate de infraestructura. `hexcell-meta` se crea
   como `lib.rs` vacío, sin tipos ni traits públicos: su forma queda abierta hasta que se abra la
   compuerta de la Fase B y se resuelva el ADR-0013, que es quien la condiciona.
6. **Inicializar el módulo Go del sidecar** (0,5 días). Estructura mínima que compile, con la
   dependencia de whatsmeow declarada y fijada a una versión concreta, de modo que un *bump* ante una
   rotura de protocolo sea un cambio de una línea.
7. **Fijar toolchain y perfiles de compilación** (0,5 días). Versión mínima de Rust, versión de Go,
   `rustfmt`, `clippy` con negación de avisos, y perfil de *release* optimizado a tamaño de binario.
8. **Montar la CI mínima** (1 día). Ejecutar en cada push: comprobación de formato, análisis
   estático sin avisos, compilación de todo el workspace y del módulo Go, y ejecución de pruebas. La
   CI debe fallar ante cualquiera de las comprobaciones. **Alcance de esta certificación**: la CI de
   A-1 certifica compilación, formato y análisis estático; no certifica la corrección semántica del
   diseño del puerto de canal. Esa la certifican los tests de contrato de la etapa A-2 y los tests de
   tipos exhaustivos de la tarea 1.
9. **Escribir el esqueleto de pruebas y un caso real** (0,5 días). Una prueba efectiva sobre el mapeo
   de un identificador de transporte a identificador interno, para que la CI nazca con contenido y no
   con un pipeline vacío. No basta con que la prueba exista: hay que demostrar, dejando constancia en
   el PR o en el commit, que introducir deliberadamente esa filtración (por ejemplo, exponiendo
   `wa_id` en un tipo de dominio) hace fallar la prueba y, por tanto, la build de CI. La prueba de que
   el guardián funciona es que muerde cuando se le provoca, no solo que está presente en el código.

---

## Criterios de aceptación

* El trait `ChannelAdapter` y sus tipos canónicos compilan en `hexcell-core` y **ninguna firma
  pública menciona un identificador de transporte** (`wa_id`, JID o equivalente).
* Existen tests de compilación con `match` exhaustivo sobre `FueraDeVentana`, `PlantillaRequerida`,
  `LimiteDeTasa` y `DestinatarioInvalido`, tales que añadir o quitar una variante de ese enumerado
  rompe la compilación de los tests, no solo la del crate.
* Cada variante de esos enums y el evento entrante canónico están cotejados documentalmente contra
  la documentación oficial de la Meta Cloud API, no solo contra el PRD.
* Existe un archivo `LICENSE` en la raíz y un ADR que justifica la elección.
* Existen los ADR `adr-0008`, `adr-0009` y `adr-0010`, que registran la estrategia de dos fases, la
  elección de whatsmeow y el puerto de canal, y el índice de ADR los recoge con esos mismos números.
* `cargo build --workspace` y `cargo test --workspace` terminan sin errores desde un clon limpio.
* `hexcell-meta` compila vacío y no expone ningún tipo ni trait público: su forma queda pendiente
  hasta que se resuelva el ADR-0013.
* `cargo fmt --check` y `cargo clippy --workspace -- -D warnings` terminan sin hallazgos.
* El módulo Go del sidecar compila y su dependencia de whatsmeow está fijada a una versión explícita.
* La CI se ejecuta automáticamente en cada push y bloquea la fusión si alguna comprobación falla.
* Existe al menos una prueba que valida el mapeo de identidad de conversación y que falla si el
  identificador de transporte se filtra al dominio. Queda documentado el experimento de introducir
  esa filtración a propósito y comprobar que la prueba —y por tanto la build de CI— falla: el
  guardián muerde, no solo existe.
* Queda explícito que la CI de esta etapa certifica compilación, formato y análisis estático, no la
  corrección semántica del diseño del puerto: esa la certifican los tests de contrato de la etapa
  A-2 y los tests de tipos exhaustivos del punto de la validación semántica del `ChannelAdapter`.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| El puerto de canal se diseña pensando solo en whatsmeow y no sirve para la Cloud API. | Muy alto: la frontera de migración deja de existir y la Fase B se convierte en una reescritura. | Abstraer hacia el **caso más restrictivo** (FR-12): envío tipado, resultado con `FueraDeVentana`/`PlantillaRequerida`/`LimiteDeTasa`/`DestinatarioInvalido`, y estado de la ventana de 24 h. Que la firma compile no demuestra nada: la validación real es que **los tests de contrato del puerto ejerciten esa semántica restrictiva contra el adaptador simulado** de la etapa A-2, con ventanas que expiran y plantillas exigidas. Un criterio puramente léxico —que compile y que ninguna firma mencione `wa_id` o JID— es insuficiente por sí solo: el mismo error de diseño puede repetirse bajo nombres distintos sin violar esa regla textual, y si la referencia de cotejo es solo el PRD, un error del PRD se traslada intacto al puerto. Por eso se añaden tests de `match` exhaustivo que rompen ante cualquier cambio de forma en los enums de resultado, y el cotejo se hace contra la documentación oficial de la Meta Cloud API, no solo contra el PRD. |
| Un identificador de transporte se cuela en el dominio o en `sessions.db`. | Alto: la migración de fase obligaría a migrar datos históricos. | Prueba explícita en esta etapa y revisión del esquema en la etapa A-2. |
| La elección de licencia se pospone "para más adelante". | Medio: cada commit posterior aumenta el coste de cambiar de licencia por la necesidad de consentimiento de los contribuyentes. | Es tarea bloqueante de la etapa: no se abre la etapa A-2 sin `LICENSE` en la raíz. |
| La frontera entre crates se elige mal y obliga a refactorizaciones profundas. | Medio. | Mantener `hexcell-core` sin dependencias de infraestructura y revisar la división al cerrar la etapa A-5, que es la que más presiona el diseño. |
| La CI se percibe como fricción y se acaba desactivando. | Medio: la deuda técnica se acumula en silencio. | Mantenerla estrictamente mínima en esta fase: comprobaciones rápidas, sin pasos lentos. |

---

## Dependencias

* **De otras etapas:** ninguna. Es la etapa inicial.
* **Externas:** una decisión del responsable del proyecto sobre la licencia. Es bloqueante.
* **Decisiones de producto pendientes:** ninguna de las listadas en STATUS.md bloquea esta etapa.
