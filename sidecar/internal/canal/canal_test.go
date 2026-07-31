package canal_test

import (
	"bytes"
	"errors"
	"log/slog"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

func TestNuevaSesionConstruyeElClienteSinAbrirNingunaConexion(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")

	sesion, err := canal.NuevaSesion(reg)
	if err != nil {
		t.Fatalf("NuevaSesion devolvió error: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	cliente := sesion.Cliente()
	if cliente == nil {
		t.Fatalf("la sesión no expone ningún cliente")
	}
	// El almacén no persistente no tiene credenciales, así que el cliente no puede estar
	// conectado ni autenticado. Es lo que hace que este test corra sin número de WhatsApp.
	if cliente.IsConnected() {
		t.Errorf("el cliente aparece conectado sin que nadie haya llamado a Conectar")
	}
	if cliente.IsLoggedIn() {
		t.Errorf("el cliente aparece autenticado sin emparejamiento")
	}

	if !strings.Contains(salida.String(), canal.EventoSesionConstruida) {
		t.Errorf("no se registró la construcción de la sesión: %s", salida.String())
	}
}

func TestNuevaSesionRechazaConstruirseSinRegistro(t *testing.T) {
	t.Parallel()

	if _, err := canal.NuevaSesion(nil); !errors.Is(err, canal.ErrRegistroNoEspecificado) {
		t.Fatalf("error = %v, se esperaba ErrRegistroNoEspecificado", err)
	}
}

func TestRegistrarManejadorEnganchaElManejadorDeEventosCrudos(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")

	sesion, err := canal.NuevaSesion(reg)
	if err != nil {
		t.Fatalf("NuevaSesion devolvió error: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	primero := sesion.RegistrarManejador()
	segundo := sesion.RegistrarManejador()
	if primero == 0 || segundo == 0 {
		t.Fatalf("los identificadores de manejador son %d y %d; se esperaban distintos de cero", primero, segundo)
	}
	if primero == segundo {
		t.Errorf("los dos manejadores comparten identificador %d", primero)
	}

	// Se retira el segundo para no dejar dos manejadores enganchados al cliente compartido.
	if !sesion.Cliente().RemoveEventHandler(segundo) {
		t.Errorf("no se pudo retirar el manejador %d", segundo)
	}
	if !sesion.Cliente().RemoveEventHandler(primero) {
		t.Errorf("no se pudo retirar el manejador %d", primero)
	}
}
