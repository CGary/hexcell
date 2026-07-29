//! Cliente y tipos de la Meta Graph API para las células sobre canal oficial.
//!
//! **Este crate está vacío a propósito y no declara ningún elemento visible desde fuera: ni
//! tipos, ni traits, ni funciones, ni módulos.** No es un olvido ni un esqueleto a medio hacer.
//!
//! El motivo es que su forma la condiciona una decisión todavía sin tomar: `adr-0013`, la
//! entrada de red del canal oficial —Cloudflare Tunnel en capa gratuita frente a VPS con
//! WireGuard—. De esa decisión depende si el TLS termina en el borde o en el propio Caddy, y con
//! ella la vigencia de FR-04 y de NFR-04. Diseñar aquí tipos de verificación de firma, de
//! webhook o de cliente HTTP antes de resolverla no adelantaría trabajo: **condicionaría la
//! decisión en lugar de esperarla**, porque la forma del código ya escrito pesa en la elección
//! que viene después.
//!
//! El crate se crea igualmente ahora, y no cuando haga falta, para que la frontera entre crates
//! quede fijada desde el primer día y el canal oficial tenga su sitio reservado en el workspace.
//! Los dos canales **conviven** en células distintas del mismo servidor: el canal propio con
//! whatsmeow es permanente y el oficial se suma cuando aparezca un cliente que lo justifique
//! (`adr-0014`).
//!
//! Ver `docs/adr/adr-0002-estructura-workspace.md` y `docs/plan/fase-a-1-fundaciones.md`.
