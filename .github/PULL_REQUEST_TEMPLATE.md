## Resumen

<!-- Qué cambia y por qué, en dos o tres frases. -->

## Tipo de cambio

- [ ] Documentación (`docs:`)
- [ ] Funcionalidad nueva (`feat:`)
- [ ] Corrección (`fix:`)
- [ ] Refactor (`refactor:`)
- [ ] Otro (especificar):

## Trazabilidad

<!-- Enlaza el FR/NFR del PRD, la etapa del plan (docs/plan/) o el ADR que corresponda.
     Si el cambio no se traza a ninguno, explica por qué y si hace falta registrarlo como
     decisión pendiente en docs/STATUS.md. -->

- Requisito/etapa/ADR relacionado:

## Checklist

- [ ] El mensaje de commit sigue el formato de conventional commits en español (`CONTRIBUTING.md`).
- [ ] Ningún commit incluye trailers de atribución a IA (por ejemplo `Co-Authored-By`).
- [ ] No se versiona ningún `*.db`, `*.db-wal`, `*.db-shm`, `.env*` ni secreto.
- [ ] Si el cambio toca una decisión de arquitectura, se revisó `docs/adr/README.md` y
      `docs/bitacora-de-descartes.md` antes de proponerlo.
- [ ] Si el cambio introduce un requisito nuevo o cambia el alcance de una etapa, queda trazado en
      `docs/PRD.md` o registrado como decisión pendiente en `docs/STATUS.md`.
- [ ] Las fechas mencionadas en el cambio son absolutas, nunca relativas.

## Cómo se validó

<!-- Describe cómo se comprobó el cambio: lectura cruzada de artefactos, quorum validate,
     verify.commands del contrato, etc. -->
