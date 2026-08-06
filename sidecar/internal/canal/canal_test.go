package canal_test

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"path/filepath"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/store/sqlstore"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// abrirAlmacenDePrueba abre un sqlstore.Container en un directorio temporal del test.
func abrirAlmacenDePrueba(t *testing.T, reg *registro.Registro) *sqlstore.Container {
	t.Helper()
	ctx := context.Background()
	ruta := filepath.Join(t.TempDir(), "sqlstore.db")
	contenedor, err := canal.AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("AbrirAlmacenDeDispositivo: %v", err)
	}
	t.Cleanup(func() { contenedor.Close() })
	return contenedor
}

func TestAbrirAlmacenDeDispositivoAbreYActualizaSinRed(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")

	ctx := context.Background()
	ruta := filepath.Join(t.TempDir(), "sqlstore.db")
	contenedor, err := canal.AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("no se pudo abrir el almacén: %v", err)
	}
	defer contenedor.Close()

	if !strings.Contains(salida.String(), canal.EventoAlmacenAbierto) {
		t.Errorf("no se registró la apertura del almacén: %s", salida.String())
	}
}

func TestAlmacenVacioDevuelveDispositivoSinIdYLaSesionEsNoEmparejada(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	if sesion.EstaEmparejada() {
		t.Errorf("la sesión aparece emparejada sobre un almacén vacío")
	}
	if sesion.Cliente() == nil {
		t.Fatalf("la sesión no expone ningún cliente")
	}
	if sesion.Cliente().IsConnected() {
		t.Errorf("el cliente aparece conectado sin que nadie haya llamado a Conectar")
	}
	if sesion.Cliente().EnableAutoReconnect || sesion.Cliente().InitialAutoReconnect {
		t.Errorf("la autoreconexion propia de whatsmeow quedó activa")
	}
	if sesion.Cliente().AutoReconnectHook == nil || sesion.Cliente().AutoReconnectHook(errors.New("fallo")) {
		t.Errorf("el gancho de autoreconexion de whatsmeow permite reintentar")
	}

	if !strings.Contains(salida.String(), canal.EventoDispositivoNuevo) {
		t.Errorf("no se registró que el almacén está vacío: %s", salida.String())
	}
}

func TestNuevaSesionRechazaConstruirseSinRegistro(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	if _, err := canal.NuevaSesion(ctx, contenedor, nil); !errors.Is(err, canal.ErrRegistroNoEspecificado) {
		t.Fatalf("error = %v, se esperaba ErrRegistroNoEspecificado", err)
	}
}

func TestRegistrarManejadorEnganchaElManejador(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
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

	sesion.Cliente().RemoveEventHandler(segundo)
	sesion.Cliente().RemoveEventHandler(primero)
}
