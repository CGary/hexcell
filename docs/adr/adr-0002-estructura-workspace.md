# ADR-0002 — División en crates del workspace Rust y sus fronteras

* **Estado:** Vigente desde el 2026-07-29.
* **Supersede a:** nada. Es la primera formalización de una división que el plan de la etapa A-1 ya
  enunciaba como entregable.
* **Etapa:** A-1 (creación del workspace y declaración del puerto de canal).
* **Requisitos tocados:** FR-05, FR-11, FR-12.

---

## Contexto

El repositorio pasa de ser un conjunto de documentos a ser software. La primera decisión estructural
—dónde se corta el código en unidades de compilación— es de las que cuesta poco tomar y mucho
deshacer: cuando una frontera está mal puesta, no se nota al ponerla, se nota dos etapas después,
cuando ya hay código a los dos lados y clientes de pago encima.

El producto tiene una frontera que ya está decidida y que no es negociable: `adr-0010` establece que
**el núcleo Rust no conoce ningún transporte de WhatsApp** y que toda integración de canal vive
detrás del trait `ChannelAdapter`. Esa frontera no se sostiene con una convención de estilo ni con
una revisión atenta. Se sostiene si es **imposible** cruzarla por accidente, y la única forma barata
de hacerla imposible en Rust es que el crate que contiene el dominio **no tenga forma de nombrar**
lo que hay al otro lado, porque no depende de ello.

Hay además una restricción de producto que condiciona la división: la Fase B —el canal oficial sobre
la Meta Cloud API— existe como destino conocido pero su entrada de red es una decisión **pendiente**
(`adr-0013`). Cualquier tipo que se escriba hoy para ese canal condiciona esa decisión en lugar de
esperarla.

Y una tercera, de operación: una célula ejecuta un binario dentro de su contenedor, y la
administración de la cartera de células se hace desde fuera, con otra herramienta. Son dos programas
con ciclos de vida, superficies y usuarios distintos.

## Decisión

1. **Un workspace Cargo con `resolver = "3"` y exactamente cinco crates bajo `crates/`**, con los
   metadatos comunes —versión, edición 2024, versión mínima de Rust 1.92 y licencia— declarados una
   sola vez en `[workspace.package]` y heredados por cada miembro.

2. **`hexcell-core` no tiene dependencias, y su tabla vacía es un criterio de aceptación**, no una
   coincidencia temporal. Contiene los tipos de dominio y la declaración del puerto de canal. No
   depende de `hexcell-storage`, ni de `hexcell-meta`, ni de ningún crate de almacenamiento,
   transporte, motor de ejecución asíncrona o cliente HTTP. Para el tiempo usa
   `std::time::SystemTime` y `std::time::Duration`; traer una biblioteca de fechas por comodidad
   abriría exactamente la puerta que esta decisión cierra. Si algún día necesita una dependencia
   externa, tendrá que ser un crate de datos puros, sin entrada ni salida, y su justificación se
   añade a este ADR.

3. **`hexcell-storage` es un crate separado y no un módulo de `hexcell-core`.** El motivo es
   directamente el punto anterior: el día que entre el motor de SQLite (etapa A-2), esa dependencia
   tiene que aterrizar en un crate que no sea el del dominio. Si la persistencia fuese un módulo del
   núcleo, la tabla de dependencias del núcleo dejaría de estar vacía y la frontera pasaría a
   depender de que nadie escriba el `use` equivocado.

4. **`hexcell-meta` es un crate separado y nace vacío, sin ningún elemento visible desde fuera.** Su
   forma la condiciona `adr-0013`, la entrada de red del canal oficial, que está sin resolver.
   Escribir hoy tipos de verificación de firma o de cliente HTTP no adelantaría trabajo: **pesaría
   sobre una decisión que aún no se ha tomado**. El crate existe igualmente desde el primer día para
   que el canal oficial tenga su sitio reservado en el workspace y para que la frontera quede fijada
   antes de que haya código que la cruce.

