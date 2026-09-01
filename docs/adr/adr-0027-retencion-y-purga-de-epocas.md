# ADR 0027: Retención y purga de épocas selladas, registro de épocas en uso con constancia no falsificable y reserva de número por marca sospechosa

- **Estado**: Vigente (2026-08-31)
- **Fecha**: 2026-08-31
- **Decisores**: Gary (Arquitecto de Sistemas), equipo Hexcell
- **Relaciones**:
  - **EXTIENDE** —nunca reescribe— [ADR 0006](adr-0006-epocas-y-conmutacion-atomica.md) (Shadow DB con conmutación atómica por épocas) y [ADR 0026](adr-0026-reversion-de-epocas-y-guardas-de-fallo-silencioso.md) (Reversión de épocas y guardas de fallo silencioso).
  - Complementa [ADR 0003](adr-0003-persistencia-dual.md) (Persistencia dual SQLite).

---

## Contexto

El almacenamiento por épocas inmutables introducido en `adr-0006` y complementado en `adr-0026` genera un nuevo archivo sellado (`knowledge_epoch_N.db`) en cada ciclo de promoción. Con el transcurso del tiempo y múltiples reingestas o reversiones, acumular indefinidamente archivos de épocas selladas saturaría el almacenamiento del hardware objetivo (equipos de bajo coste y disco limitado).

Sin embargo, la doctrina fundamental del proyecto frente a anomalías en persistencia es **verificar-y-abortar**, prohibiendo expresamente la eliminación o truncado ciego de archivos de base de datos. La purga de archivos antiguos requiere una excepción estrictamente acotada, gobernada por cercas estructurales y garantías de no-purga absolutas para no comprometer la integridad del sistema.

---

## Decisión

1. **Excepción acotada a la doctrina de no-borrado sujeta a cuatro cercas estructurales**:
   - **Cerca 1 (Localización exclusiva)**: Las operaciones de eliminación de archivos (`remove_file`) para épocas selladas existen únicamente en el módulo `crates/hexcell-storage/src/retencion.rs`. Ningún otro módulo de persistencia ni de orquestación tiene permitido eliminar archivos de época.
   - **Cerca 2 (Identificación positiva por número intrínseco)**: Solo se consideran candidatas a purga aquellas bases de datos cuyos metadatos internos (`metadatos_de_epoca`) certifiquen positivamente que se trata de una época sellada con número intrínseco válido.
   - **Cerca 3 (Preservación de evidencia de transacciones)**: Si una época candidata posee un archivo `-wal` con tamaño mayor a cero bytes (datos no consolidados), la purga se abstiene de eliminarla y la conserva como evidencia diagnóstica (`MotivoDeConservacion::DiarioConDatosSinConsolidar`). Solo se eliminan archivos `-wal` de cero bytes y archivos `-shm` residuales benignos.
   - **Cerca 4 (Inmunidad absoluta de marcas y sesgo a sobre-retención)**: Los archivos `.sospechosa` jamás se eliminan; si una época está registrada en `epocas_en_uso` por falta de drenaje, sobrevive indefinidamente (`SuperseidaSinDrenar`).

2. **Registro `epocas_en_uso` gobernado por `ConstanciaDeDrenaje` no falsificable**:
   - `GestorDePools` mantiene en memoria el inventario `epocas_en_uso: Mutex<BTreeMap<i64, PathBuf>>`.
   - Las promociones y reversiones registran en dicho mapa el número intrínseco y la ruta canónica de la época superseída.
   - El **único** camino para retirar una época del registro es invocar `retirar_epoca_en_uso(&ConstanciaDeDrenaje)`.
   - `ConstanciaDeDrenaje` es un *Value Object* con campos privados y constructor `pub(crate) fn nueva`, no clonable (`!Clone`), garantizando que no pueda ser falsificada por consumidores externos ni reutilizada.

3. **Marcas de época sospechosa (`.sospechosa`) y reserva de número ordinal**:
   - Cuando una reversión conmuta hacia una época anterior, escribe de forma síncrona el archivo `knowledge_epoch_N.sospechosa` **antes** de reasignar el enlace simbólico (D-32).
   - La marca graba en su contenido el número intrínseco, motivo y fecha absoluta. Cualquier discrepancia entre el nombre del archivo y su contenido aborta la purga (`NumeroDeMarcaDiscrepante`).
   - Una época marcada como sospechosa pierde el beneficio de la ventana de retención (se purga prioritariamente), pero su número queda reservado de forma permanente: `numero_de_epoca_siguiente` calcula el máximo entre épocas selladas y épocas marcadas, impidiendo que una promoción posterior re-acuñe un número que fue revertido por defecto.

4. **Configuración de ventana de retención**:
   - La cantidad de épocas retenidas fuera de la viva se parametriza opcionalmente mediante la variable de entorno `HEXCELL_VENTANA_DE_RETENCION_DE_EPOCAS`, con un valor por omisión de 2 (`VENTANA_DE_RETENCION_DE_EPOCAS_POR_DEFECTO`).

---

## Consecuencias

### Positivas
- **Agotamiento de disco controlado**: Las épocas obsoletas se eliminan ordenadamente en producción sin acumular gigabytes de bases de datos históricas.
- **Cero riesgo de purga prematura**: Las épocas vivas, las que siguen siendo leídas por lectores lentos sin drenar y las que conservan diarios WAL con transacciones pendientes están blindadas contra cualquier purga.
- **Invariante de no-reutilización de números sospechosos**: Ninguna época defectuosa revertida puede ver su número reasignado tras ser purgada de disco.

### Negativas / Mitigaciones
- **Retención prolongada ante drenajes fallidos**: Si una época superseída nunca alcanza el reposo (lectores colgados), ocupará espacio indefinidamente en disco; mitigado porque esto hace visible la anomalía en vez de ocultarla destruyendo datos en uso.
