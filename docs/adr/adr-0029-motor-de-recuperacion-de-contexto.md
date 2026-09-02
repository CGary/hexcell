# ADR 0029: Motor de recuperación de contexto RAG y frontera de prompt

- **Estado**: Vigente (2026-09-02)
- **Fecha**: 2026-09-02
- **Decisores**: Gary (Arquitecto de Sistemas), equipo Hexcell
- **Relaciones**:
  - Extiende [ADR 0006](adr-0006-epocas-y-conmutacion-atomica.md) (Shadow DB y conmutación por épocas): el motor consume la época viva expuesta por `GestorDePools::conocimiento()`.
  - Complementa [ADR 0025](adr-0025-puerto-de-embeddings.md) (Puerto de embeddings): consume vectores de incrustación comparándolos con el coseno de `hexcell-core`.
  - Preserva [ADR 0002](adr-0002-estructura-workspace.md) (Estructura de workspace): los tipos de valor del contexto viven en `hexcell-core` con la tabla de dependencias vacía (`std` únicamente).

---

## Contexto

La etapa A-5 (motor de conocimiento) requiere seleccionar los fragmentos más relevantes de la época viva de conocimiento ante una consulta de usuario en el flujo RAG (tarea 9). El motor de recuperación debe operar de forma síncrona dentro de `crates/hexcell-storage`, que es una biblioteca libre de ejecutores asíncronos (`adr-0003`).

Se presentaron dos decisiones de diseño clave con implicaciones de arquitectura y seguridad:

1. **Gestión de vectores de fragmentos corruptos o incomparables en una época viva**:
    SQLite permite almacenar cualquier BLOB en `vectores_de_fragmento` cuyo tamaño en bytes sea múltiplo de 4, sin garantizar que su dimensión coincida con la declarada en `metadatos_de_epoca` ni que sus componentes formen un vector con norma no nula o valores finitos (`0002-esquema-de-conocimiento.sql`). En el validador offline (`validar_integridad_del_indice`), las filas incomparables se cuentan como fallos para rechazar la promoción de la base. Sin embargo, en una época ya viva, surgir la pregunta de qué hacer si la recuperación topa con un vector que produce `None` en `similitud_coseno`.

2. **Formato y frontera del resultado devuelto**:
   El motor podía ensamblar directamente una cadena de texto pre-formateada con las citas del contexto listas para inyectar en el prompt del modelo de lenguaje, o bien devolver una estructura de datos fuertemente tipada con los fragmentos y sus metadatos separados.

3. **Parametrización de la anchura del pool de conexiones de lectura**:
   `PoolDeConocimiento` nacía acotado a 2 conexiones de lectura en solo lectura (`CONEXIONES_DE_LECTURA_DE_CONOCIMIENTO`). Las pruebas de estrés y conmutación de la tarea 11 del plan requieren evaluar la degradación bajo 20 lecturas RAG simultáneas, lo que sobrepasarían las 2 conexiones fijas produciendo colas de espera en los cerrojos.

---

## Decisión

1. **Aborto estricto con error nombrado (`VectorDeFragmentoIncomparable`) ante vectores incomparables**:
   Si durante el barrido de la época viva un vector almacenado no se puede decodificar (`VectorDeEmbedding::desde_bytes_le`) o su similitud coseno devuelve `None` (mismatch de dimensión interna, norma cero, valores NaN o infinitos), la llamada a `recuperar_contexto` aborta inmediatamente con `ErrorDeAlmacen::VectorDeFragmentoIncomparable { id_fragmento }`. **Nunca se omite en silencio y nunca se puntúa como cero**.
   *Justificación*: Una época viva que contenga un vector incomparable delata que la compuerta de validación de integridad (`validar_integridad_del_indice`) fue omitida upstream. Omitir el fragmento de forma silenciosa degradaría la precisión del motor de búsqueda de forma indetectable y sin emitir ninguna señal operativa.

2. **Resultado devuelto como tipo estructurado del dominio (`ContextoRecuperado`) sin ensamblado de prompt**:
   El motor devuelve una instancia de `ContextoRecuperado` (declarado en `hexcell-core/src/recuperacion.rs`), que encapsula la lista de `FragmentoRecuperado` conteniendo `id_fragmento`, `texto` y `similitud`. Ningún tipo ni función del núcleo o del almacenamiento concatena los textos en un prompt.
   *Justificación*: Mantener el texto del cliente y el conocimiento recuperado separados como tipos de datos independientes establece una frontera de seguridad explícita contra inyecciones de prompt (*prompt-injection boundary*), facilita la observabilidad de las citas en los registros y permite probar el motor de búsqueda en aislamiento sin depender del formato del prompt de inferencia.

3. **Comprobación dimensional previa al escaneo (`DimensionDeConsultaDiscrepante`)**:
   La dimensión del vector de consulta se compara contra `dimension_de_embedding` de `metadatos_de_epoca` **antes** de preparar o ejecutar la consulta a las tablas de fragmentos. Si discrepa, se retorna de inmediato `ErrorDeAlmacen::DimensionDeConsultaDiscrepante`.

4. **Anchura del pool de conocimiento aditiva y parametrizable**:
   Se introducen los constructores `PoolDeConocimiento::abrir_sobre_con_anchura(ruta, anchura)` y `GestorDePools::abrir_con_anchura_de_conocimiento(ruta_datos, anchura)`, manteniendo los constructores originales delegando en el valor por omisión de 2 conexiones. La anchura configurada se propaga en los puntos de conmutación de `promocion.rs` y `reversion.rs`, permitiendo que la tarea 11 mida concurrencia real sin modificar los módulos existentes.

---

## Consecuencias

### Positivas
- **Inmunidad a la degradación silenciosa**: Un índice corrupto en producción se detecta inmediatamente en la primera consulta que toca el fragmento defectuoso.
- **Frontera de seguridad y observabilidad limpia**: Los adaptadores de inferencia reciben datos estructurados y son los únicos responsables de formatear el prompt.
- **Parametrización transparente**: `GestorDePools` puede ajustar su pool de lecturas para mediciones de rendimiento sin afectar a ningún consumidor existente.
- **Filtro temprano eficiente**: Consultas con vectores de dimensión errónea fallan en tiempo O(1) sin realizar I/O sobre las tablas de fragmentos.

### Negativas / Mitigaciones
- **Disponibilidad de la época ante corrupción de una sola fila**: Si un fragmento corrupto llega a la época viva, las consultas que escanean ese fragmento fallan. Se acepta por diseño: la solución es revertir a la época anterior (`revertir_a_epoca`) o volver a promover una base de staging válida.
