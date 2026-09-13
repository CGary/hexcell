# adr-0034 — Contrato tipado de códigos de salida y sumideros de salida en `hexcell-admin`

* **Estado:** Vigente (2026-09-13).
* **Etapa que lo produce:** A-6 (tarea 10 del plan de la etapa A-6: `docs/plan/fase-a-6-empaquetado-cli.md`, HEX-074-b, segunda de tres hijas de HEX-074).
* **Relación con otros ADR:** Ninguno anterior fija el código de salida ni la separación de flujos de un binario de este workspace; los binarios existentes (`hexcell`, `hexcell-admin` sin CLI todavía) solo usaban `ExitCode::SUCCESS` / `ExitCode::FAILURE` de forma implícita. Sigue el precedente de diseño de `EstadoDeCelula` (HEX-074-a, `crates/hexcell-admin/src/estado_de_celula.rs`): enumerado cerrado, sin `#[non_exhaustive]`, para que el código externo lo empareje de forma exhaustiva.

## Contexto

HEX-074-a cerró el vocabulario de estados de la célula en el plano de control. La tarea 10 completa,
sin embargo, sigue teniendo dos piezas pendientes que la tarea hermana HEX-074-c —el analizador de
argumentos, los seis subcomandos y el cableado de `src/main.rs`— necesita como cimiento antes de
poder devolver nada al sistema operativo:

1. **Un código de salida numérico distinguible por clase de problema.** `std::process::ExitCode`
   solo ofrece `SUCCESS` (0) y `FAILURE` (1) como constantes. Un operador que invoca mal la CLI
   (subcomando desconocido, argumento faltante) recibe hoy el mismo `1` que un fallo real de
   ejecución (Docker no responde, el contenedor no arrancó), y no hay forma de anunciar por
   convención un subcomando declarado pero todavía sin comportamiento real detrás.
2. **Una disciplina de escritura que no entre en pánico.** El perfil de publicación del workspace
   fija `panic = "abort"` (`Cargo.toml`). Las macros `println!`, `eprintln!` y `print!` entran en
   pánico si la escritura subyacente falla —el caso más común es una tubería rota, por ejemplo
   `hexcell-admin cell list | head`—, lo que convertiría un fallo de E/S trivial en un abort del
   proceso completo en vez de un código de salida limpio.

Ninguna de las dos piezas puede esperar a que exista el analizador de argumentos: ese analizador
necesita poder devolver ya un código de uso incorrecto y escribir ya sus mensajes a través de un
sumidero que no entre en pánico. Por eso esta tarea se ejecuta antes que HEX-074-c, la consume.

## Decisión

**1. `CodigoDeSalida`, enumerado cerrado de cuatro variantes con valor numérico explícito:**

```
Exito = 0
Fallo = 1
UsoIncorrecto = 2
NoImplementadoTodavia = 3
```

`Exito` es 0 porque `ExitCode::SUCCESS` lo es por convención del sistema operativo. `Fallo` es el
fallo genérico que hoy ocupa `ExitCode::FAILURE`. `UsoIncorrecto` y `NoImplementadoTodavia` son las
dos variantes nuevas: la primera distingue un error del operador (invocación incorrecta) de un
fallo de ejecución real; la segunda reserva un número propio para un subcomando anunciado en la
superficie de la CLI pero sin comportamiento implementado, en vez de hacerlo fallar con el mismo
código genérico que un error real. El enumerado no lleva `#[non_exhaustive]`: es una decisión
deliberada, igual que en `EstadoDeCelula`, para que el crate de pruebas externo
(`tests/codigo_de_salida.rs`) lo empareje de forma exhaustiva y sin brazo por defecto, de modo que
añadir o quitar una variante rompa esa compilación en vez de degradar en silencio hacia un caso
genérico.

**2. La conversión hacia el proceso pasa siempre por `ExitCode::from(u8)`:**
`impl From<CodigoDeSalida> for std::process::ExitCode` no mapea cada variante a mano contra
`ExitCode::SUCCESS` / `ExitCode::FAILURE`; llama a `ExitCode::from(codigo.codigo())`, con
`codigo()` como el único lugar donde vive el número de cada variante. Así el contrato numérico
documentado (el que un operador ve en un script de shell) y el código de salida real del proceso
no pueden divergir con el tiempo.

**3. `Salida<S, D>`, genérico sobre dos sumideros `std::io::Write` independientes:**
`estandar: S` recibe texto legible para el operador; `diagnostico: D` recibe diagnóstico. Son dos
parámetros de tipo distintos —no un único campo `Write` compartido con un indicador de a cuál
flujo escribir— para que el propio compilador impida, por construcción de la firma, mezclar ambos
flujos en un solo sumidero. `Salida::nueva` es el constructor de pruebas, que inyecta cualquier par
de escritores (típicamente dos `Vec<u8>` en memoria); `Salida::estandar()` es el único constructor
de producción, especializado sobre `Salida<Stdout, Stderr>`, y conecta los descriptores reales del
proceso. Cada método de escritura (`linea`, `diagnostico`) usa `Write::write_all` y devuelve
`io::Result<()>` sin envolver el error ni descartarlo: nunca `println!`, `eprintln!`, `print!` ni
`write!` contra la salida o el error estándar directamente, para que ningún fallo de escritura
pueda convertirse en un pánico bajo `panic = "abort"`. Quien llama decide cómo mapear ese
`io::Result` a un `CodigoDeSalida` — ese mapeo pertenece a HEX-074-c, que sí conoce el contexto de
cada subcomando.

## Alternativas consideradas y descartadas

Ninguna alternativa de diseño de esta tarea alcanza el umbral de una entrada propia en
`docs/bitacora-de-descartes.md`: no se evaluó ni descartó ningún mecanismo externo (una crate de
manejo de errores, un tipo de error dinámico `Box<dyn Error>` para el código de salida, o un
`Salida` con un único `Write` parametrizado por una bandera de flujo) que mereciera registro
propio; la elección de un enumerado cerrado con valor numérico explícito y de un sumidero genérico
sobre dos parámetros de tipo se deriva directamente del precedente ya registrado de
`EstadoDeCelula` y de la restricción de `panic = "abort"` ya vigente en el workspace.

## Consecuencias

* HEX-074-c puede devolver `CodigoDeSalida::UsoIncorrecto` ante un subcomando desconocido y
  `CodigoDeSalida::NoImplementadoTodavia` ante un subcomando declarado pero aún vacío, cada uno
  numéricamente distinguible del fallo genérico y del éxito, sin inventar más números.
* Ningún mensaje de `hexcell-admin` puede volver a entrar en pánico por una tubería rota: toda
  escritura de texto pasa por `Salida`, cuyo tipo de retorno obliga a manejar el error como valor.
* Una prueba puede capturar la salida legible para el operador y el diagnóstico por separado sin
  redirigir descriptores de archivo del sistema operativo, inyectando dos búferes en memoria a
  través de `Salida::nueva`.
* El número de cada `CodigoDeSalida` queda documentado como contrato estable: cualquier script que
  invoque `hexcell-admin` y lea `$?` puede depender de él.

## Referencias

* `crates/hexcell-admin/src/codigo_de_salida.rs`, `crates/hexcell-admin/src/salida.rs`.
* `crates/hexcell-admin/tests/codigo_de_salida.rs`, `crates/hexcell-admin/tests/salida.rs`.
* `crates/hexcell-admin/src/estado_de_celula.rs` (HEX-074-a, precedente de enumerado cerrado).
* `docs/plan/fase-a-6-empaquetado-cli.md`, tarea 10.
