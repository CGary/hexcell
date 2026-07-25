# ZeroClaw Orchestrator

ZeroClaw es un motor orquestador multi-inquilino de ultra alta eficiencia escrito en **Rust**, diseñado para desplegar y administrar bots automatizados de WhatsApp para microempresas locales. La arquitectura está optimizada estructuralmente para ejecutarse en servidores locales con severas restricciones de hardware (procesadores heredados de consumo doméstico y baja densidad de memoria RAM) sin comprometer la estabilidad, el aislamiento de datos ni el presupuesto financiero de las APIs de lenguaje natural.

> **Estado del proyecto:** fase de diseño. Ver [docs/PRD.md](docs/PRD.md) (requisitos) y [docs/STATUS.md](docs/STATUS.md) (avance).

---

## 🛡️ Pilares de la Arquitectura de Software

### 1. Inferencia Externa y Hardware Local Protegido
El hardware local no procesa modelos de lenguaje grande (LLMs). Toda la inferencia semántica y generativa se delega mediante conexiones HTTPS salientes hacia infraestructuras externas de bajo costo (Gemini 1.5 Flash, Groq u OpenRouter). El motor nativo en Rust limita su consumo a la lógica de control, enrutamiento, consumo de API y consultas vectoriales locales, logrando un consumo objetivo de **< 50 MB de RAM por cliente**.

### 2. Persistencia Segregada en SQLite Dual y Aislamiento WAL
Para evitar la contención de escrituras concurrentes y el bloqueo de transacciones (`SQLITE_BUSY`) al interactuar con servicios de red de alta latencia, cada inquilino corre en un contenedor Docker aislado equipado con dos bases de datos físicas independientes:
* `sessions.db`: Almacena el historial y el estado conversacional. Modo lectura/escritura continua en caliente.
* `knowledge_live.db`: Contiene las reglas de negocio, catálogos y embeddings vectoriales para el motor de Recuperación Aumentada por Generación (RAG). Se opera en modo estrictamente de lectura durante producción.

### 3. Pipeline de Actualización Inmutable y Cambio Atómico (Shadow DB)
Las actualizaciones de conocimiento se gestionan en una base de datos en sombra (`knowledge_staging.db`) aislando las llamadas por lotes a APIs de embeddings. Una vez validada la integridad estructural y semántica del índice, se ejecuta la secuencia atómica por épocas:
1. Sellar y colapsar el WAL de staging vía `PRAGMA wal_checkpoint(TRUNCATE);`.
2. Renombrar el archivo a una época inmutable (`knowledge_epoch_N.db`).
3. Reasignar de forma atómica el enlace simbólico del sistema de archivos y actualizar el pool de conexiones en memoria empleando `ArcSwap`.
4. Ejecutar un drenaje controlado asíncrono (`Graceful Drain`) de las conexiones del pool obsoleto, erradicando corrupciones o bloqueos de descriptores de archivos (`-wal` y `-shm`).

### 4. Defensa Perimetral contra Tormentas de Reintentos de Meta (GCRA)
El middleware HTTP del contenedor integra el algoritmo **GCRA (Generic Cell Rate Algorithm)** acoplado de forma nativa a la capa de red. Ante ráfagas de tráfico o ataques de spam:
* Intercepta las peticiones que exceden el límite de tasa antes de alocar memoria en el heap.
* Responde con un código **HTTP 200 OK sintético e inmediato** a Meta (patrón *Fast-Reject*), anulando las tormentas de reintentos automáticos generadas por la API Graph cuando recibe códigos de error estándar (429/503).
* Garantiza el control presupuestario mediante un sistema de contabilidad de cuotas financieras en dos fases: Reserva Previa (*Pre-Execution Hold*) antes de invocar al LLM y Conciliación Exacta (*Post-Execution Reconcile*) posterior a la recepción de los metadatos de tokens.

---

## 🛠️ Flujo de Onboarding e Inyección de Red (Anti-Hairpin NAT)

El proceso de alta de una nueva microempresa utiliza el flujo **Meta Embedded Signup** bajo una única aplicación del proveedor para una experiencia de usuario sin fricción técnica. El aislamiento de red se logra mediante la propiedad `override_callback_uri` de la API Graph, enviando el tráfico de cada WABA directamente al subdominio del inquilino (`https://clienteX.midominio.com/webhook`).

Para asegurar el apretón de manos síncrono inicial frente a Meta, el script de orquestación mitiga la ausencia de Hairpin NAT en enrutadores locales forzando la resolución del socket del cliente HTTP hacia la interfaz de loopback local, enviando explícitamente el SNI y el encabezado Host del dominio público:

```bash
# Handshake sintético ejecutado por el orquestador local para forzar el desafío ACME en Caddy
curl --resolve cliente1.midominio.com:443:127.0.0.1 \
  -v "https://cliente1.midominio.com/webhook?hub.mode=subscribe&hub.verify_token=CRYPTO_TOKEN&hub.challenge=handshake_test"
```

Este método garantiza de manera matemática que la Autoridad Certificadora (Let's Encrypt/ZeroSSL) validó externamente el entorno WAN del servidor local antes de autorizar la suscripción definitiva en la API Graph de Meta.

---

## 💻 Manual de Operación de la CLI de Administración

La suite de administración central compila como un binario nativo que interactúa directamente con el socket Unix de Docker (`/var/run/docker.sock`) y la API local de administración en memoria de Caddy (`http://localhost:2019`).

### 1. Suspender Temporalmente un Inquilino (Falta de pago / Pausa)

Garantiza la liberación inmediata de RAM y CPU en el hardware local sin inyectar códigos de error de enrutamiento (502 Bad Gateway) hacia la red de Meta.

```bash
./zeroclaw-admin tenant pause --id <tenant_id> --domain cliente1.midominio.com
```

*Mecanismo Interno:* Aplica un parche en Caddy para sustituir el `reverse_proxy` por un `static_response_handler` (HTTP 200 instantáneo). Acto seguido, envía una señal `SIGTERM` al contenedor Docker con un margen de 30 segundos para drenar lecturas RAG en vuelo y flush del WAL a disco.

### 2. Reactivar un Inquilino

Restaura la producción asegurando que el backend está completamente listo antes de desviar tráfico real de mensajería.

```bash
./zeroclaw-admin tenant unpause --id <tenant_id> --domain cliente1.midominio.com
```

*Mecanismo Interno:* Inicia el contenedor Docker de forma aislada. La CLI ejecuta un bucle de *Readiness Polling* local hacia el endpoint `GET /health/ready` del contenedor cada 100ms. Caddy conmuta el tráfico de la respuesta estática al proxy inverso únicamente cuando el binario en Rust responde con código 200 OK tras comprobar de extremo a extremo la vitalidad de sus pools de persistencia SQLite.

### 3. Eliminar Definitivamente un Inquilino

Remoción destructiva limpia y desvinculación perimetral.

```bash
./zeroclaw-admin tenant terminate --id <tenant_id> --domain cliente1.midominio.com --waba <waba_id>
```

*Mecanismo Interno:* Invoca la desasociación del webhook en la API Graph de Meta, ejecuta el drenaje por `SIGTERM` del contenedor, destruye los volúmenes de disco locales de manera física (`std::fs::remove_dir_all`) y purga de forma atómica la regla de enrutamiento y la memoria caché de certificados en el servidor web Caddy.
