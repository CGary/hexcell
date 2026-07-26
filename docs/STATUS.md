# Estado del Proyecto

> Registro vivo del avance. Última actualización: 2026-07-24.

## Fase actual
Diseño / planificación. No existe todavía código fuente ni scaffold Rust.

## Definido
* La pila tecnológica: Rust (backend nativo), Docker (aislamiento por inquilino), Caddy (proxy inverso + SSL), SQLite dual (persistencia).
* El modelo de despliegue por contenedores aislados (imágenes Alpine/Scratch, < 50 MB RAM por cliente).
* La viabilidad técnica del hardware (Intel i7 de 10 años, 8 GB RAM, SSD).
* Requisitos funcionales y no funcionales: ver [PRD.md](PRD.md).
* **Plan de implementación por etapas (8 etapas): ver [plan/README.md](plan/README.md).** Cubre
  FR-01..FR-11 y NFR-01..NFR-05, y sitúa los pendientes de producto de más abajo como bloqueos
  declarados en las etapas que los necesitan.

## Pendiente
* Lógica de negocio específica. — *Bloquea el alcance funcional de la etapa 2.*
* Flujos de usuario finales. — *Bloquean la etapa 7 y la superficie de carga de catálogo de la etapa 4.*
* Manejo de excepciones comerciales. — *Condiciona el modo degradado (etapa 3) y las alertas (etapa 8).*
* Modelo de monetización. — *Bloquea la calibración de saldos (etapa 3) y la suspensión por impago (etapa 6).*
* Proceso exacto de alta (onboarding) de una nueva microempresa en la infraestructura local. — *Bloquea la etapa 7.*
* Reconstruir **FR-01** del PRD (perdido por truncado en el documento original). — *Etapa 1, tarea 1.*
* Decidir licencia (`LICENSE`). — *Etapa 1, tarea 3. El repositorio git ya está inicializado.*
* Scaffold del workspace Rust (`Cargo.toml`, `src/`) cuando arranque la fase de implementación. — *Etapa 1, tarea 5.*
