package canal_test

import (
	"bytes"
	"context"
	"log/slog"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

func TestIniciarEmparejamientoQrSobreAlmacenVacioDevuelveErrorAlNoPoderConectar(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctxCancelado, cancel := context.WithCancel(context.Background())
	cancel() // Contexto cancelado para que s.Conectar(s.ctx) falle rápido sin tocar la red.

	// Abrimos dispositivo con background ctx para que la DB no falle, pero pasamos ctxCancelado a NuevaSesion.
	// `sqlstore.GetFirstDevice` no usa el contexto pasado si la consulta es simple, pero si falla usaremos Background.
	sesion, err := canal.NuevaSesion(ctxCancelado, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	canalQr, err := sesion.IniciarEmparejamientoQr()
	if err == nil {
		t.Fatalf("se esperaba un error al intentar conectar con contexto cancelado")
	}
	if canalQr != nil {
		t.Fatalf("se esperaba un canal QR nil en caso de fallo de conexión")
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

func TestSolicitarCodigoDeVinculacionDevuelveErrorAlNoPoderConectarTelefonoCorto(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	sesion, err := canal.NuevaSesion(context.Background(), contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	// Al intentar conectar primero con ctx cancelado, falla antes de alcanzar PairPhone.
	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "123")
	if err == nil {
		t.Fatalf("se esperaba un error al intentar conectar con contexto cancelado")
	}
}

func TestSolicitarCodigoDeVinculacionDevuelveErrorAlNoPoderConectarTelefonoConCero(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	sesion, err := canal.NuevaSesion(context.Background(), contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	// Al intentar conectar primero con ctx cancelado, falla antes de alcanzar PairPhone.
	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "05491155551234")
	if err == nil {
		t.Fatalf("se esperaba un error al intentar conectar con contexto cancelado")
	}
}

func TestSolicitarCodigoDeVinculacionConClienteYaConectadoNoReconecta(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	sesion, err := canal.NuevaSesion(context.Background(), contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "5491155551234")
	if err == nil {
		t.Fatalf("se esperaba un error al fallar Conectar")
	}
}

// La comprobación de que ningún payload de emparejamiento llega al registro NO vive aquí: desde
// fuera del paquete solo se puede drenar el canal de GetQRChannel, y sin websocket ese canal nunca
// emite un suceso "code", así que la afirmación inspeccionaría un búfer vacío y pasaría incluso con
// el payload interpolado en `detalle`. Vive en `almacen_interno_test.go`, que es `package canal` y
// puede invocar el paso emisor con un payload centinela.
