# ADR 0030: Prueba de estrés de conmutación de época bajo lecturas concurrentes

- **Estado**: Vigente (2026-09-07)
- **Fecha**: 2026-09-07
- **Decisores**: Gary (Arquitecto de Sistemas), equipo Hexcell
- **Relaciones**:
  - Extiende [ADR 0006](adr-0006-epocas-y-conmutacion-atomica.md) (Shadow DB y conmutación atómica por épocas): la prueba ejercita la conmutación real, no un modelo de ella.
  - Consume [ADR 0029](adr-0029-motor-de-recuperacion-de-contexto.md) (Motor de recuperación de contexto): usa `recuperar_contexto` como carga de lectura y la anchura parametrizable del pool que aquel ADR introdujo para esta tarea.
  - Complementa [ADR 0027](adr-0027-retencion-y-purga-de-epocas.md) (Retención y purga) y el drenaje de `adr-0006`: la prueba cierra el ciclo completo conmutar → drenar → purgar.
  - Preserva [ADR 0003](adr-0003-persistencia-dual.md): la prueba vive en `crates/hexcell-storage`, que sigue libre de ejecutores asíncronos; toda la concurrencia es `std::thread` y `std::sync`.

---

## Contexto

`docs/PRD.md` declara como criterio de QA de la etapa A-5 una **«Prueba de Consistencia en Modo
WAL»**: conmutar la época viva mientras hay lecturas RAG simultáneas y demostrar que ninguna recibe
`SQLITE_BUSY` ni observa una época a medio construir. La tarea 11 del plan
(`docs/plan/fase-a-5-conocimiento-shadow-db.md`) la fija en 20 lecturas concurrentes.

Hasta el 2026-09-07 ese criterio estaba **declarado y no verificado**. Es la peor de las dos
situaciones posibles: un criterio ausente se nota al leer el plan; uno declarado sin ejecutar se
confunde con uno cumplido. La conmutación tenía pruebas unitarias por pieza —promoción, drenaje,
purga, recuperación—, pero ninguna las hacía coincidir en el tiempo, que es exactamente la
condición bajo la que el defecto que el criterio busca podría aparecer.

Tres hechos del árbol condicionaban cómo escribirla:

1. `PoolDeConocimiento::con_lectura` reparte las lecturas con `fetch_add % len` y a continuación
   toma un `Mutex` **bloqueante**. Con la anchura por omisión (2 conexiones) veinte lectores no son
   veinte lecturas simultáneas: son veinte lectores haciendo cola sobre dos cerrojos.
2. La medición de descriptores de archivo se hace sobre `/proc/self/fd`, que es del **proceso
   entero**, no del test.
3. `docs/bitacora-de-descartes.md` **D-33** prohíbe, como principio de diseño y sin reapertura,
   serializar la batería con `--test-threads=1` o `serial_test`.

---

## Decisión

1. **La prueba abre el pool de conocimiento con anchura 20** (`abrir_con_anchura_de_conocimiento`),
   igual al número de hilos lectores, **y lo afirma dos veces**: la anchura efectiva leída del
   gestor debe ser `>= 20` y estrictamente mayor que `CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO`, y el
   número de descriptores del proceso que apuntan al archivo de la época viva —las conexiones
   SQLite realmente abiertas sobre ella— debe ser `>= 20`.
   *Justificación*: es la única configuración bajo la que hay hasta veinte conexiones SQLite vivas
   a la vez. Con la anchura por omisión, `SQLITE_BUSY` sería imposible **por construcción** en vez
   de por corrección, y la prueba pasaría sin demostrar nada. Configurarla no basta: una anchura
   fijada por constante y nunca comprobada se puede estrechar sin que ninguna aserción se entere,
   y CI seguiría certificando en verde un criterio que ya no se ejercita. Comprobado por mutación
   el 2026-09-07: con la anchura en 2, ambas aserciones fallan por separado (anchura efectiva 2 y
   3 conexiones vivas frente a las 21 de la configuración correcta). La segunda aserción mide el
   **hecho** y no la **intención**: la primera dice lo que se pidió, la segunda lo que hay.