5. **Dos binarios, `hexcell` y `hexcell-admin`.** El primero es el núcleo que corre dentro del
   contenedor de cada célula; el segundo, la CLI central de administración (FR-11). Separarlos evita
   que el binario que se despliega en cada célula cargue el código de operación de toda la cartera:
   sobre un presupuesto de línea base de 80 MB por célula, meter la herramienta de administración
   dentro del contenedor del cliente es pagar memoria y superficie por algo que ahí no se usa nunca.

6. **Los métodos del puerto se declaran devolviendo `impl Future<Output = ...> + Send`**, y no con
   la forma abreviada asíncrona dentro del trait. Sobre rustc 1.92.0 esa forma abreviada dispara el
   aviso `async_fn_in_trait`, activo por omisión, que la comprobación de análisis estático de la
   etapa —`cargo clippy --workspace -- -D warnings`— convierte en error. Silenciar el aviso con un
   atributo estaba disponible y se rechaza: apagar la única señal que avisa de una consecuencia real
   es peor que asumir la consecuencia con los ojos abiertos. La cota `Send` se declara ya, porque el
   consumidor de la etapa A-2 necesitará lanzar esos futuros en tareas.

7. **Los identificadores del dominio son tipos distintos y opacos**, uno por cosa identificada:
   conversación, remitente y deduplicación. Podría haber uno solo, o podrían ser cadenas. Se
   rechazan las dos opciones: confundir un remitente con una conversación tiene que ser un error de
   compilación y no un error de producción, y una cadena suelta es la vía natural por la que un
   identificador de transporte acaba dentro del dominio.

8. **`CicloDeVidaSesion` se declara como trait aparte y nunca como supertrait de `ChannelAdapter`.**
   Si fuese supertrait, el adaptador de la Cloud API tendría que implementarlo para nada y acabaría
   devolviendo errores en métodos que su transporte no necesita. Separado, sencillamente no lo
   implementa.

## Consecuencias

### Positivas

* **La frontera del dominio es verificable con una orden, no con una revisión.** Que la tabla de
  dependencias de `hexcell-core` esté vacía se comprueba en un segundo y no admite matices. Una
  regla que se comprueba sola es una regla que sigue viva dentro de seis meses.
* **La dependencia pesada aterriza donde toca.** Cuando la etapa A-2 traiga SQLite y la A-4 el
  cliente de inferencia, cada una tiene ya su crate destino, y el núcleo no se entera.
* **El canal oficial tiene sitio sin tener forma.** `hexcell-meta` reserva la frontera sin
  condicionar `adr-0013`, y los dos canales pueden convivir en células distintas sin que ninguno
  aparezca en el dominio.
* **La compilación es incremental de verdad.** Tocar la capa de persistencia no recompila el
  dominio, y con cinco unidades pequeñas el ciclo de trabajo sobre el hardware objetivo —un i7 de
  diez años— sigue siendo tolerable.

### Negativas

Se enuncian sin atenuar, porque una decisión cuyo coste se maquilla no se puede revisar después.

* **El trait `ChannelAdapter` no es compatible con objetos de trait.** Es consecuencia directa de
  devolver `impl Future` en sus métodos: `Box<dyn ChannelAdapter>` no compila. Hoy no molesta,
  porque cada célula es un proceso con exactamente un adaptador y la selección estática por
  parámetro genérico sobra y es más barata. **Pero si la etapa A-2 quiere elegir el canal en tiempo
  de ejecución a partir de la configuración, dentro de un mismo binario, tendrá que escribir un
  trait envoltorio compatible con objetos y con futuros en caja.** Queda escrito aquí para que la
  etapa A-2 lo herede en lugar de redescubrirlo con el código a medias.
* **Cinco crates para un esqueleto es fricción real.** Cinco manifiestos que mantener, cinco sitios
  donde mirar y un `Cargo.toml` raíz que sincronizar. La alternativa —un crate único que se parte
  después— es más cómoda hoy y más cara el día de la partición, cuando ya hay `use` cruzados que
  nadie escribió con mala intención.
* **La prohibición de dependencias en el núcleo se pagará alguna vez.** Habrá un momento en que la
  biblioteca cómoda esté prohibida en el único crate donde haría falta, y la salida será escribirlo
  a mano o mover el código a otro crate. Es el precio de que la frontera sea comprobable.
