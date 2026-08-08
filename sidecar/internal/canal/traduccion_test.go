package canal_test

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
)

// Los escenarios que necesitan ejercitar procesarMensaje (unexported, por diseño del
// blueprint) viven en traduccion_interno_test.go. Este archivo prueba solo la superficie
// exportada: construcción, conformidad de tipos y las utilidades ajenas al traductor.

type buzonEspia struct{}

func (b *buzonEspia) Persistir(_ context.Context, _ string, _ []byte) error { return nil }

var _ canal.BuzonDurable = (*buzonEspia)(nil)

type sumideroEspia struct{}

func (s *sumideroEspia) recibir(ipc.EventoEntrante) {}

var _ canal.SumideroDeEvento = (&sumideroEspia{}).recibir

func TestNuevoTraductor_ConstruyeConLosCincoColaboradores(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(dir, "identidad.db")})
	if err != nil {
		t.Fatalf("error al abrir almacen de identidad: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })

	traductor := canal.NuevoTraductor(almacen, &buzonEspia{}, (&sumideroEspia{}).recibir, nil, nil)
	if traductor == nil {
		t.Fatalf("NuevoTraductor no debe devolver nil")
	}
}

func TestTraductor_ConfiguracionCargarRutaIdentidad(t *testing.T) {
	t.Parallel()
	entorno := func(valores map[string]string) func(string) (string, bool) {
		return func(clave string) (string, bool) {
			v, ok := valores[clave]
			return v, ok
		}
	}

	cfg, _ := configuracion.Cargar(entorno(map[string]string{}))
	if cfg.RutaIdentidad != identidad.RutaPorOmision {
		t.Errorf("se esperaba ruta por omisión")
	}

	cfg2, _ := configuracion.Cargar(entorno(map[string]string{"HEXCELL_RUTA_IDENTIDAD": "/tmp/otra.db"}))
	if cfg2.RutaIdentidad != "/tmp/otra.db" {
		t.Errorf("se esperaba ruta configurada")
	}

	_, err := configuracion.Cargar(entorno(map[string]string{"HEXCELL_RUTA_IDENTIDAD": ""}))
	if err != configuracion.ErrRutaIdentidadVacia {
		t.Errorf("se esperaba ErrRutaIdentidadVacia al estar la variable presente pero vacía, se obtuvo %v", err)
	}
}
