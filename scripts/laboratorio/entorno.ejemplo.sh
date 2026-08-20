#!/usr/bin/env sh
# Entorno compartido para el arnés de laboratorio de la célula (tarea 15 de A-3).
# AVISO: Este arnés opera procesos directos para pruebas de laboratorio.
# El empaquetado operable final (Docker + hexcell-admin) corresponde a la etapa A-6.

# Directorio raíz del laboratorio (configurable por el operador)
export HEXCELL_LAB_DIR="${HEXCELL_LAB_DIR:-/tmp/hexcell-laboratorio}"

# Identificador de la célula de laboratorio
export HEXCELL_ID_CELULA="${HEXCELL_ID_CELULA:-lab-01}"

# Ruta del socket IPC compartido entre núcleo y sidecar
export HEXCELL_SOCKET_IPC="${HEXCELL_SOCKET_IPC:-$HEXCELL_LAB_DIR/ipc/sidecar.sock}"

# Rutas de almacenamiento del núcleo y del sidecar (cuatro bases de persistencia)
export HEXCELL_RUTA_DATOS="${HEXCELL_RUTA_DATOS:-$HEXCELL_LAB_DIR/datos}"
export HEXCELL_RUTA_SQLSTORE="${HEXCELL_RUTA_SQLSTORE:-$HEXCELL_LAB_DIR/datos/sqlstore.db}"
export HEXCELL_RUTA_IDENTIDAD="${HEXCELL_RUTA_IDENTIDAD:-$HEXCELL_LAB_DIR/datos/identidad.db}"
export HEXCELL_RUTA_OUTBOX="${HEXCELL_RUTA_OUTBOX:-$HEXCELL_LAB_DIR/datos/outbox.db}"

# Canal de mensajería para el núcleo
export HEXCELL_CANAL="${HEXCELL_CANAL:-whatsmeow}"

# Zona horaria requerida para la ventana de atención de la célula (hallazgo 10)
export HEXCELL_VENTANA_ZONA="${HEXCELL_VENTANA_ZONA:-America/La_Paz}"

# Dirección del servidor interno de salud
export HEXCELL_DIRECCION_SALUD="${HEXCELL_DIRECCION_SALUD:-127.0.0.1:8081}"

# Teléfono de la célula para emparejamiento (opcional, leído del entorno del operador)
# Nunca fijar un literal aquí (adr-0010): debe provenir del entorno del operador.
export HEXCELL_TELEFONO_CELULA="${HEXCELL_TELEFONO_CELULA:-}"

# Ruta por omisión para almacenar los respaldos de laboratorio (consumida por el script, no por el binario)
export HEXCELL_RUTA_RESPALDOS="${HEXCELL_RUTA_RESPALDOS:-$HEXCELL_LAB_DIR/respaldos}"
