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

4. **Las dos duraciones se miden por separado y el presupuesto de NFR-03 se contrasta contra la
   estrecha.** `DesenlaceDePromocion::Promovida.duracion_de_conmutacion_ms` mide el intercambio del
   `ArcSwap` y su lectura de vitalidad, y solo ese campo se compara con los 10 ms de NFR-03. La
   prueba mide además, con su propio `Instant`, el intervalo desde la invocación de `promover_epoca`
   hasta que devuelve la primera lectura servida por la época nueva, y lo reporta aparte.
   *Justificación*: la medición ancha incluye el sellado, el punto de control, el renombrado y la
   apertura del pool nuevo. Contrastar NFR-03 contra ella acusaría de incumplimiento a un requisito
   que no cubre ese trabajo; contrastarlo solo contra ella y llamarlo «conmutación» sería medir una
   cosa y afirmar otra.

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
* Alternativas evaluadas y descartadas: **D-35** y **D-36** en `docs/bitacora-de-descartes.md`.
