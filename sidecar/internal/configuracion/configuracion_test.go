package configuracion_test

import (
	"errors"
	"log/slog"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
)

// entornoFalso construye una función de consulta del entorno a partir de un mapa, sin tocar el
// entorno real del proceso de test.
func entornoFalso(valores map[string]string) func(string) (string, bool) {
	return func(clave string) (string, bool) {
		valor, presente := valores[clave]
		return valor, presente
	}
}

func TestCargarAplicaLosValoresPorOmisionConEntornoVacio(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error con el entorno vacío: %v", err)
	}
	if cfg.RutaSocket != configuracion.RutaSocketPorOmision {
		t.Errorf("ruta del socket = %q, se esperaba %q", cfg.RutaSocket, configuracion.RutaSocketPorOmision)
	}
	if cfg.NivelDeRegistro != slog.LevelInfo {
		t.Errorf("nivel = %v, se esperaba info", cfg.NivelDeRegistro)
	}
	if cfg.IdCelula != configuracion.IdCelulaPorOmision {
		t.Errorf("id de célula = %q, se esperaba %q", cfg.IdCelula, configuracion.IdCelulaPorOmision)
	}
}

func TestCargarLeeLosTresParametrosDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableSocket:          "/tmp/celula/ipc.sock",
		configuracion.VariableNivelDeRegistro: "aviso",
		configuracion.VariableIdCelula:        "piloto-01",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.RutaSocket != "/tmp/celula/ipc.sock" {
		t.Errorf("ruta del socket = %q", cfg.RutaSocket)
	}
	if cfg.NivelDeRegistro != slog.LevelWarn {
		t.Errorf("nivel = %v, se esperaba aviso", cfg.NivelDeRegistro)
	}
	if cfg.IdCelula != "piloto-01" {
		t.Errorf("id de célula = %q", cfg.IdCelula)
	}
}

func TestCargarRechazaUnaRutaDeSocketVacia(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableSocket: "",
	}))
	if !errors.Is(err, configuracion.ErrRutaSocketVacia) {
		t.Fatalf("error = %v, se esperaba ErrRutaSocketVacia", err)
	}
}

func TestCargarRechazaUnNivelDeRegistroDesconocido(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableNivelDeRegistro: "verboso",
	}))
	if !errors.Is(err, configuracion.ErrNivelDeRegistroDesconocido) {
		t.Fatalf("error = %v, se esperaba ErrNivelDeRegistroDesconocido", err)
	}
}
