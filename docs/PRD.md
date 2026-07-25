# Documento de Requisitos del Producto (PRD)
## Proyecto: Orquestador Multi-Inquilino ZeroClaw (v1.0.0)

### 1. Control de Versiones y Estado
* **Estado:** Aprobado para Desarrollo.
* **Rol de Autoría:** Consultor de Producto Senior & Arquitecto de Soluciones.
* **Pila Tecnológica Núcleo:** Rust (Backend Nativo), Docker (Aislamiento), Caddy (Proxy Inverso), SQLite (Persistencia Dual).

---

### 2. Descripción General y Objetivos Comerciales
ZeroClaw es una plataforma de software multi-inquilino (*multi-tenant*) de alta eficiencia diseñada para ejecutarse en entornos de hardware locales restringidos (servidor Intel i7 de hace 10 años, 8 GB de memoria RAM, almacenamiento SSD). El producto permite empaquetar, desplegar y operar de forma masiva bots automatizados para WhatsApp dirigidos a microempresas locales, cubriendo los casos de uso de atención al cliente, respuestas a preguntas frecuentes, catálogo/venta de productos y agendamiento de servicios.

El objetivo central es minimizar el costo operativo por inquilino mediante una ejecución nativa sin sobrecarga de memoria.

---

### 3. Requisitos

#### A. Requisitos Funcionales (FR)
* **FR-01:** ⚠️ **TODO — contenido perdido en el documento original** (el texto fuente llegó truncado a mitad de frase; por el contexto, FR-01 cubría la recepción/verificación de webhooks de la Meta Graph API. Reconstruir y validar).
* **FR-02: Aislamiento Completo por Inquilino:** Cada microempresa debe operar dentro de un contenedor Docker dedicado e independiente basado en imágenes mínimas (Alpine/Scratch), con un consumo objetivo menor a 50 MB de RAM en reposo.
* **FR-03: Gestión de Configuración Dinámica (Caddy):** El sistema debe registrar subdominios únicos por cliente (`clienteX.midominio.com`) de manera programática en la API de administración de Caddy sin interrumpir el tráfico de terceros.
* **FR-04: Handshake Sintético de Red:** Antes de registrar cualquier URL en Meta, el orquestador local debe validar la validez del certificado TLS y el enrutamiento público inyectando el SNI y resolviendo el socket directamente a la interfaz local (`127.0.0.1:443`) para eludir restricciones de Hairpin NAT.
* **FR-05: Arquitectura de Persistencia Dual (Dual-DB):** Cada contenedor debe desacoplar el estado transaccional del conocimiento de negocio mediante dos bases de datos SQLite físicas independientes: `sessions.db` (Lectura/Escritura continua) y `knowledge_live.db` (Lectura intensiva de RAG).
* **FR-06: Indexación en Sombra (Shadow DB):** Las actualizaciones de catálogo o embeddings de IA no deben bloquear la producción. Deben compilarse asíncronamente en un archivo `knowledge_staging.db` mediante llamadas por lotes a APIs externas.
* **FR-07: Conmutación Atómica por Épocas:** La promoción de nuevos conocimientos en el bot debe ocurrir en microsegundos usando renombrado de archivos por épocas (`knowledge_epoch_N.db`), manipulación de enlaces simbólicos y reemplazo atómico de punteros en memoria (`ArcSwap`), seguido de un drenaje asíncrono controlado (`Graceful Drain`) del pool antiguo para evitar corrupciones en el modo WAL de SQLite.
* **FR-08: Control de Admisión Anti-Spam (GCRA):** Middleware HTTP integrado basado en el algoritmo *Generic Cell Rate Algorithm* (GCRA) sin cerrojos de memoria para rechazar ráfagas de tráfico respondiendo un HTTP 200 OK rápido a Meta, evitando el consumo de heap y penalizaciones por reintentos de la API Graph.
* **FR-09: Semáforo de Concurrencia de CPU:** Límite estricto de tareas Tokio en vuelo simultáneas por contenedor para mitigar la degradación por cambio de contexto en el procesador.
* **FR-10: Contabilidad Financiera de Dos Fases:** Control atómico previo a la llamada del LLM (*Pre-Execution Hold*) basado en la longitud estimada del prompt y conciliación posterior (*Post-Execution Reconcile*) según los tokens reales devueltos por la API (Gemini/Groq), conmutando a un modo degradado de reglas fijas locales al agotarse el saldo.
* **FR-11: Operaciones CLI de Tráfico Amortiguado (Traffic Shedding):** Herramienta de línea de comandos capaz de suspender clientes aplicando *blackholing* en Caddy (HTTP 200 inmediato estático) antes de emitir un SIGTERM de Docker, asegurando que no se generen respuestas HTTP 502 hacia Meta.

