package identidad_test

import (
	"bytes"
	"context"
	"encoding/hex"
	"io"
	"log/slog"
	"path/filepath"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"go.mau.fi/whatsmeow/types"
)

func abrirAlmacenDePrueba(t *testing.T) *identidad.Almacen {
	t.Helper()
	ruta := filepath.Join(t.TempDir(), "identidad.db")
	almacen, err := identidad.Abrir(identidad.Opciones{
		Ruta:     ruta,
		Registro: registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test"),
	})
	if err != nil {
		t.Fatalf("Abrir: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })
	return almacen
}

func TestAbrir_CreaArchivoYAplicaEsquemaIdempotente(t *testing.T) {
	ruta := filepath.Join(t.TempDir(), "identidad.db")
	opc := identidad.Opciones{
		Ruta:     ruta,
		Registro: registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test"),
	}

	almacen, err := identidad.Abrir(opc)
	if err != nil {
		t.Fatalf("Primera apertura falló: %v", err)
	}
	almacen.Cerrar()

	// Segunda apertura, debe ser idempotente
	almacen2, err := identidad.Abrir(opc)
	if err != nil {
		t.Fatalf("Segunda apertura falló: %v", err)
	}
	almacen2.Cerrar()
}

func TestGenerarIdInterno_FormatoYUnicidad(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()

	// Llamar a Resolver para que genere ids internamente
	pn1 := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	pn2 := types.JID{User: "5491155551235", Server: types.DefaultUserServer}

	id1, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn1})
	id2, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn2})

	verificarFormato := func(id string) {
		if !strings.HasPrefix(id, identidad.PrefijoIdentidad) {
			t.Errorf("Id %q no tiene prefijo %q", id, identidad.PrefijoIdentidad)
		}
		if len(id) != 35 {
			t.Errorf("Id %q tiene longitud %d, se esperaba 35", id, len(id))
		}
		parteHex := strings.TrimPrefix(id, identidad.PrefijoIdentidad)
		if strings.ToLower(parteHex) != parteHex {
			t.Errorf("Id %q debe ser minúscula pura", id)
		}
		if _, err := hex.DecodeString(parteHex); err != nil {
			t.Errorf("Id %q no tiene hex válido: %v", id, err)
		}
	}

	verificarFormato(id1.IdInterno)
	verificarFormato(id2.IdInterno)

	if id1.IdInterno == id2.IdInterno {
		t.Errorf("Dos ids generados consecutivamente colisionaron")
	}
}

func TestResolver_SoloPN_CreaIdentidadAnclada(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}

	res, err := almacen.Resolver(ctx, identidad.Observacion{PN: pn})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}

	if res.Estado != identidad.EstadoAnclada {
		t.Errorf("Estado esperado %q, obtenido %q", identidad.EstadoAnclada, res.Estado)
	}
}

func TestResolver_SoloLID_CreaIdentidadProvisional(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	lid := types.JID{User: "abc123", Server: "lid"}

	res, err := almacen.Resolver(ctx, identidad.Observacion{LID: lid})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}

	if res.Estado != identidad.EstadoProvisional {
		t.Errorf("Estado esperado %q, obtenido %q", identidad.EstadoProvisional, res.Estado)
	}
}

func TestResolver_MismoPN_DevuelveMismoIdInterno(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}

	res1, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn})
	res2, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn})

	if res1.IdInterno != res2.IdInterno {
		t.Errorf("Esperaba el mismo id interno, obtuve %q y %q", res1.IdInterno, res2.IdInterno)
	}
}

func TestResolver_PNLuegoLID_AdjuntaAliasSinMintear(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	res1, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn})
	res2, err := almacen.Resolver(ctx, identidad.Observacion{PN: pn, LID: lid})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}

	if res1.IdInterno != res2.IdInterno {
		t.Errorf("Id interno cambió al adjuntar alias")
	}
}

