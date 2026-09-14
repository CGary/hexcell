# adr-0036 — Contrato de análisis de argumentos, subcomandos y modo de simulación en `hexcell-admin`

* **Estado:** Vigente (2026-09-13).
* **Etapa que lo produce:** A-6 (tarea 10-c del plan de la etapa A-6:
  `docs/plan/fase-a-6-empaquetado-cli.md`, HEX-074-c, tercera de tres hijas de HEX-074).
* **Relación con otros ADR:** **CONSUME** —nunca reescribe— `adr-0034`, que fijó el
  vocabulario cerrado de `CodigoDeSalida` y los sumideros tipados de `Salida`. Este ADR
  declara cómo el analizador de argumentos y el despacho de subcomandos usan ese
  vocabulario y esos sumideros, sin reabrir ninguna de sus decisiones. Sigue el precedente
  de diseño de `EstadoDeCelula` (HEX-074-a) y `CodigoDeSalida` (HEX-074-b): enumerados
  cerrados, sin `#[non_exhaustive]`, para que el crate de pruebas externo los empareje de
  forma exhaustiva y sin brazo por defecto.

## Contexto

HEX-074-a y HEX-074-b entregaron el agregado de estado de la célula, los códigos de
salida y los sumideros tipados. Lo que faltaba para que `hexcell-admin` dejara de
imprimir el talón de la etapa A-1 era el analizador de argumentos, los seis subcomandos
declarados y el cableado de `src/main.rs`. Este ADR fija el contrato de esas tres piezas
y el modo de simulación, de modo que cualquier implementación futura —las tareas 11 a 15
del plan de la etapa A-6, que darán comportamiento real a cada subcomando— tenga una
superficie estable sobre la que construir.

La decisión de fondo es que `hexcell-admin` no adopta ninguna biblioteca externa de
análisis de argumentos. El workspace viene de una línea de minimización de dependencias
abierta por `adr-0019`, y el precedente de `crates/hexcell/src/emparejar.rs` ya mostraba
que un analizador a mano sobre `std::env::args` basta para la superficie de CLI que este
proyecto necesita. El descarte de `clap`, `argh`, `pico-args` y `structopt` queda
registrado en `docs/bitacora-de-descartes.md` como **D-53**, en el mismo commit que este
ADR.

## Decisión

**1. Analizador puro sobre `&[String]`, sin I/O ni lectura de `std::env`.** El módulo
`crates/hexcell-admin/src/argumentos.rs` expone `analizar(argumentos: &[String]) ->
Result<Invocacion, ErrorDeArgumentos>`, una función pura que recibe la porción de
argumentos ya recortada (sin `argv[0]`). El único punto del proceso que toca
`std::env::args` es `src/main.rs`. Esta separación es la que permite que el crate de
pruebas externo conduzca el analizador con un `Vec<String>` propio, sin redirigir
descriptores de archivo ni depender del entorno del proceso.

**2. Gramática cerrada, sin `#[non_exhaustive]`.** El único grupo de nivel superior es
`cell`. Los seis subcomandos admitidos son `pause`, `unpause`, `terminate`, `rebind`,
`list` y `status`, cada uno con su nombre de cable centralizado en
`Subcomando::nombre_en_cli`. El enumerado `Subcomando` no lleva `#[non_exhaustive]`: las
pruebas externas lo emparejan sin brazo por defecto, de modo que añadir o quitar una
variante rompe esa compilación.

**3. Reglas de opciones por subcomando, validadas antes del despacho.** `--id
<cell_id>` es obligatorio para `pause`, `unpause`, `terminate`, `rebind` y `status`, y
se rechaza en `list`. `--motivo <texto>` es obligatorio sólo para `rebind` y se rechaza
en los demás. `--confirmar` es obligatoria para `terminate` y `rebind` (destructivos) y
se rechaza en los demás. `--simular` es admitido por los seis. Tanto `--clave valor`
como `--clave=valor` se aceptan para `--id` y `--motivo`, siguiendo el precedente de
`crates/hexcell/src/emparejar.rs`. Un valor vacío, una opción repetida, una opción
desconocida, una opción no admitida por el subcomando y un argumento posicional
sobrante se rechazan en vez de resolverse en silencio.

**4. Confirmación por bandera, no por prompt interactivo.** La confirmación de los dos
subcomandos destructivos se expresa con la bandera `--confirmar`, no con una lectura de
`stdin`. Un prompt interactivo sería un efecto lateral incompatible con el modo de
simulación —que por contrato no toca ningún descriptor de archivo— y haría el
subcomando imposible de ejercitar fuera de una TTY.

