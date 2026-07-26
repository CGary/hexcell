# Etapa 5 — Empaquetado y aislamiento por contenedor

**Duración relativa:** Media.

---

## Objetivo

Hasta aquí existe un binario que funciona en la máquina del desarrollador. Esta etapa lo convierte en
la unidad de despliegue real del producto: una imagen de contenedor mínima, con límites de recursos
explícitos y un aislamiento de almacenamiento que se pueda demostrar, no solo afirmar.

Hay dos requisitos del PRD que solo se pueden verificar de verdad en este punto. El primero es
NFR-01, el techo de 50 MB de RAM por instancia: una medición en el escritorio del desarrollador no
significa nada, porque el objetivo de negocio es alojar decenas de inquilinos en un servidor con 8 GB
de memoria. El segundo es NFR-05, el aislamiento estricto de almacenamiento, que exige que un
contenedor **no pueda** acceder al volumen de datos de otro. Nótese la diferencia entre "no accede" y
"no puede acceder": la primera es una convención y la segunda es una propiedad del sistema. El
producto vende privacidad a microempresas que comparten hardware, así que solo la segunda es
aceptable, y demostrarla requiere un intento explícito de violarla que debe fallar.

La etapa se coloca después del conocimiento porque la disposición del directorio de datos —las dos
bases activas, la de staging, las épocas históricas y sus enlaces simbólicos— solo queda fijada al
terminar la etapa 4. Empaquetar antes obligaría a rehacer el `Dockerfile` y el diseño de volúmenes en
cada iteración.

---

## Alcance

### Qué entra

* `Dockerfile` multi-etapa que compila el binario del inquilino y lo entrega sobre una imagen base
  mínima (Alpine o Scratch), sin cadena de herramientas ni dependencias innecesarias.
* Compilación con enlazado adecuado a la imagen base elegida y perfil de *release* orientado a
  tamaño.
* Ejecución del proceso como usuario sin privilegios, con sistema de archivos raíz de solo lectura
  salvo el volumen de datos, y sin capacidades de kernel superfluas.
* Diseño definitivo del volumen de datos por inquilino: un volumen dedicado, montado en una única
  ruta, con permisos que impiden el acceso cruzado entre contenedores.
* Límites de recursos por contenedor: memoria, CPU y número de descriptores de archivo.
* Plantilla de composición o especificación de arranque parametrizada por inquilino, con las
  variables de entorno, el volumen y los límites ya resueltos.
* Comprobación de salud del contenedor apoyada en `GET /health/ready`.
* Manejo correcto de señales dentro del contenedor, para que el `SIGTERM` de Docker llegue al
  proceso Rust y active el apagado ordenado de la etapa 2.
* Medición formal del consumo de memoria residente en reposo y bajo carga.
* Publicación de la imagen desde la CI, versionada de forma reproducible.

### Qué NO entra

* La gestión del ciclo de vida de los contenedores desde la CLI: etapa 6.
* Caddy, certificados y enrutamiento: etapa 6.
* Orquestadores de clúster. El PRD fija un servidor local único; introducir Kubernetes o similares
  contradice el objetivo de eficiencia.

### Requisitos del PRD cubiertos

* **FR-02** — aislamiento completo por inquilino en contenedores dedicados sobre imágenes mínimas.
* **NFR-01** — consumo máximo de 50 MB de RAM por instancia en reposo, verificado por medición.
* **NFR-05** — aislamiento estricto de almacenamiento entre inquilinos, verificado por intento de
  violación.

---

## Entregables

* `Dockerfile` multi-etapa del inquilino y `.dockerignore`.
* `deploy/tenant.compose.yml` (o especificación equivalente) parametrizada por inquilino.
* `docs/adr/adr-0007-imagen-y-aislamiento.md` documentando la imagen base elegida, el modelo
  de permisos del volumen y los límites de recursos.
* Script de medición de memoria y de tamaño de imagen, ejecutable de forma repetible.
* Prueba automatizada de aislamiento: un contenedor intenta leer el volumen de otro y falla.
* Trabajo de CI que construye y publica la imagen etiquetada.

---

## Tareas

1. **Escribir el `Dockerfile` multi-etapa** (1 día). Etapa de compilación con la cadena de
   herramientas y etapa final mínima con solo el binario y sus datos.
2. **Resolver el enlazado y minimizar el binario** (1 día). Ajustar el objetivo de compilación a la
   imagen base, activar las optimizaciones de tamaño y eliminar símbolos innecesarios.
