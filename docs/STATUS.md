# Estado del Proyecto

> Registro vivo del avance. Última actualización: 2026-07-24.

## Fase actual
Diseño / planificación. No existe todavía código fuente ni scaffold Rust.

## Definido
* La pila tecnológica: Rust (backend nativo), Docker (aislamiento por inquilino), Caddy (proxy inverso + SSL), SQLite dual (persistencia).
* El modelo de despliegue por contenedores aislados (imágenes Alpine/Scratch, < 50 MB RAM por cliente).
* La viabilidad técnica del hardware (Intel i7 de 10 años, 8 GB RAM, SSD).
* Requisitos funcionales y no funcionales: ver [PRD.md](PRD.md).

## Pendiente
* Lógica de negocio específica.
* Flujos de usuario finales.
* Manejo de excepciones comerciales.
* Modelo de monetización.
* Proceso exacto de alta (onboarding) de una nueva microempresa en la infraestructura local.
* Reconstruir **FR-01** del PRD (perdido por truncado en el documento original).
* Decidir licencia (`LICENSE`) e inicializar repositorio git.
* Scaffold del workspace Rust (`Cargo.toml`, `src/`) cuando arranque la fase de implementación.
