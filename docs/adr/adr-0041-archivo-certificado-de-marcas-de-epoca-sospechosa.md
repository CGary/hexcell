# adr-0041 — Archivo certificado de marcas de época sospechosa

* **Estado:** Vigente (2026-09-30).
* **Etapa que lo produce:** A-5 (HEX-093, saneamiento de marcas de época sospechosa — hijo a: núcleo).
* **Relación con otros ADR:** **EXTIENDE** —nunca reescribe— `adr-0027-retencion-y-purga-de-epocas.md`,
  que registró la inmunidad de las marcas `.sospechosa` a la purga y la reserva permanente del
  número de la época marcada. No supersede a ningún ADR.

## Contexto

`docs/STATUS.md` mantiene pendiente la superficie del operador para el saneamiento de marcas de
época sospechosa (2026-08-31, HEX-057-b): las marcas `.sospechosa` son permanentes e inmunes a la
purga ordinaria, y `adr-0027` no definió qué hacer con una marca histórica cuando el operador
certifica que la época defectuosa ya no representa un riesgo. Este ADR cubre el **hijo a** (el
núcleo): la superficie HTTP administrativa y la capa de persistencia. El subcomando de
`hexcell-admin`, el README y el runbook pertenecen al hijo b.

La decisión que este ADR fija es doble: **qué** ocurre con la marca al archivarla (renombrado más
certificación, nunca borrado) y **qué** ocurre con el número de la época (sigue reservado).

## Decisión

1. **Archivar es renombrar y anexar, nunca borrar.** El archivo `knowledge_epoch_N.sospechosa` se
   renombra a `knowledge_epoch_N.sospechosa.archivada` y a su contenido original se **anexan** las
   tres líneas `certificacion_certifico`, `certificacion_motivo` y `certificacion_fecha_absoluta`
   (fecha absoluta ISO del día, reutilizando `reversion::fecha_absoluta_de_hoy`). El anexo usa
   `OpenOptions::append`, sin truncar y sin forzar la sincronía con el disco: el mismo criterio del
   escritor existente (`std::fs::write` más renombrado). La marca se borra **menos que nunca**: ni
   la purga, ni el archivo, ni ningún otro camino eliminan un archivo de marca en ninguna de sus
   dos formas.
2. **El número de la época archivada sigue reservado.** `numeros_de_epoca_marcados` —la fuente
   única de verdad de los números reservados— cuenta las marcas activas **y** las archivadas, de
   modo que ni `purgar_epocas_retiradas` ni `numero_de_epoca_siguiente` ni la compuerta 4b de
   `revertir_a_epoca` la liberan. Una época archivada sigue siendo un destino de reversión
   inválido (`EpocaMarcadaComoSospechosa`) y sigue sin protección de recencia: se purga
   prioritariamente igual que una marcada activa.
3. **El escaneo de épocas nunca confunde una marca archivada con una base de datos.** El
   predicado compartido `es_nombre_ajeno_al_escaneo_de_epocas` añade la cláusula
   `ends_with(".sospechosa.archivada")` a las cinco cláusulas existentes; cada sitio de escaneo
   (purga y numeración) filtra nombres por su propia función de listado que llama a ese predicado,
   y ninguna conserva el filtro viejo en línea.
4. **La certificación es obligatoria y se valida una sola vez, en la capa de persistencia.**
   `certifico` y `motivo` deben quedar no vacíos tras recortar y no contener caracteres de control
   (un salto de línea forjaría líneas del archivo de marca); el rechazo es un valor de resultado
   (`DesenlaceDeArchivoDeMarca::Rechazada`) que la ruta traduce a 400 sin tocar ningún archivo.
   El DTO HTTP usa `#[serde(default)]` para que un cuerpo sin esos campos llegue a esa compuerta en
   vez de morir en `serde`. La superficie HTTP no duplica la validación.
5. **Archivar es idempotente y seguro ante caídas.** Una marca ya archivada con certificación
   responde `sin_cambios` sin tocar el archivo. Si el proceso cae entre el renombrado y el anexo
   (archivada sin certificación), el siguiente intento solo anexa. Si existen **ambos** archivos,
   la operación aborta sin renombrar: `rename()` de POSIX sobrescribiría la evidencia certificada.
6. **El listado es tolerante.** `GET /admin/epocas/sospechosas` devuelve 200 con una entrada
   `ilegible` (y el nombre del variante de error) por cada marca corrupta o con número
   discrepante, en vez de un 500; solo un fallo de `read_dir` es un error.

## Consecuencias

* **La marca es evidencia permanente, en dos estados.** El contenido original se conserva íntegro
  al inicio del archivo archivado; la certificación viaja al final, en el mismo formato de línea
  `clave: valor` que el resto de la marca.
* **Una marca archivada corrupta bloquea purga, promoción y reversión** exactamente como lo haría
  una activa: `numeros_de_epoca_marcados` las lee a todas con el mismo parser estricto.
* **El contrato HTTP queda congelado para el hijo b**: listado con `estado` `vigente`|`archivada`|
  `ilegible` y `certificacion` nula u objeto; archivo con `resultado` `archivada`|`sin_cambios`|
  `marca_inexistente` y los códigos 200/404/400/413/500 descritos en el contrato de la tarea.
* **El hijo b (CLI, README, runbook) queda fuera de este ADR.**
* **Se descarta explícitamente borrar la marca al archivar** (D-60): la razón y la condición de
  reapertura viven en la bitácora, en el mismo commit que este ADR.

## Referencias

* `docs/adr/adr-0027-retencion-y-purga-de-epocas.md` (inmunidad de marcas y reserva de número).
* `docs/bitacora-de-descartes.md`, D-60 (borrado de la marca al archivar).
* `crates/hexcell-storage/src/retencion.rs` (archivo, listado, fuente única de números reservados).
* `crates/hexcell/src/admin.rs` (rutas `GET /admin/epocas/sospechosas` y
  `POST /admin/epocas/sospechosas/archivar`).