#### B. Requisitos No Funcionales (NFR)
| ID | Categoría | Requisito Técnico |
| :--- | :--- | :--- |
| **NFR-01** | Eficiencia | Consumo máximo por instancia de cliente en reposo: 50 MB de RAM. |
| **NFR-02** | Disponibilidad | Tasa nula (0%) de errores HTTP 502/503 expuestos hacia la WAN de Meta durante suspensiones o reactivaciones. |
| **NFR-03** | Latencia | Conmutación interna de base de datos de conocimiento inferior a 10 milisegundos. |
| **NFR-04** | Seguridad | Cifrado forzoso HTTPS TLS v1.2/v1.3 gestionado automáticamente vía Caddy (On-Demand TLS). |
| **NFR-05** | Seguridad | Aislamiento estricto de almacenamiento: Un contenedor no puede mapear ni acceder al volumen de datos de otro inquilino. |

---

### 4. Arquitectura y Ciclo de Vida de los Datos

#### Patrón Shadow DB e Inmutabilidad de Épocas

```
[Flujo de Actualización de Conocimiento]
Panel Admin -> Payload JSON -> Contenedor Rust
|
(Crea) knowledge_staging.db
| -> Ingesta de Embeddings (API externa)
(Sella) PRAGMA wal_checkpoint(TRUNCATE);
|
(Renombra) knowledge_epoch_2.db
| -> Cambia enlace simbólico atómico
(Memoria) ArcSwap::store(Nuevo Pool)
|
[Mensajes de WhatsApp consumen Epoch 2]
|
(Drena) old_pool.close().await
| -> Libera FDs de Epoch 1 sin corrupción WAL
```

---

### 5. Matrices de Ciclo de Vida de Administración

#### Secuencia de Suspensión (CLI Central)
1. **PATCH Caddy Admin API:** Sustituir la ruta de `reverse_proxy` por un `static_response_handler` que devuelva HTTP 200 OK con `{}` a Meta de forma inmediata.
2. **SIGTERM Docker Container:** Detener el contenedor del cliente con un tiempo de gracia de 30 segundos (`t=30`). El binario en Rust intercepta la señal, detiene la aceptación de sockets, drena las peticiones RAG activas, ejecuta un checkpoint de SQLite y finaliza limpiamente (`Exit 0`).
3. **Liberación de Memoria:** El kernel remueve el proceso de la memoria RAM del servidor local.

#### Secuencia de Reactivación (CLI Central)
1. **POST Docker API:** Iniciar el contenedor del inquilino. Caddy mantiene el comportamiento estático activo absorbiendo webhooks en paralelo.
2. **Readiness Polling local:** La CLI interroga al endpoint interno `http://{IP_DOCKER}/health/ready` cada 100ms. El contenedor solo responde 200 OK tras comprobar que las conexiones SQLite (`sessions.db` y `knowledge_live.db`) están activas y las estructuras atómicas GCRA cargadas.
3. **PATCH Caddy Admin API:** Conmutar de la respuesta estática al `reverse_proxy` solo tras la primera confirmación positiva de salud.

---

### 6. Criterios de Aceptación para QA
* **Prueba de Carga de Red:** Sometimiento de un contenedor a 100 peticiones concurrentes simulando la API de Meta. El sistema debe activar el middleware GCRA, devolver HTTP 200 rápido para el exceso de tasa y el uso de memoria RAM no debe incrementarse en más del 15% del consumo base.
* **Prueba de Resiliencia del Enlace TLS:** Bloquear artificialmente el Hairpin NAT del router local. El script de orquestación debe completar el onboarding con éxito mediante la bandera `--resolve` forzada a nivel de socket.
* **Prueba de Consistencia en Modo WAL:** Ejecutar un intercambio de conocimiento mientras se procesan 20 lecturas RAG simultáneas. El sistema no debe arrojar excepciones de tipo `SQLITE_BUSY` ni dejar huérfanos archivos `.db-wal` o `.db-shm`.
