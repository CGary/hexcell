// Package outbox_test implementa el centinela de rutas de envío.
//
// NOTA PARA LA ETAPA A-3 TAREA 12:
// Este archivo actúa como centinela para asegurar que todas las llamadas de envío
// pasen a través del buzón de salida (outbox) y no se invoquen directamente en otras partes del código.
// La tarea 12 de la etapa A-3 puede extender la lista de llamadas vigiladas
// o eximir algún directorio en su lugar, pero no debe borrar ni deshabilitar esta guardia.
// Fecha: 2026-08-09.
package outbox_test

import (
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// detectarLlamadasDeEnvio recorre un directorio o analiza un código sintético para
// detectar llamadas a funciones de envío que no pasen por el outbox. Devuelve además el
// número de archivos analizados: un corpus vacío convertiría el centinela en un no-op
// silencioso y el test principal debe rechazarlo.
func detectarLlamadasDeEnvio(ruta string, codigoSintetico string) ([]string, int, error) {
	var violaciones []string
	archivosAnalizados := 0

	// esRutaDeEnvioVigilada reconoce los nombres de función cuya llamada directa constituye
	// una violación: enviar solo se permite dentro del módulo outbox.
	esRutaDeEnvioVigilada := func(nombre string) bool {
		switch nombre {
		case "SendMessage", "SendPresence", "SendChatPresence", "SendReceipt", "Transmitir":
			return true
		default:
			return strings.HasPrefix(nombre, "Enviar")
		}
	}

	analizarArchivo := func(rutaArchivo string, codigo any) error {
		fset := token.NewFileSet()
		nodo, err := parser.ParseFile(fset, rutaArchivo, codigo, 0)
		if err != nil {
			return err
		}
		archivosAnalizados++

		ast.Inspect(nodo, func(n ast.Node) bool {
			llamada, ok := n.(*ast.CallExpr)
			if !ok {
				return true
			}

			var nombreFuncion string
			switch exp := llamada.Fun.(type) {
			case *ast.Ident:
				nombreFuncion = exp.Name
			case *ast.SelectorExpr:
				nombreFuncion = exp.Sel.Name
			}

			if nombreFuncion != "" && esRutaDeEnvioVigilada(nombreFuncion) {
				violaciones = append(violaciones, fmt.Sprintf("%s: %s", rutaArchivo, nombreFuncion))
			}

			return true
		})
		return nil
	}

	if codigoSintetico != "" {
		err := analizarArchivo(ruta, codigoSintetico)
		return violaciones, archivosAnalizados, err
	}

	err := filepath.Walk(ruta, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}

		if info.IsDir() {
			nombre := info.Name()
			if nombre == "vendor" || nombre == "outbox" {
				return filepath.SkipDir
			}
			return nil
		}

		if !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_test.go") {
			return nil
		}

		return analizarArchivo(path, nil)
	})

	return violaciones, archivosAnalizados, err
}

func TestCentinelaSinRutasDeEnvioFueraDelOutbox(t *testing.T) {
	t.Parallel()

	// La ruta raíz es sidecar/ (subimos desde sidecar/internal/outbox)
	rutaRaiz := filepath.Join("..", "..")

	violaciones, archivosAnalizados, err := detectarLlamadasDeEnvio(rutaRaiz, "")
	if err != nil {
		t.Fatalf("error al recorrer el directorio: %v", err)
	}

	// Un corpus vacío significa que el recorrido ya no encuentra el código del sidecar
	// (p. ej. cambió la disposición de directorios): el centinela estaría pasando en vacío.
	if archivosAnalizados == 0 {
		t.Fatalf("el centinela no analizó ningún archivo: la ruta raíz %q ya no cubre el código del sidecar", rutaRaiz)
	}

	if len(violaciones) > 0 {
		t.Errorf("se encontraron rutas de envío directas fuera del outbox:\n%s", strings.Join(violaciones, "\n"))
	}
}

func TestCentinelaDetectaUnaRutaDeEnvioSintetica(t *testing.T) {
	t.Parallel()

	codigoSintetico := `
package trampa

import "fmt"

func EnviarMensajeTramposo() {
	cliente.SendMessage(ctx, jid, mensaje)
	transmisor.Transmitir(ctx, conv, cont)
}
`
	violaciones, _, err := detectarLlamadasDeEnvio("trampa.go", codigoSintetico)
	if err != nil {
		t.Fatalf("error al procesar el código sintético: %v", err)
	}

	if len(violaciones) != 2 {
		t.Fatalf("se esperaban exactamente 2 violaciones, se encontraron %d: %v", len(violaciones), violaciones)
	}
}