func TestResolver_FusionConservaElMasAntiguo(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	// Crear LID primero (provisional) -> es el más antiguo
	resLid, _ := almacen.Resolver(ctx, identidad.Observacion{LID: lid})

	// Crear PN después (anclada)
	resPn, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn})

	// Fusionar
	resFusion, err := almacen.Resolver(ctx, identidad.Observacion{PN: pn, LID: lid})
	if err != nil {
		t.Fatalf("Resolver fusión: %v", err)
	}

	if resFusion.IdInterno != resLid.IdInterno {
		t.Errorf("Se esperaba que sobreviviera el más antiguo %q, sobrevivió %q", resLid.IdInterno, resFusion.IdInterno)
	}
	if resFusion.IdInterno == resPn.IdInterno {
		t.Errorf("El id interno más joven %q no debía sobrevivir a la fusión", resPn.IdInterno)
	}
	if resFusion.Estado != identidad.EstadoAnclada {
		t.Errorf("La identidad resultante debe ser anclada")
	}
}

func TestResolver_BusquedaDeFusionadaSigueUnSalto(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	resLid, _ := almacen.Resolver(ctx, identidad.Observacion{LID: lid})
	almacen.Resolver(ctx, identidad.Observacion{PN: pn})
	almacen.Resolver(ctx, identidad.Observacion{PN: pn, LID: lid}) // fusiona, lid sobrevive

	// Ahora, la búsqueda solo por PN (que originalmente generó la otra) debe seguir un salto y retornar la que sobrevivió
	resSigueSalto, err := almacen.Resolver(ctx, identidad.Observacion{PN: pn})
	if err != nil {
		t.Fatalf("Resolver sigue salto: %v", err)
	}

	if resSigueSalto.IdInterno != resLid.IdInterno {
		t.Errorf("Debería haber seguido el salto a %q, obtuve %q", resLid.IdInterno, resSigueSalto.IdInterno)
	}
}

func TestResolver_ConflictoDePNDistinto_NoFusiona(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn1 := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	pn2 := types.JID{User: "5491155559999", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	// Ligar LID a PN1
	res1, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn1, LID: lid})

	// Intentar fusionar LID a PN2
	res2, err := almacen.Resolver(ctx, identidad.Observacion{PN: pn2, LID: lid})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}

	if !res2.ConflictoDeAlias {
		t.Errorf("Se esperaba ConflictoDeAlias en true")
	}
	if res1.IdInterno == res2.IdInterno {
		t.Errorf("No se debe fusionar con un PN distinto")
	}
}

func TestDireccionDe_PNParaAnclada_LIDParaProvisional(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	resAnclada, _ := almacen.Resolver(ctx, identidad.Observacion{PN: pn, LID: lid})
	resProvisional, _ := almacen.Resolver(ctx, identidad.Observacion{LID: types.JID{User: "soloLid", Server: "lid"}})

	dirAnclada, err := almacen.DireccionDe(ctx, resAnclada.IdInterno)
	if err != nil {
		t.Fatalf("DireccionDe anclada: %v", err)
	}
	if dirAnclada.String() != pn.String() {
		t.Errorf("Se esperaba %v, obtuve %v", pn, dirAnclada)
	}

	dirProvisional, err := almacen.DireccionDe(ctx, resProvisional.IdInterno)
	if err != nil {
		t.Fatalf("DireccionDe provisional: %v", err)
	}
	if dirProvisional.Server != "lid" || dirProvisional.User != "soloLid" {
		t.Errorf("Se esperaba LID soloLid@lid, obtuve %v", dirProvisional)
	}
}

func TestRegistro_NoContieneJIDTrasResolver(t *testing.T) {
	var buf bytes.Buffer
	ruta := filepath.Join(t.TempDir(), "identidad.db")
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	almacen, err := identidad.Abrir(identidad.Opciones{
		Ruta:     ruta,
		Registro: reg,
	})
	if err != nil {
		t.Fatalf("Abrir: %v", err)
	}
	defer almacen.Cerrar()

	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	_, err = almacen.Resolver(ctx, identidad.Observacion{PN: pn, LID: lid})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}

	salida := buf.String()
	if salida == "" {
		t.Fatalf("el registro está vacío: la prueba no ejercita ninguna línea de log")
	}
	if strings.Contains(salida, "@") || strings.Contains(salida, "s.whatsapp.net") || strings.Contains(salida, "lid") {
		t.Errorf("El registro contiene identificadores o un JID: %s", salida)
	}
}

func TestCerrar_Idempotente(t *testing.T) {
	almacen := abrirAlmacenDePrueba(t)

	if err := almacen.Cerrar(); err != nil {
		t.Fatalf("Primer cerrado falló: %v", err)
	}
	if err := almacen.Cerrar(); err != nil {
		t.Fatalf("Segundo cerrado falló: %v", err)
	}
}