2. **La prueba se marca `#[ignore]` y se invoca por nombre en un paso dedicado de
   `.github/workflows/ci.yml`.** Las dos mitades son obligatorias.
   *Justificación*: `#[ignore]` la saca de `cargo test --workspace`, donde competiría por CPU con
   los demás binarios y donde la medición de descriptores del proceso sería ruido. El paso dedicado
   es lo que impide que el `#[ignore]` degenere en una prueba que existe y nunca corre. La
   propiedad de la que depende el aislamiento está verificada: `cargo` ejecuta los binarios de test
   de integración **secuencialmente**, y cada `tests/*.rs` es su propio binario y su propio
   proceso; al ser el único test de su archivo, cuando corre no hay ningún otro test vivo en su
   proceso. No se serializa nada: D-33 sigue intacto.

3. **La procedencia de cada lectura se verifica por contenido, con marcadores de época.** La época
   previa se siembra con fragmentos marcados `EPOCA-UNO` y la nueva con `EPOCA-DOS`. La prueba
   afirma dos cosas distintas: que **ambos** marcadores se observaron durante la corrida (prueba de
   que hubo solapamiento real) y que **ningún** resultado devuelto mezcló marcadores.
   *Justificación*: `recuperar_contexto` no expone el número de época que sirvió un resultado. Sin
   un marcador, una lectura servida por una época a medio construir sería indistinguible de una
   correcta, y la invariante «ninguna lectura observa una época parcial» quedaría comprobada por
   suerte. El marcador la hace **detectable**.

4. **Las dos duraciones se miden y se reportan por separado, y NFR-03 NO se re-certifica aquí.**
   `DesenlaceDePromocion::Promovida.duracion_de_conmutacion_ms` abarca el intercambio del `ArcSwap`
   **más** la toma de un cerrojo de lectura del pool nuevo y la consulta de vitalidad que sirve la
   primera lectura de la época nueva (`promocion.rs`, pasos 5 y siguientes). Ese tramo —de la
   reasignación del puntero a la primera lectura servida— es exactamente el que NFR-03 define, así
   que es el campo correcto para el requisito. La prueba mide además, con su propio `Instant`, el
   intervalo desde la invocación de `promover_epoca` hasta esa primera lectura, que incluye
   revalidación, sellado, punto de control, renombrado y apertura del pool nuevo, y lo reporta
   aparte. La prueba de estrés afirma sobre la medición estrecha únicamente un **techo de regresión
   catastrófica** de 1000 ms, cuyo propósito declarado es detectar que la conmutación empezó a
   *esperar* por algo (E/S, convoy de cerrojos), no certificar NFR-03.
   *Justificación*: ser el campo correcto para NFR-03 es justo lo que lo vuelve el objeto
   equivocado para acotarlo **bajo contención deliberada**. La consulta de vitalidad tiene que
   ganarle un cerrojo del pool a veinte hilos que lo están saturando a propósito; en un runner de
   dos núcleos con sobresuscripción 20:2, una sola expropiación del planificador rompe un muro de
   10 ms, y la latencia de cola no es proporcional a la media. Sería una intermitencia cableada en
   CI que estallaría semanas después sobre trabajo ajeno. NFR-03 ya está certificado, y estricto,
   en `tests/promocion.rs:377`, que no lanza ningún hilo: esa es la condición no contendida y
   parecida a producción que el requisito describe, y esta tarea no la toca. Duplicar el muro bajo
   una contención que el requisito nunca contempló no compraba certeza adicional y pagaba
   fragilidad por ella. El techo de 1000 ms se fija sobre datos, y su justificación tiene dos lados
   que apuntan en la misma dirección: **el techo queda muy por encima del peor caso observado**
   —44 corridas medidas el 2026-09-07 (20 sin restricción, 12 fijadas a dos núcleos, 12 fijadas a
   dos núcleos con carga externa) dan un peor caso de 0,047 ms, así que el techo lo supera unas
   21.000 veces y ninguna expropiación del planificador lo alcanza—, y **el techo queda además muy
   por encima de la secuencia de promoción entera**, que en esta máquina tarda entre 88 y 140 ms con
   la revalidación de índice incluida. Ese segundo lado es el que conserva la capacidad de
   detección: superar el techo significaría que el intercambio del puntero, una parte diminuta de la
   promoción, tardó más de siete veces lo que tarda la promoción completa. Eso no es una latencia
   peor, es un **cambio de clase**: la conmutación dejó de calcular y pasó a esperar (E/S, convoy de
   cerrojos, espera de red). Un techo generoso no es un techo ciego mientras la magnitud que vigila
   y la magnitud que toleraría el ruido estén separadas por cuatro órdenes de magnitud, como aquí.
   Decisión humana del 2026-09-07; el descarte del muro estricto queda en D-37.

