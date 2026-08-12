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

func TestNuevoTraductor_ConstruyeConLosOchoColaboradores(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: filepath.Join(dir, "identidad.db")})
	if err != nil {
		t.Fatalf("error al abrir almacen de identidad: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })

	traductor := canal.NuevoTraductor(almacen, &buzonEspia{}, (&sumideroEspia{}).recibir, nil, nil, nil, nil, nil)
	if traductor == nil {
		t.Fatalf("NuevoTraductor no debe devolver nil")
	}
}

func TestSelectorDePlantilla_SeleccionDeterministaSinAleatoriedad(t *testing.T) {
	t.Parallel()

	variantes := []string{
		"¡Hola! Gracias por escribir.",
		"Hola, ¿en qué te puedo ayudar?",
		"Buenas, gracias por tu mensaje.",
	}
	selector := canal.NuevoSelectorDePlantilla(variantes)

	id1 := "ct-00000000000000000000000000000001"
	id2 := "ct-00000000000000000000000000000002"

	// 1. Determinismo y estabilidad: 100 llamadas para el mismo id_interno devuelven la misma variante
	v1Primera := selector.Elegir(id1)
	for i := 0; i < 100; i++ {
		v := selector.Elegir(id1)
		if v != v1Primera {
			t.Fatalf("la selección para id1 no fue determinista en el intento %d: %q vs %q", i, v, v1Primera)
		}
	}

	v2Primera := selector.Elegir(id2)
	for i := 0; i < 100; i++ {
		v := selector.Elegir(id2)
		if v != v2Primera {
			t.Fatalf("la selección para id2 no fue determinista en el intento %d: %q vs %q", i, v, v2Primera)
		}
	}

	// 2. Comprobar que dos id_internos diferentes en la misma célula pueden seleccionar variantes distintas
	// Probamos varios IDs para comprobar variación efectiva en el conjunto de 3 variantes
	variantesEncontradas := make(map[string]bool)
	for i := 0; i < 20; i++ {
		id := "ct-test-contact-" + string(rune('A'+i))
		v := selector.Elegir(id)
		variantesEncontradas[v] = true
	}
	if len(variantesEncontradas) < 2 {
		t.Errorf("se esperaba que el selector distribuyera entre más de 1 variante, sólo se encontraron: %v", variantesEncontradas)
	}
}

func TestComponerPresentacion_Formato(t *testing.T) {
	t.Parallel()

	variante := "¡Hola! Gracias por escribir."
	identificacion := "Te atiende un asistente automático. Si preferís hablar con una persona, escribí «humano»."

	compuesto := canal.ComponerPresentacion(variante, identificacion)
	esperado := "¡Hola! Gracias por escribir. Te atiende un asistente automático. Si preferís hablar con una persona, escribí «humano»."

	if compuesto != esperado {
		t.Errorf("ComponerPresentacion = %q, esperado %q", compuesto, esperado)
	}

	// Casos de borde vacíos
	if canal.ComponerPresentacion("", identificacion) != identificacion {
		t.Errorf("con variante vacía debía devolver sólo identificación")
	}
	if canal.ComponerPresentacion(variante, "") != variante {
		t.Errorf("con identificación vacía debía devolver sólo variante")
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
