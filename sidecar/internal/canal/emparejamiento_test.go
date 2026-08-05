package canal_test

import (
	"bytes"
	"context"
	"log/slog"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

func TestIniciarEmparejamientoQrSobreAlmacenVacioDevuelveCanalSinError(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	canalQr, err := sesion.IniciarEmparejamientoQr()
	if err != nil {
		t.Fatalf("IniciarEmparejamientoQr: %v", err)
	}
	if canalQr == nil {
		t.Fatalf("el canal QR es nil")
	}

	if !strings.Contains(salida.String(), canal.EventoEmparejamientoQrIniciado) {
		t.Errorf("no se registró el inicio del emparejamiento QR: %s", salida.String())
	}
}

func TestSolicitarCodigoDeVinculacionRechazaTelefonoVacio(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "")
	if err != canal.ErrTelefonoNoConfigurado {
		t.Fatalf("error = %v, se esperaba ErrTelefonoNoConfigurado", err)
	}
}

func TestSolicitarCodigoDeVinculacionRechazaTelefonoCorto(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	// whatsmeow valida el número antes de cualquier I/O: números cortos se rechazan.
	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "123")
	if err == nil {
		t.Fatalf("se esperaba un error con un número demasiado corto")
	}
}

func TestSolicitarCodigoDeVinculacionRechazaTelefonoConCero(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	// whatsmeow rechaza números que empiezan con 0.
	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "05491155551234")
	if err == nil {
		t.Fatalf("se esperaba un error con un número que empieza con 0")
	}
}

// La comprobación de que ningún payload de emparejamiento llega al registro NO vive aquí: desde
// fuera del paquete solo se puede drenar el canal de GetQRChannel, y sin websocket ese canal nunca
// emite un suceso "code", así que la afirmación inspeccionaría un búfer vacío y pasaría incluso con
// el payload interpolado en `detalle`. Vive en `almacen_interno_test.go`, que es `package canal` y
// puede invocar el paso emisor con un payload centinela.