5. **El fixture `preparar_staging_valido` se promueve a `tests/comun/mod.rs`** y `tests/promocion.rs`
   y `tests/drenaje.rs` lo consumen desde allí. El fixture multi-fragmento con marcadores de la
   prueba de estrés queda **privado** de su archivo.
   *Justificación*: el primero estaba duplicado literalmente en dos archivos y la tarea iba a
   añadir un tercer uso. El segundo no es una copia del primero sino otro fixture, con otro fin
   —barrido medible y procedencia verificable—, que ninguna otra prueba necesita: promoverlo sería
   compartir por parecido, no por uso.

6. **La cuenta de descriptores se toma después de una purga en vacío previa.** La línea base se
   registra tras ejecutar una vez `purgar_epocas_retiradas` sobre un estado en el que no hay nada
   que purgar.
   *Justificación*: el VFS unix de SQLite no cierra de inmediato el descriptor de un archivo sobre
   el que otra conexión del mismo proceso mantiene cerrojos POSIX —cerrarlo borraría los cerrojos
   ajenos, el defecto histórico de `close()`—, sino que lo aparca por inodo y lo reutiliza en la
   apertura siguiente. La primera conexión transitoria sobre un inodo deja allí un descriptor
   aparcado; las siguientes no. Sin la purga previa, la línea base se tomaría en frío y la cuenta
   final en caliente, y la aserción estaría midiendo el calentamiento de esa caché en lugar del
   ciclo de vida de los pools. Medido el 2026-09-07: 52 descriptores en ambos extremos, estable en
   ocho corridas consecutivas.

---

## Consecuencias

* El criterio de QA de la etapa A-5 pasa de declarado a **verificado en cada empuje**. Un `#[ignore]`
  sin paso de CI habría dejado escrito lo contrario de lo que ocurría.
* La prueba no es un test de regresión de latencia: las cifras de conmutación dependen del host. Lo
  que sí es invariante y sí se afirma es lo cualitativo —cero contenciones, cero lecturas fallidas,
  cero resultados con épocas mezcladas, cero diarios huérfanos y descriptores de vuelta en su línea
  base—. Las latencias se imprimen con `--nocapture` para lectura humana.
* Depende de Linux por `/proc/self/fd`, igual que `crates/hexcell/tests/rss_linea_base.rs`. El árbol
  ya asume Linux como destino de despliegue y de CI.
* Si un día el aislamiento por binario dejara de bastar —por ejemplo, si `cargo` pasara a ejecutar
  binarios de test en paralelo—, la aserción de descriptores fallaría de forma visible en vez de
  degradarse en silencio; ese es el modo de fallo elegido.
* Alternativas evaluadas y descartadas: **D-35**, **D-36** y **D-37** en `docs/bitacora-de-descartes.md`.
