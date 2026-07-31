package ipc_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// rutaDelDocumento es la ruta del documento del protocolo relativa a este paquete
// (sidecar/internal/ipc → raíz del repositorio).
const rutaDelDocumento = "../../../docs/protocolo-ipc-nucleo-sidecar.md"

// leerDocumento devuelve el texto del documento del protocolo.
//
// Estos tests existen a propósito: un criterio de aceptación comprobado solo leyendo el documento
// se degrada en cuanto alguien reorganiza el archivo. Comprobarlo con una prueba lo convierte en
// algo que la CI puede romper si una de las cuatro secciones obligatorias desaparece.
func leerDocumento(t *testing.T) string {
	t.Helper()
	contenido, err := os.ReadFile(filepath.Clean(rutaDelDocumento))
	if err != nil {
		t.Fatalf("no se pudo leer %s: %v", rutaDelDocumento, err)
	}
	return string(contenido)
}

func TestElDocumentoDelProtocoloCubreLosCuatroAspectosObligatorios(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	secciones := []string{
		"## 1. Formato de mensaje",
		"## 2. Transporte: socket de dominio Unix sobre el volumen compartido",
		"## 4. Semántica de confirmación de entrega",
		"## 5. Reconexión de cualquiera de los dos extremos",
	}
	for _, seccion := range secciones {
		if !strings.Contains(documento, seccion) {
			t.Errorf("el documento del protocolo no declara la sección %q", seccion)
		}
	}
}

func TestElDocumentoDelProtocoloDeclaraElSocketDeDominioUnixComoTransporte(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	for _, exigido := range []string{"socket de dominio Unix", "AF_UNIX", "SOCK_STREAM"} {
		if !strings.Contains(documento, exigido) {
			t.Errorf("el documento del protocolo no menciona %q", exigido)
		}
	}
	if strings.Contains(documento, "TCP sobre `localhost`") && !strings.Contains(documento, "Y no TCP sobre `localhost`") {
		t.Errorf("el documento menciona TCP sin declararlo descartado")
	}
}

func TestElDocumentoDelProtocoloEstaVersionadoYFechadoEnAbsoluto(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	if !strings.Contains(documento, "**Versión de este protocolo:** 1.0, fijada el 2026-07-31.") {
		t.Errorf("el documento no lleva cabecera de versión con fecha absoluta")
	}
	if !strings.Contains(documento, "docs/contrato-ipc-respaldo-del-sqlstore.md") {
		t.Errorf("el documento no referencia el contrato de respaldo de la etapa A-2")
	}
}
