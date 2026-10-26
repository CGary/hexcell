# Runbook: despliegue en el OptiPlex

Encargo para el agente que trabaje en este repo: **levantar la primera célula real de HexCell en el servidor `optiplex`** y correr sobre ese hardware las mediciones que hasta ahora solo se hicieron en la máquina de desarrollo. Aquí va solo lo que no se deduce del repo (estado del servidor y cómo entrar). Cómo se materializa una célula: [`plantilla-celula.md`](plantilla-celula.md), [`runbook-operacion.md`](runbook-operacion.md), `deploy/cell.compose.yml`.

## El servidor (estado al 2026-10-24)

| Dato | Valor |
|------|-------|
| Hostname / usuario | `optiplex` / `gary` (sudo) |
| Acceso | `ssh gary@optiplex` por Tailscale (resuelve dentro de la tailnet). Solo clave SSH; la de `pc-gary` ya está autorizada. No escribir en el repo IPs ni datos de la red doméstica |
| SO | Debian 13 (trixie), sin escritorio, actualizaciones de seguridad automáticas |
| Hardware | i7-4770 (Haswell, 8 hilos), 9,7 GiB RAM, SSD 196 GB libres. Es exactamente el perfil objetivo del PRD (i7 de ~10 años, ~8 GB) |
| Docker | 29.x + Compose v5, **cgroup v2** (`cgroup2fs`, driver `systemd`). `gary` está en el grupo `docker` |
| Instalado además | `git`, `just`, `cron`, `tmux`, `fish`, `nvim`, `cloudflared` |
| Convive con | **Cauce** (otro proyecto): un contenedor en `127.0.0.1:8080` y el servicio del host `cloudflared-quick` que lo expone. No tocar ninguno de los dos |

Aprovisionado con scripts idempotentes del repo `Scriptorium` (en `pc-gary`): `~/Scriptorium/scripts/debian/server-bootstrap.sh`, `server-comfort.sh`, `cloudflared-quick.sh`.

## Lo que hay que hacer

1. Código: `git clone https://github.com/CGary/hexcell.git ~/hexcell` en el servidor. Sin toolchain de Rust ni Go en el host: todo se construye dentro de Docker.
2. Imágenes: `docker build -t hexcell-nucleo:local .` y `docker build -t hexcell-sidecar:local ./sidecar` (mismos contextos que la CI). La primera compilación de Rust en un Haswell tardará bastante; usar `tmux`.
3. Guardas en el hardware real, en este orden: `bash deploy/verificar_imagenes.sh`, `deploy/verificar_endurecimiento.sh`, `deploy/verificar_aislamiento.sh` (dos células reales), `deploy/verificar_apagado_ordenado.sh`.
4. Medición que el PRD tiene pendiente (NFR-01): `deploy/medir_memoria_y_imagenes.sh` en reposo y bajo el generador declarado; registrar los valores en `plantilla-celula.md` como referencia del servidor real, y revisar los límites provisionales de `deploy/celula.env.ejemplo`.
5. Primera célula: renderizar su `.env` con `hexcell-admin config render` a partir de `deploy/celula.defecto.env.ejemplo` + overlay, levantarla con `deploy/cell.compose.yml` y emparejar el canal propio según `runbook-canal-whatsmeow.md`.

## Restricciones

- **Secretos** (claves de LLM, tokens de Meta, sesiones de WhatsApp) se crean en el servidor y no se pegan en el chat ni en git. Para `sudo` sin TTY, la contraseña se canaliza desde `pc-gary` con `sops -d --extract '["passwords"]["gary_optiplex"]' ~/fleet/secrets/main.yaml | ssh gary@optiplex sudo -S -p '' <cmd>`; nunca se muestra.
- Las células **no publican puertos al host** (lo verifica `verificar_aislamiento_estatica.sh`); el 8080 es de Cauce.
- Presupuesto compartido: Cauce usa poco, pero la RAM total son 9,7 GiB. Dejar margen al medir el techo de células.
- Entrada pública para el canal oficial (Cloudflare Tunnel vs VPS + WireGuard, README § handshake sintético) sigue **pendiente de ADR**: no decidirla desde este encargo. Hoy no hay dominio; el Quick Tunnel del host es de Cauce y no sirve para webhooks de Meta (URL cambia en cada reinicio).
- Registro de imágenes (GHCR u otro) también pendiente (`STATUS.md`); construir local, no publicar.
- Cloudflare: existe cuenta (2026-10-25); credenciales en `sops` de `pc-gary` (`cloudflare.api_token`, `cloudflare.account_id` en `~/fleet/secrets/main.yaml`), usadas solo desde `pc-gary` vía `wrangler`/API, nunca en el servidor ni en el repo. El túnel con nombre del servidor será uno solo, como servicio del host, compartido con Cauce; HexCell solo añadiría rutas si una decisión (ADR) lo pide.

## Pendiente externo (no bloquea)

- Dominio propio para el canal oficial y el túnel con nombre.
- `vault_gary_optiplex_become_password` en el vault de `Scriptorium` si Ansible va a gestionar el host.
