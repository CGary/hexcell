// Esqueleto mínimo del sidecar Go de HexCell. Este binario compila y arranca, pero no
// abre ninguna conexión, sesión ni pairing de WhatsApp: esa lógica pertenece a la etapa
// A-3 (adaptador whatsmeow). El único propósito de importar go.mau.fi/whatsmeow aquí es
// mantener la dependencia viva en go.sum — un `require` sin ningún import correspondiente
// sería eliminado por `go mod tidy`, y entonces la fijación de versión de AC-2 dejaría de
// ser verificable con herramientas.
package main

import (
	"fmt"

	_ "go.mau.fi/whatsmeow"
)

func main() {
	fmt.Println("hexcell-sidecar: esqueleto en pie, sin sesión de WhatsApp todavía")
}