**5. Tabla de desenlaces de `comandos::ejecutar`, cerrada aquí.** La función recibe el
`Result<Invocacion, ErrorDeArgumentos>` del analizador y un `&mut Salida<S, D>`
genérico sobre los dos escritores, y devuelve un `CodigoDeSalida`:

* Error de análisis: el `Display` en español del error más `TEXTO_DE_USO` se escriben
  en el sumidero de diagnóstico y se devuelve `UsoIncorrecto` (2).
* Invocación válida con `--simular`: una única línea en español con la acción
  planificada, el identificador de célula y el estado objetivo se escribe en el
  sumidero estándar y se devuelve `Exito` (0). No se abre ningún socket, no se crea
  ningún archivo y no se muta ningún estado.
* Invocación válida sin `--simular`: un aviso en español que nombra el subcomando y la
  tarea del plan a la que pertenece su implementación real se escribe en el sumidero de
  diagnóstico y se devuelve `NoImplementadoTodavia` (3).
* Cualquier `io::Error` de los sumideros se convierte en `Fallo` (1).

`ejecutar` no recibe un `ClienteDocker`, ni una ruta de sistema de archivos, ni un reloj
y ninguna asa de red: esa es la propiedad que hace que «no hay efecto lateral» sea una
consecuencia de la firma y no del resultado de una revisión de código.

**6. Estado objetivo nombrado, transición no aplicada.** La función
`comandos::estado_objetivo(Subcomando) -> Option<EstadoDeCelula>` es una función total
con coincidencia exhaustiva de seis brazos: `Pausar` a `Suspendida`, `Reanudar` a
`EnEjecucion`, `Retirar` a `Retirada`, `Reemparejar` a `Reemparejando`, `Listar` y
`Estado` a `None`. No construye ningún `CicloDeVidaDeCelula` ni aplica ninguna
transición: el estado actual de la célula es incognoscible sin el almacén de estado del
plano de control, que es una entrega diferida de la etapa A-6, así que esta función
sólo nombra el destino. El estado terminal del plano de control es
`EstadoDeCelula::Retirada`; el literal `desvinculada_sesion_cerrada` —que en
`docs/protocolo-ipc-nucleo-sidecar.md` es una `causa` que proyecta al estado de sesión
`desvinculada` del sidecar, no un estado de la célula— no aparece en ningún sitio de
`crates/hexcell-admin`.

## Consecuencias

* Las tareas 11 a 15 del plan de la etapa A-6 pueden implementar el comportamiento real de cada
  subcomando contra una superficie estable: el analizador, la tabla de desenlaces y la función de
  estado objetivo ya están cerrados y ejercitados por pruebas externas.
* `src/main.rs` deja de imprimir el talón de la etapa A-1 y se convierte en una raíz de composición
  delgada: recoge `std::env::args`, llama a `argumentos::analizar`, construye `Salida::estandar`,
  llama a `comandos::ejecutar` y devuelve `ExitCode::from(codigo)`.
* El modo de simulación es una propiedad de la firma de `ejecutar`, no de una revisión de código:
  la función no recibe ningún recurso externo, así que no puede tener efectos laterales por
  construcción.
* Las pruebas externas pueden conducir el analizador y el despacho con búferes en memoria, sin
  redirigir descriptores de archivo del sistema operativo, y asertar tanto el código de salida como
  los bytes exactos que caen en cada sumidero.

## Referencias

* `crates/hexcell-admin/src/argumentos.rs`, `crates/hexcell-admin/src/comandos.rs`,
  `crates/hexcell-admin/src/main.rs`, `crates/hexcell-admin/src/lib.rs`.
* `crates/hexcell-admin/tests/argumentos.rs`, `crates/hexcell-admin/tests/comandos.rs`.
* `crates/hexcell-admin/src/codigo_de_salida.rs`, `crates/hexcell-admin/src/salida.rs`,
  `crates/hexcell-admin/src/estado_de_celula.rs` (HEX-074-a y HEX-074-b, contratos consumidos sin
  modificación).
* `docs/adr/adr-0034-contrato-de-codigos-de-salida-y-sumideros-de-salida.md` (mecanismo consumido).
* `docs/bitacora-de-descartes.md`, **D-53** (descarte de `clap`, `argh`, `pico-args` y `structopt`).
* `docs/plan/fase-a-6-empaquetado-cli.md`, tarea 10-c.
