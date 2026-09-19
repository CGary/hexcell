# ADR-0038: grupo `config render` en `hexcell-admin`

*Fecha: 2026-09-19 — Estado: vigente*

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
