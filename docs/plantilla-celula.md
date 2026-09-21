# Plantilla de arranque de una célula sobre canal propio

Nota de uso de la **plantilla de composición** de una célula sobre canal propio
(whatsmeow). Una célula sobre este canal son **dos contenedores** —núcleo Rust
y sidecar Go— que comparten una red local y un volumen
(`docs/plan/fase-a-6-empaquetado-cli.md`, etapa A-6).

## Qué archivo es qué

* **`deploy/cell.compose.yml`** — la **plantilla**. Define los dos servicios
  (`nucleo` y `sidecar`), la red local de la célula y el volumen compartido.
  Cada valor que distingue una célula de otra es una referencia `${VARIABLE}`:
  desplegar una célula nueva es proveer valores, no editar la plantilla.
* **`deploy/celula.env.ejemplo`** — el **referente de variables**. Documenta,
  con un comentario por entrada, todas las variables que la plantilla consume.
  Sus valores son marcadores seguros de versionar; no es un archivo de
  configuración.

## Cómo levantar una célula nueva

1. Copiar el referente a un archivo real, o proveer los valores desde el
   entorno del operador. Los secretos (claves de API) entran **solo** por
   variable de entorno; el referente solo documenta su existencia con un
   marcador.
2. Sustituir los marcadores por los valores per-célula: identificador, nombres
   de red y volumen, zona horaria, teléfono, claves de API y límites de
   recursos.
3. Resolver la plantilla y comprobar que todas las variables se sustituyen:

   ```sh
   docker compose --env-file deploy/celula.env.ejemplo \
     -f deploy/cell.compose.yml config
   ```

El alta completa de una célula —volúmenes, emparejamiento, salud— pertenece a
las tareas 5, 11 y 17 de la etapa A-6; esta plantilla es su punto de partida.

## Por qué `HEXCELL_DIRECCION_SALUD` está fijada a `0.0.0.0:8081`

El binario del núcleo escucha su servidor de salud por defecto en **loopback**
(`crates/hexcell/src/configuracion.rs:347-348`). Una célula empaquetada en
contenedores necesita que un **contenedor hermano** sondee `GET /health/ready`
dentro de la red de la célula, y contra `127.0.0.1` del otro contenedor ese
sondeo nunca llegaría. Por eso la plantilla fija explícitamente un bind no
loopback en lugar de confiar en el valor por omisión del código. La dirección
no es una dimensión per-célula —es el mismo bind en todas las células, y lo que
las aísla es la red de célula—, así que no se parametriza.

## Qué queda fuera de esta plantilla (y por qué no es un hueco)

* **Renderizar archivos de configuración por célula desde plantillas**: la
  tarea 22 de la etapa A-6. Aquí la plantilla y su referente son artefactos
  estáticos; no hay lógica de renderizado ni de sustitución de variables.
* **Elegir los valores de los límites de recursos y endurecer los
  contenedores**: las tareas 6 y 4 de la etapa A-6. Los límites se
  **parametrizan** en la plantilla, no se eligen aquí.
* **Levantar la célula (`docker compose up`), sondear su salud y probar el
  aislamiento entre células**: las tareas 5, 11 y 17 de la etapa A-6. Esta
  tarea es de verificación estática: `docker compose config` resuelve la
  plantilla sin crear ningún contenedor.
* **El vigilante de vida externo (dead-man's switch, HEX-077-d)**: es una
  entrada de cron **por servidor**, no por célula, e instalada fuera de esta
  plantilla. Nunca va en `deploy/celula.env.ejemplo` ni en este archivo — un
  cron por célula convertiría un anfitrión con N células en N pings idénticos
  hacia el mismo servicio externo. Ver `docs/runbook-vigilancia-externa.md`.

## Valores de referencia de memoria y tamaño de imágenes

Primera corrida manual el 2026-09-21 (commit `5d03a19`, host `pc-gary`, Docker 29.8.1,
cgroup v2), transcrita del informe de `deploy/medir_memoria_y_imagenes.sh` (HEX-079,
tarea 16 de la etapa A-6) sin retocar. La célula se levantó desde volumen vacío y **sin
dispositivo emparejado**: el sidecar no cargó ninguna sesión de WhatsApp. Cada nueva
corrida añade una fila por medición con su fecha, commit y host; nunca se sobrescribe
una fila anterior.

| Medición | Valor | Fecha | Commit | Host |
| --- | --- | --- | --- | --- |
| Memoria en reposo — `anon` agregado (núcleo+sidecar, cgroup v2) | 9 420 800 B (9,0 MiB) | 2026-09-21 | `5d03a19` | pc-gary |
| Memoria en reposo — `memory.current` agregado (incluye caché de página) | 37 400 576 B (35,7 MiB) | 2026-09-21 | `5d03a19` | pc-gary |
| Memoria bajo carga — `anon` agregado, pico (20 muestras; cota inferior) | 9 506 816 B (9,1 MiB) | 2026-09-21 | `5d03a19` | pc-gary |
| Memoria bajo carga — `memory.current` agregado, pico (20 muestras; cota inferior) | 38 293 504 B (36,5 MiB) | 2026-09-21 | `5d03a19` | pc-gary |
| Tamaño de la imagen del núcleo (`HEXCELL_IMAGEN_NUCLEO`) | 11 891 950 B (11,3 MiB) | 2026-09-21 | `5d03a19` | pc-gary |
| Tamaño de la imagen del sidecar (`HEXCELL_IMAGEN_SIDECAR`) | 30 446 271 B (29,0 MiB) | 2026-09-21 | `5d03a19` | pc-gary |

**LIMITACIÓN del generador de carga (AC-3 de HEX-079):** la cifra bajo carga
es una **cota inferior**, no el peor caso. `crates/hexcell/tests/carga.rs` no
se reutiliza como generador externo (no se modifica; cuatro razones en la
cabecera del script), y el sustituto golpea el listener de salud del núcleo:
ejercita el listener HTTP, el runtime tokio y el asignador, pero **no** la
admisión GCRA, **no** el pipeline de inferencia, **no** el motor de
conocimiento y **no** la ruta whatsmeow del sidecar. Un valor bajo carga no
ratifica por sí solo los valores provisionales de `adr-0007`.