package identidad_test

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"go.mau.fi/whatsmeow/types"
	_ "modernc.org/sqlite"
)

func TestAlmacen_ReclamarPresentacion_ReclamoUnico(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	ruta := filepath.Join(dir, "identidad.db")

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("error al abrir almacen: %v", err)
	}
	defer almacen.Cerrar()

	ctx := context.Background()
	obs := identidad.Observacion{PN: types.JID{User: "5491155551234", Server: "s.whatsapp.net"}}
	ident, err := almacen.Resolver(ctx, obs)
	if err != nil {
		t.Fatalf("error al resolver identidad: %v", err)
	}

	// Primer reclamo: gana
	ganada1, err := almacen.ReclamarPresentacion(ctx, ident.IdInterno, "pres-1", 1000)
	if err != nil {
		t.Fatalf("primer reclamo falló: %v", err)
	}
	if !ganada1 {
		t.Fatal("primer reclamo debía ganar (ganada=true)")
	}

	// Segundo reclamo: pierde
	ganada2, err := almacen.ReclamarPresentacion(ctx, ident.IdInterno, "pres-2", 2000)
	if err != nil {
		t.Fatalf("segundo reclamo falló: %v", err)
	}
	if ganada2 {
		t.Fatal("segundo reclamo debía perder (ganada=false)")
	}

	if !strings.Contains(buf.String(), identidad.EventoPresentacionReclamada) {
		t.Errorf("registro no contiene %s: %s", identidad.EventoPresentacionReclamada, buf.String())
	}
}

func TestAlmacen_ReclamarPresentacion_ContencionConcurrente(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	ruta := filepath.Join(dir, "identidad.db")

	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir almacen: %v", err)
	}
	defer almacen.Cerrar()

	ctx := context.Background()
	obs := identidad.Observacion{PN: types.JID{User: "5491155550000", Server: "s.whatsapp.net"}}
	ident, err := almacen.Resolver(ctx, obs)
	if err != nil {
		t.Fatalf("error al resolver identidad: %v", err)
	}

	const concurrentes = 10
	var wg sync.WaitGroup
	resultados := make([]bool, concurrentes)
	errores := make([]error, concurrentes)

	for i := 0; i < concurrentes; i++ {
		wg.Add(1)
		go func(idx int) {
			defer wg.Done()
			idMensaje := "pres-concurrente-" + string(rune('A'+idx))
			ganada, err := almacen.ReclamarPresentacion(ctx, ident.IdInterno, idMensaje, int64(1000+idx))
			resultados[idx] = ganada
			errores[idx] = err
		}(i)
	}
	wg.Wait()

	ganadas := 0
	for i := 0; i < concurrentes; i++ {
		if errores[i] != nil {
			t.Fatalf("error en intento concurrente %d: %v", i, errores[i])
		}
		if resultados[i] {
			ganadas++
		}
	}

	if ganadas != 1 {
		t.Fatalf("exactamente 1 intento concurrente debía ganar el reclamo de presentación, ganaron %d", ganadas)
	}
}

func TestAlmacen_ReclamarPresentacion_SobreviveReinicio(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	ruta := filepath.Join(dir, "identidad.db")

	ctx := context.Background()
	obs := types.JID{User: "5491155559999", Server: "s.whatsapp.net"}

	// Primer proceso abre, resuelve y reclama
	almacen1, err := identidad.Abrir(identidad.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir almacen 1: %v", err)
	}
	ident1, err := almacen1.Resolver(ctx, identidad.Observacion{PN: obs})
	if err != nil {
		t.Fatalf("error al resolver en almacen 1: %v", err)
	}
	ganada1, err := almacen1.ReclamarPresentacion(ctx, ident1.IdInterno, "pres-1", 1000)
	if err != nil || !ganada1 {
		t.Fatalf("primer reclamo debía ganar: ganada=%v, err=%v", ganada1, err)
	}
	if err := almacen1.Cerrar(); err != nil {
		t.Fatalf("error al cerrar almacen 1: %v", err)
	}

	// Segundo proceso reabre sobre la misma ruta
	almacen2, err := identidad.Abrir(identidad.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir almacen 2: %v", err)
	}
	defer almacen2.Cerrar()

	ident2, err := almacen2.Resolver(ctx, identidad.Observacion{PN: obs})
	if err != nil {
		t.Fatalf("error al resolver en almacen 2: %v", err)
	}
	if ident2.IdInterno != ident1.IdInterno {
		t.Fatalf("id_interno no persistió tras reinicio: %s vs %s", ident2.IdInterno, ident1.IdInterno)
	}

	ganada2, err := almacen2.ReclamarPresentacion(ctx, ident2.IdInterno, "pres-reinicio", 2000)
	if err != nil {
		t.Fatalf("reclamo tras reinicio falló con error: %v", err)
	}
	if ganada2 {
		t.Fatal("segundo reclamo tras reinicio debía perder (RowsAffected en SQLite, no bandera en memoria)")
	}
}

func TestAlmacen_ReclamarPresentacion_AlmacenCerrado(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	ruta := filepath.Join(dir, "identidad.db")

	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir almacen: %v", err)
	}
	if err := almacen.Cerrar(); err != nil {
		t.Fatalf("error al cerrar almacen: %v", err)
	}

	_, err = almacen.ReclamarPresentacion(context.Background(), "ct-cerrado", "pres-1", 1000)
	if !errors.Is(err, identidad.ErrAlmacenCerrado) {
		t.Fatalf("se esperaba ErrAlmacenCerrado, se obtuvo %v", err)
	}
}

func TestAlmacen_DireccionDe_ExclusionEstructuralGruposDifusionEstados(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	ruta := filepath.Join(dir, "identidad.db")

	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir almacen: %v", err)
	}
	defer almacen.Cerrar()

	ctx := context.Background()
	obs := identidad.Observacion{PN: types.JID{User: "5491100001111", Server: "s.whatsapp.net"}}
	ident, err := almacen.Resolver(ctx, obs)
	if err != nil {
		t.Fatalf("error al resolver identidad: %v", err)
	}

	// Verificar que DireccionDe resuelve la JID PN
	jid, err := almacen.DireccionDe(ctx, ident.IdInterno)
	if err != nil {
		t.Fatalf("error en DireccionDe: %v", err)
	}
	if jid.Server != "s.whatsapp.net" {
		t.Errorf("servidor resuelto = %q, esperado 's.whatsapp.net'", jid.Server)
	}

	// Abrir conexión directa para comprobar que la restricción CHECK de SQLite rechaza tipos no soportados
	dsn := "file:" + ruta + "?_pragma=journal_mode(WAL)&_pragma=foreign_keys(1)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("error al abrir conexión directa SQLite: %v", err)
	}
	defer db.Close()

	// Intentar insertar tipo 'group', 'broadcast' o 'status' debe ser rechazado por CHECK (tipo IN ('pn','lid'))
	tiposInvalidos := []string{"group", "g.us", "broadcast", "status"}
	for _, tipoInv := range tiposInvalidos {
		_, err := db.Exec("INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?)",
			tipoInv, "12345", "g.us", ident.IdInterno, 1000)
		if err == nil {
			t.Errorf("se esperaba que CHECK(tipo IN ('pn','lid')) rechazara tipo %q", tipoInv)
		}
	}
}
