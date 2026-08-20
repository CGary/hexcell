package ipc_test

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
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
	if !strings.Contains(documento, "**Versión de este protocolo:** 1.4, fijada el 2026-08-20.") {
		t.Errorf("el documento no lleva cabecera de versión con fecha absoluta")
	}
	if !strings.Contains(documento, "docs/contrato-ipc-respaldo-del-sqlstore.md") {
		t.Errorf("el documento no referencia el contrato de respaldo de la etapa A-2")
	}
}

func TestElDocumentoDeclaraLosNueveTiposQueElCodigoRegistra(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	for _, tipo := range ipc.TiposDeclarados() {
		if !strings.Contains(documento, "| `"+string(tipo)+"` |") {
			t.Errorf("el documento no declara el tipo %q en la tabla de la sección 6", tipo)
		}
	}
}

func TestLosCamposDelDocumentoCoincidentConElCodigo(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	for _, tipo := range ipc.TiposDeclarados() {
		campos := ipc.CamposDe(tipo)
		for _, campo := range campos {
			if campo == "version" || campo == "tipo" {
				continue
			}
			if !strings.Contains(documento, "| `"+campo+"` |") {
				t.Errorf("el tipo %q declara el campo %q en el código pero no en el documento", tipo, campo)
			}
		}
	}
}

func TestElDocumentoDeclaraLaCorrespondenciaDeVersiones(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	if !strings.Contains(documento, "| 1.4 | `5` |") {
		t.Errorf("el documento no declara la correspondencia 1.4 → cable 5")
	}
	if !strings.Contains(documento, "| 1.3 | `4` |") {
		t.Errorf("el documento no declara la correspondencia 1.3 → cable 4")
	}
	if !strings.Contains(documento, "| 1.2 | `3` |") {
		t.Errorf("el documento no declara la correspondencia 1.2 → cable 3")
	}
	if !strings.Contains(documento, "| 1.1 | `2` |") {
		t.Errorf("el documento no declara la correspondencia 1.1 → cable 2")
	}
	if !strings.Contains(documento, "| 1.0 | `1` |") {
		t.Errorf("el documento no declara la correspondencia 1.0 → cable 1")
	}
}

func TestElDocumentoDeclaraLasCausasDelCodigo(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	for _, causa := range ipc.CausasDeclaradas() {
		if !strings.Contains(documento, "| `"+causa+"` |") {
			t.Errorf("el documento no declara la causa %q", causa)
		}
	}
}

func TestElDocumentoNoDeclaraCausasQueElCodigoNoConoce(t *testing.T) {
	t.Parallel()

	declaradasEnDocumento := causasDelDocumento(t, leerDocumento(t))
	declaradasEnCodigo := append([]string(nil), ipc.CausasDeclaradas()...)
	sort.Strings(declaradasEnDocumento)
	sort.Strings(declaradasEnCodigo)
	if strings.Join(declaradasEnDocumento, ",") != strings.Join(declaradasEnCodigo, ",") {
		t.Fatalf("causas del documento = %v, causas del código = %v", declaradasEnDocumento, declaradasEnCodigo)
	}
}

func TestElDocumentoDeclaraLosCuatroEstadosDelCodigo(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	for _, estado := range ipc.EstadosDeclarados() {
		if !strings.Contains(documento, "`"+estado+"`") {
			t.Errorf("el documento no declara el estado %q", estado)
		}
	}
}

func TestElDocumentoDeclaraLosEstadosDeEnvioDelCodigo(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	for _, estado := range ipc.EstadosDeEnvioDeclarados() {
		if !strings.Contains(documento, "`"+estado+"`") {
			t.Errorf("el documento no declara el estado de envío %q", estado)
		}
	}
}

func causasDelDocumento(t *testing.T, documento string) []string {
	t.Helper()
	inicio := strings.Index(documento, "<!-- inicio-causas-estado-sesion -->")
	fin := strings.Index(documento, "<!-- fin-causas-estado-sesion -->")
	if inicio < 0 || fin < 0 || fin <= inicio {
		t.Fatalf("no se encontraron los delimitadores de la tabla de causas")
	}
	seccion := documento[inicio:fin]
	var causas []string
	for _, linea := range strings.Split(seccion, "\n") {
		linea = strings.TrimSpace(linea)
		if !strings.HasPrefix(linea, "| `") {
			continue
		}
		partes := strings.Split(linea, "|")
		if len(partes) < 3 {
			continue
		}
		causa := strings.Trim(strings.TrimSpace(partes[1]), "`")
		if causa != "" && causa != "causa" {
			causas = append(causas, causa)
		}
	}
	return causas
}
