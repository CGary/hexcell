# ADR 0026: Reversión de épocas condicionada por re-chequeo estructural y sonda semántica, y guardas de fallo silencioso

- **Estado**: Vigente (2026-08-31)
- **Fecha**: 2026-08-31
- **Decisores**: Gary (Arquitecto de Sistemas), equipo Hexcell
- **Relaciones**:
  - **EXTIENDE** —nunca reescribe— [ADR 0006](adr-0006-epocas-y-conmutacion-atomica.md) (Shadow DB con
    conmutación atómica por épocas): `adr-0006` está acotado a la promoción hacia adelante y declara
    explícitamente que no existe época previa a la que revertir. Este ADR añade ese camino sin tocar
    una sola línea del anterior, siguiendo el precedente de `adr-0022`.
  - Complementa [ADR 0003](adr-0003-persistencia-dual.md) (Persistencia dual SQLite)

---

## Contexto

El mecanismo de épocas introducido en `adr-0006` garantiza la promoción atómica hacia adelante desde una base en sombra (`knowledge_staging.db`) hacia una época sellada inmutable (`knowledge_epoch_N.db`), actualizando el puntero `knowledge_live.db` mediante un enlace simbólico relativo atómico y conmutando el pool en memoria vía `ArcSwap`.

No obstante, la operación en producción requiere la capacidad de retornar a una época sellada previa (reversión o *rollback*) ante degradaciones semánticas detectadas a posteriori o incidencias en el catálogo. La arquitectura debe resolver tres desafíos críticos:

1. **Reutilización de identidad vs. Re-acuñación**: Re-promover una época vieja como época $N+1$ duplicaría archivos en disco y crearía ambigüedad sobre la procedencia de los embeddings. La reversión debe reutilizar el número ordinal y el archivo existente en disco (`knowledge_epoch_N.db`), sin incrementar contadores ni reescribir metadatos.
2. **Condicionalidad estricta y partición disjunta**: Antes de conmutar, la época destino debe superar tanto la auditoría de integridad estructural de sus índices SQLite como la evaluación de similitud coseno de su sonda semántica persistida (`sonda_semantica`). Los fallos deben clasificarse de forma mutuamente excluyente y determinista entre anomalías estructurales e insuficiencia semántica para garantizar la mutabilidad aislada en pruebas.
3. **Guardas contra defectos de fallo silencioso**:
   - *Enlace vivo colgante*: Si `knowledge_live.db` es un symlink apuntando a un archivo que no existe, una llamada a `Connection::open` en modo lectura-escritura sigue el enlace y crea silenciosamente una base vacía de 40.960 bytes, certificando erróneamente vitalidad sana.
   - *Resolución silenciosa de ruta previa*: Utilizar `.unwrap_or(ruta_de_apertura)` ante un error de `canonicalize` oculta roturas de enlaces y hace que el drenaje verifique el diario WAL del archivo incorrecto.

---

## Decisión

1. **Secuencia de reversión de época**:
   - Se implementa `revertir_a_epoca` en `crates/hexcell-storage/src/reversion.rs` y su orquestación asíncrona `revertir_epoca_de_conocimiento` en `crates/hexcell/src/promocion.rs`.
   - Adquiere la compuerta atómica `gestor.iniciar_promocion()` compartiendo exclusión mutua con `promover_epoca`.
   - Valida que la época destino no sea la que ya está actualmente activa en producción (`EpocaYaEsLaViva`).
   - Lee la sonda semántica persistida en el archivo destino mediante `leer_sonda_semantica`.
   - Ejecuta `validar_integridad_del_indice` y aplica la función exhaustiva `es_motivo_semantico` para particionar los motivos de rechazo: si hay cualquier defecto estructural retorna `IntegridadEstructuralRechazada`; si solo hay fallos de similitud retorna `SondaSemanticaRechazada`.
   - Reasigna atómicamente el enlace simbólico `knowledge_live.db` mediante `reasignar_enlace_simbolico_vivo`.
   - Conmuta atómicamente el pool de conexiones en memoria vía `ArcSwap` e instrumenta la latencia NFR-03 (sub-10 ms).

2. **Guarda de enlace vivo colgante (guarda 3)**:
   - Se añade `verificar_enlace_vivo_resoluble` en `crates/hexcell-storage/src/pools.rs`, ejecutada en `GestorDePools::abrir` antes de abrir `ruta_conocimiento` en modo lectura-escritura.
   - Si `knowledge_live.db` es un enlace simbólico cuyo destino no existe, aborta de inmediato con `ErrorDeAlmacen::EnlaceVivoColgante { ruta, destino }` sin crear archivos ni invocar `Connection::open`.
   - Se mantiene deliberadamente fuera de `abrir_solo_lectura` (que ya falla limpiamente bajo SQLite) y de `promover_epoca` para mantener conjuntos de fallo disjuntos.

3. **Guarda de canonicalización ruidosa (guarda 4)**:
   - En `promover_epoca`, la resolución canónica de la ruta viva previa antes de la conmutación reemplaza el antiguo `.unwrap_or(ruta_de_apertura)` por un mapeo explícito de error a `ErrorDeAlmacen::ArchivoDeEpocaInaccesible`.
   - El aborto en este punto es limpio y reintentable, pues `knowledge_staging.db` permanece intacto y `numero_de_epoca_siguiente` omite archivos de staging por nombre.

---

## Consecuencias

### Positivas
- **Invariante de inmutabilidad y trazabilidad**: Las épocas selladas conservan su número original y su archivo físico, preservando la correspondencia exacta entre documentos, fragmentos y vectores.
- **Inercia estricta ante rechazo**: Ninguna reversión rechazada modifica el enlace simbólico ni conmuta el pool en memoria; el sistema continúa sirviendo la época viva previa.
- **Prevención de bases fantasma**: Desaparece el riesgo de crear bases vacías no migradas de 40.960 bytes ante symlinks rotos.
- **Trazabilidad de diario WAL**: El descriptor superseído retiene la ruta física canónica de la época reemplazada, garantizando que el drenaje verifique el WAL del archivo correcto.

### Negativas / Mitigaciones
- **Inspección offline síncrona**: La reversión audita la época destino en disco antes de conmutar; mitigado porque las épocas selladas son locales y la verificación es del orden de pocos milisegundos.
