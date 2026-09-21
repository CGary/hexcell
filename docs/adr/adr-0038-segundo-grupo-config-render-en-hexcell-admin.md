# ADR-0038: grupo `config render` en `hexcell-admin`

*Fecha: 2026-09-19 — Estado: vigente*

Extiende `adr-0036` (contrato de análisis de argumentos y modo de simulación): añade un segundo
grupo de nivel superior (`config`) junto a `cell` sin tocar la gramática existente de `cell`.

## Contexto

La tarea 22 de la etapa A-6 (`docs/plan/fase-a-6-empaquetado-cli.md`) necesita fusionar un archivo
de valores compartidos y un archivo de superposición por célula en el archivo de entorno que
`deploy/cell.compose.yml` interpola, sin darle a `crates/hexcell` un segundo lector de
configuración y sin dejar pasar un secreto ni una clave desconocida.

## Decisión

`hexcell-admin` incorpora el grupo superior `config` y el subcomando `render`. Lee un archivo de
valores compartidos y otro de superposición en formato `KEY=VALUE`, valida ambos contra una lista
cerrada de parámetros no secretos y aplica la superposición por clave. La salida se ordena de forma
estable y solo se escribe después de validar todo.

Los errores de contenido usan `CodigoDeSalida::Fallo`; `UsoIncorrecto` queda reservado para una
invocación mal formada. `--simular` valida y cuenta las claves sin escribir.

La CLI centraliza la lectura de configuración sin introducir un segundo lector en `hexcell`. Una
clave desconocida, una credencial o un valor inválido falla cerrado. Los overlays reales deben vivir
en el repositorio privado del operador; el repositorio público solo conserva ejemplos.

## Consecuencias

Positivas: la validación de configuración corre antes del arranque de la célula («falla en el
renderizado, no en el arranque»); el esquema cerrado es el único punto que decide qué clave es
segura de exponer. Costos: el esquema de `crates/hexcell-admin/src/esquema_configuracion.rs` debe
mantenerse alineado a mano con lo que `crates/hexcell/src/configuracion.rs` realmente parsea; un
desalineamiento entre ambos reabre exactamente el defecto que corrige este mismo ciclo de revisión:
el render aceptaba un decimal para `HEXCELL_ALERTAS_SUELO_BALANCE_DISPONIBLE` que la célula rechaza
al arrancar por parsearlo como `i64`.
