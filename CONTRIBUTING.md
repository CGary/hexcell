# Guía de contribución

Este proyecto se documenta y se desarrolla en **español**, incluidos los mensajes de commit. Antes de
tocar código o documentación, revisa la jerarquía documental de `CLAUDE.md`: ante contradicciones,
manda `docs/PRD.md`, luego `README.md`, luego `docs/plan/`, luego `docs/STATUS.md`, luego
`docs/adr/README.md`, y por último `docs/bitacora-de-descartes.md`.

## Ramas

* **`main`**: rama estable. Todo cambio llega por revisión, nunca por commit directo.
* **`ai/<ID>`**: ramas generadas por el flujo de tareas de Quorum, una por tarea (por ejemplo
  `ai/HEX-001`). Se corresponden con un artefacto de tarea en `.ai/tasks/` y no se renombran.
* **`feature/<descripcion-corta>`**: ramas de trabajo humano para una funcionalidad o corrección
  concreta, con nombre descriptivo en minúsculas y guiones (por ejemplo
  `feature/backup-cuatro-bases`).

## Mensajes de commit

Se usan **conventional commits**, siempre en **español**:

```
<tipo>(<alcance opcional>): <descripción breve en imperativo>

<cuerpo opcional con más contexto>
```

Tipos habituales: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `build`, `ci`.

Ejemplo:

```
docs: añadir ADR de licencia y actualizar el índice
```

**Prohibido en cualquier mensaje de commit:**

* Trailers de atribución a IA (por ejemplo `Co-Authored-By: <asistente>`).
* Cualquier mención de que el cambio fue generado o asistido por una herramienta de IA.
* Fechas relativas ("hoy", "ayer", "la semana pasada"); usar siempre fechas absolutas
  (`2026-07-29`), consistente con `CLAUDE.md`.

El autor humano responsable de la contribución es quien firma el commit con su propia identidad de
Git; no se añade ninguna coautoría automática.

## Qué nunca se versiona

Estos patrones están y deben seguir en `.gitignore`; nunca se añaden con `git add -f`:

* `*.db`, `*.db-wal`, `*.db-shm` — datos de inquilinos (bases SQLite por célula).
* `.env`, `.env.*` — secretos y variables de entorno.
* Cualquier credencial, token o clave privada, con o sin extensión reconocida por `.gitignore`.

Si un archivo de este tipo se añadió por error, no se corrige con un nuevo commit que lo borre: hay
que avisar antes de empujar el cambio, porque el contenido ya quedó en el historial local.

## Antes de abrir una propuesta de cambio

1. Si el cambio afecta a una decisión de arquitectura, revisa si ya existe un ADR relacionado en
   `docs/adr/README.md` y si la idea concreta ya se descartó en `docs/bitacora-de-descartes.md`.
2. Si el cambio introduce un requisito nuevo o modifica el alcance de una etapa, esa trazabilidad
   debe quedar escrita en `docs/PRD.md` o registrada como decisión pendiente en `docs/STATUS.md`; el
   plan no inventa requisitos.
3. Usa la plantilla de `.github/PULL_REQUEST_TEMPLATE.md` al abrir la propuesta.