* **`Cargo.lock` está ignorado por `.gitignore`, y la guía de Rust recomienda versionarlo en los
  workspaces que producen binarios.** Hoy el impacto es nulo, porque no hay ni una dependencia
  externa, y `.gitignore` es entregable de otra tarea, así que esta no lo corrige. **Debe revisarse
  en cuanto entre la primera dependencia real, en la etapa A-2**; si se olvida, dos máquinas podrán
  compilar versiones distintas del mismo commit sin que nada avise.
* **La división se ha elegido antes de que exista la presión que la pondrá a prueba.** La etapa A-5,
  con las épocas de conocimiento y la conmutación atómica, es la que más va a tensar la frontera
  entre dominio y persistencia. Revisar esta división al cerrar esa etapa es parte del plan, no un
  imprevisto.

## Alternativas consideradas y descartadas

### A. Un solo crate y partirlo cuando duela

Es lo más rápido de arrancar y lo que casi todo el mundo hace. Se descarta porque la frontera que
`adr-0010` declara es justamente la que un crate único no puede sostener: dentro de una sola unidad
de compilación, "el núcleo no conoce el transporte" es una promesa que se rompe con un `use` que
nadie revisa. Además, la partición se acaba haciendo con datos de clientes de pago ya en producción,
que es el peor momento posible.

### B. `hexcell-storage` y `hexcell-meta` como módulos de `hexcell-core`

Ahorra dos manifiestos. Se descarta porque arrastraría al núcleo, el día de la primera dependencia
real, todo lo que esta decisión quiere mantener fuera de él, y convertiría el criterio de aceptación
"la tabla de dependencias del núcleo está vacía" en algo imposible de cumplir.

### C. Un único binario con subcomandos para célula y administración

Un solo programa con `hexcell run` y `hexcell admin`. Se descarta por el presupuesto de memoria y
por superficie: el contenedor de cada cliente acabaría llevando dentro el código de operación de
toda la cartera, que ahí no se usa y que no debería estar al alcance.

### D. `hexcell-meta` como esqueleto con forma anticipada

Escribir ya los tipos de webhook y de verificación de firma, "que se van a necesitar igual". Se
descarta porque `adr-0013` sigue sin resolverse y la forma del código ya escrito pesa sobre la
decisión que viene después. Un crate vacío no condiciona nada; un crate con forma, sí.

### E. Silenciar el aviso `async_fn_in_trait` con un atributo y usar la forma abreviada

La firma queda más corta y más legible. Se descarta porque el aviso señala una consecuencia real
—entre otras, que la cota `Send` del futuro no queda declarada en el contrato— y apagarlo la deja
igual de presente pero invisible. Escribir `impl Future<Output = ...> + Send` cuesta una línea más y
declara la cota que la etapa A-2 necesita.

### F. Enriquecer con datos las cuatro variantes de fallo del envío

Cargar `LimiteDeTasa` con el tiempo de espera sugerido, o `DestinatarioInvalido` con el motivo. Se
descarta **en esta etapa**: qué dato necesita cada variante lo sabe quien las consume, y ese
consumidor se escribe en la etapa A-2. Fijar hoy la forma del dato sería decidir sin el caso de uso
delante. El conjunto de variantes, en cambio, no es una decisión de implementación: lo fija FR-12 y
solo el PRD lo cambia.

## Referencias

* `docs/PRD.md`, FR-12 (puerto de canal), FR-05 (persistencia dual) y FR-11 (CLI de operación).
* `docs/adr/adr-0010-puerto-de-canal.md` — la frontera que esta división hace comprobable; esta
  etapa la **implementa**, no la reescribe.
* `docs/adr/adr-0013-entrada-publica-fase-b.md` — decisión pendiente que mantiene vacío a
  `hexcell-meta`.
* `docs/adr/adr-0014-canal-propio-permanente.md` — los dos canales conviven en células distintas.
* `docs/cotejo-puerto-de-canal-cloud-api.md` — cotejo de las variantes contra la documentación
  oficial, resolución de la discrepancia del código 131047 y hallazgo abierto sobre la familia de
  fallos de plantilla.
* `docs/plan/fase-a-1-fundaciones.md`, tareas 1, 5 y 9.
* `docs/STATUS.md` — estado del scaffold y decisiones pendientes.