3. **Endurecer el contenedor** (1 día). Usuario sin privilegios, raíz de solo lectura, eliminación de
   capacidades no necesarias y ausencia de shell si la imagen base lo permite.
4. **Diseñar y aplicar el modelo de volumen por inquilino** (1 día). Ruta única de datos, propiedad y
   permisos del volumen, y verificación de que la disposición de épocas y enlaces simbólicos de la
   etapa 4 funciona igual dentro del contenedor.
5. **Fijar los límites de recursos** (0,5 días). Memoria, CPU y descriptores de archivo, con valores
   coherentes con NFR-01 y con la densidad de inquilinos objetivo.
6. **Verificar la propagación de señales** (0,5 días). Comprobar que `docker stop` con margen de 30
   segundos produce el apagado ordenado y una salida con código 0, sin necesidad de matar el proceso.
7. **Parametrizar la plantilla de arranque por inquilino** (1 día). Todo lo que distingue a un
   inquilino de otro pasa a ser configuración: identificador, volumen, secretos, límites y puerto
   interno.
8. **Medir memoria y tamaño de imagen** (0,5 días). Consumo en reposo, consumo bajo carga y peso de
   la imagen final, registrados como valores de referencia.
9. **Escribir la prueba de aislamiento** (1 día). Levantar dos inquilinos y demostrar que ninguno
   puede leer ni escribir el volumen del otro, ni siquiera conociendo la ruta.
10. **Integrar la construcción de la imagen en la CI** (1 día). Construcción reproducible, etiquetado
    por versión y por commit, y publicación en el registro elegido.

---

## Criterios de aceptación

* El contenedor de un inquilino arranca, responde `GET /health/ready` con `200 OK` y procesa un
  webhook firmado de extremo a extremo.
* El consumo de memoria residente en reposo es inferior a 50 MB, medido con el contenedor en
  funcionamiento y con ambas bases abiertas (NFR-01).
* `docker stop` con margen de 30 segundos produce salida con código 0 y checkpoint del WAL
  completado, sin recurrir a `SIGKILL`.
* Un contenedor no puede listar, leer ni escribir el volumen de datos de otro inquilino; el intento
  falla por permisos y queda registrado (NFR-05).
* El proceso dentro del contenedor no se ejecuta como `root` y el sistema de archivos raíz es de solo
  lectura salvo la ruta de datos.
* La imagen se construye de forma reproducible desde la CI y su tamaño queda registrado.
* Con varios contenedores simultáneos, el consumo agregado es compatible con la capacidad del
  servidor objetivo de 8 GB.

---

## Riesgos y mitigaciones

| Riesgo | Impacto | Mitigación |
| :--- | :--- | :--- |
| Problemas de enlazado con la biblioteca C de la imagen base mínima. | Medio: retrasos de integración y binarios que no arrancan. | Decidir imagen base y objetivo de compilación al principio de la etapa y validarlos con un binario mínimo antes de empaquetar el real. |
| El consumo real supera los 50 MB por el tamaño de los pools y de las estructuras de RAG. | Alto: incumplimiento directo de NFR-01 y del modelo de densidad del negocio. | Medir pronto; si se supera, ajustar tamaño de pools, caché de vectores y límites de concurrencia antes de continuar. |
| Permisos de volumen mal configurados que dejan datos accesibles entre inquilinos. | Muy alto: fallo de privacidad frente al cliente final. | Prueba automatizada de aislamiento como criterio bloqueante de la etapa. |
| El proceso no recibe `SIGTERM` por quedar bajo un intérprete de shell dentro del contenedor. | Alto: apagados abruptos y riesgo de corrupción del WAL. | Ejecutar el binario como proceso principal directo y verificar la señal en la tarea 6. |
| El diseño de enlaces simbólicos de épocas se comporta distinto sobre el volumen montado. | Medio: la conmutación atómica falla solo en producción. | Repetir la prueba de estrés de la etapa 4 dentro del contenedor antes de cerrar esta etapa. |

---

## Dependencias

* **De otras etapas:** etapas 2, 3 y 4 completas. En particular, la disposición definitiva del
  directorio de datos que fija la etapa 4 y la línea base de memoria de la etapa 2.
* **Externas:** un registro de imágenes donde publicar, y acceso a un entorno con Docker equivalente
  al servidor de destino para las mediciones.
* **Decisiones de producto pendientes:** ninguna bloquea esta etapa.
