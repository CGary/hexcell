package identidad_test

import (
	"context"
	"database/sql"
	"errors"
	"io"
	"log/slog"
	"path/filepath"
	"sync"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"go.mau.fi/whatsmeow/types"
	_ "modernc.org/sqlite"
)

func crearContactoPrueba(t *testing.T, almacen *identidad.Almacen, user string) string {
	t.Helper()
	ctx := context.Background()
	pn := types.JID{User: user, Server: types.DefaultUserServer}
	id, err := almacen.Resolver(ctx, identidad.Observacion{PN: pn})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}
	return id.IdInterno
}

func TestRegistrarBaja_IdempotenteYRowsAffected(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000001")

	creada, err := almacen.RegistrarBaja(ctx, idInterno, 1000)
	if err != nil {
		t.Fatalf("RegistrarBaja: %v", err)
	}
	if !creada {
		t.Fatal("primera llamada a RegistrarBaja debía devolver creada=true")
	}

	// Segunda llamada para el mismo contacto
	creada2, err := almacen.RegistrarBaja(ctx, idInterno, 2000)
	if err != nil {
		t.Fatalf("RegistrarBaja (2): %v", err)
	}
	if creada2 {
		t.Fatal("segunda llamada a RegistrarBaja debía devolver creada=false")
	}
}

func TestRegistrarBaja_50ConcurrentesUnSoloGanador(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000002")

	const concurrencia = 50
	var wg sync.WaitGroup
	resultados := make([]bool, concurrencia)
	errores := make([]error, concurrencia)

	wg.Add(concurrencia)
	for i := 0; i < concurrencia; i++ {
		idx := i
		go func() {
			defer wg.Done()
			creada, err := almacen.RegistrarBaja(ctx, idInterno, int64(1000+idx))
			resultados[idx] = creada
			errores[idx] = err
		}()
	}
	wg.Wait()

	ganadores := 0
	for i := 0; i < concurrencia; i++ {
		if errores[i] != nil {
			t.Fatalf("error en llamada concurrente %d: %v", i, errores[i])
		}
		if resultados[i] {
			ganadores++
		}
	}

	if ganadores != 1 {
		t.Fatalf("se esperaba exactamente 1 ganador entre %d llamadas concurrentes, hubo %d", concurrencia, ganadores)
	}
}

func TestReclamarConfirmacionDeBaja_SinBaja_DevuelveErrBajaNoRegistrada(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000003")

	_, err := almacen.ReclamarConfirmacionDeBaja(ctx, idInterno, "msg-conf-1", 1000)
	if !errors.Is(err, identidad.ErrBajaNoRegistrada) {
		t.Fatalf("se esperaba ErrBajaNoRegistrada, se obtuvo: %v", err)
	}
}

func TestReclamarConfirmacionDeBaja_50ConcurrentesUnSoloReclamo(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000004")

	if _, err := almacen.RegistrarBaja(ctx, idInterno, 1000); err != nil {
		t.Fatalf("RegistrarBaja: %v", err)
	}

	const concurrencia = 50
	var wg sync.WaitGroup
	resultados := make([]bool, concurrencia)
	errores := make([]error, concurrencia)

	wg.Add(concurrencia)
	for i := 0; i < concurrencia; i++ {
		idx := i
		go func() {
			defer wg.Done()
			reclamada, err := almacen.ReclamarConfirmacionDeBaja(ctx, idInterno, "msg-conf-win", int64(2000+idx))
			resultados[idx] = reclamada
			errores[idx] = err
		}()
	}
	wg.Wait()

	ganadores := 0
	for i := 0; i < concurrencia; i++ {
		if errores[i] != nil {
			t.Fatalf("error en reclamo concurrente %d: %v", i, errores[i])
		}
		if resultados[i] {
			ganadores++
		}
	}

	if ganadores != 1 {
		t.Fatalf("se esperaba exactamente 1 reclamo exitoso entre %d concurrentes, hubo %d", concurrencia, ganadores)
	}

	// Llamadas posteriores deben devolver false, nil (ya gastada)
	reclamadaExtra, err := almacen.ReclamarConfirmacionDeBaja(ctx, idInterno, "msg-conf-extra", 3000)
	if err != nil {
		t.Fatalf("error en reclamo posterior: %v", err)
	}
	if reclamadaExtra {
		t.Fatal("el reclamo posterior debía devolver false")
	}
}

func TestEnvioPermitido_ReglaUnica(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000005")

	// 1. Sin baja: cualquier mensaje está permitido
	permitido, err := almacen.EnvioPermitido(ctx, idInterno, "msg-normal-1")
	if err != nil || !permitido {
		t.Fatalf("contacto sin baja debía permitir envío, permitido=%v, err=%v", permitido, err)
	}

	// 2. Con baja registrada pero sin confirmación asignada: nada está permitido
	if _, err := almacen.RegistrarBaja(ctx, idInterno, 1000); err != nil {
		t.Fatalf("RegistrarBaja: %v", err)
	}

	permitido, err = almacen.EnvioPermitido(ctx, idInterno, "msg-normal-2")
	if err != nil || permitido {
		t.Fatalf("contacto con baja no debía permitir mensaje ordinario, permitido=%v, err=%v", permitido, err)
	}

	// 3. Reclamar confirmación asignando id_mensaje_confirmacion = "msg-conf-1"
	if _, err := almacen.ReclamarConfirmacionDeBaja(ctx, idInterno, "msg-conf-1", 2000); err != nil {
		t.Fatalf("ReclamarConfirmacionDeBaja: %v", err)
	}

	// La confirmación asignada es permitida
	permitidoConf, err := almacen.EnvioPermitido(ctx, idInterno, "msg-conf-1")
	if err != nil || !permitidoConf {
		t.Fatalf("el mensaje de confirmación asignado debía estar permitido, permitido=%v, err=%v", permitidoConf, err)
	}

	// Cualquier otro mensaje sigue bloqueado
	permitidoOtro, err := almacen.EnvioPermitido(ctx, idInterno, "msg-otro")
	if err != nil || permitidoOtro {
		t.Fatalf("otro mensaje para contacto dado de baja debía estar bloqueado, permitido=%v, err=%v", permitidoOtro, err)
	}
}

func TestBajaSobreviveReinicioYReconstruccionDeSqlstore(t *testing.T) {
	t.Parallel()
	ruta := filepath.Join(t.TempDir(), "identidad_persistencia.db")
	reg := registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("Abrir: %v", err)
	}

	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000006")
	if _, err := almacen.RegistrarBaja(ctx, idInterno, 1000); err != nil {
		t.Fatalf("RegistrarBaja: %v", err)
	}
	almacen.Cerrar()

	// Reconstrucción real del sqlstore whatsmeow: se crea un contenedor VACÍO nuevo, como
	// deja un re-emparejamiento del dispositivo. La baja vive en identidad.db, no en el
	// sqlstore, así que debe sobrevivir intacta.
	ctxSqlstore := context.Background()
	contenedor, err := canal.AbrirAlmacenDeDispositivo(ctxSqlstore, filepath.Join(t.TempDir(), "sqlstore_reconstruido.db"), reg)
	if err != nil {
		t.Fatalf("AbrirAlmacenDeDispositivo: %v", err)
	}
	defer contenedor.Close()

	almacen2, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("Reabrir: %v", err)
	}
	defer almacen2.Cerrar()

	permitido, err := almacen2.EnvioPermitido(ctx, idInterno, "msg-post-reinicio")
	if err != nil {
		t.Fatalf("EnvioPermitido tras reabrir: %v", err)
	}
	if permitido {
		t.Fatal("el contacto debe permanecer bloqueado tras reabrir el almacén de identidad")
	}
}

func TestBaja_AperturaBaseExistenteSinTablaGanaBajaDeContacto(t *testing.T) {
	t.Parallel()
	ruta := filepath.Join(t.TempDir(), "identidad_legacy.db")
	dsn := "file:" + ruta + "?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("sql.Open: %v", err)
	}
	esquemaAntiguo := `
	CREATE TABLE IF NOT EXISTS identidad (
	  id_interno TEXT PRIMARY KEY,
	  estado TEXT NOT NULL CHECK (estado IN ('provisional','anclada','fusionada')),
	  fusionada_en TEXT NULL REFERENCES identidad(id_interno) ON DELETE RESTRICT,
	  creada_en_ms INTEGER NOT NULL,
	  actualizada_en_ms INTEGER NOT NULL
	);
	CREATE TABLE IF NOT EXISTS direccion (
	  tipo TEXT NOT NULL CHECK (tipo IN ('pn','lid')),
	  usuario TEXT NOT NULL,
	  servidor TEXT NOT NULL,
	  id_interno TEXT NOT NULL REFERENCES identidad(id_interno) ON DELETE CASCADE,
	  observada_en_ms INTEGER NOT NULL,
	  PRIMARY KEY (tipo, usuario, servidor)
	);
	INSERT INTO identidad (id_interno, estado, creada_en_ms, actualizada_en_ms) VALUES ('ct-existente', 'anclada', 100, 100);
	INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES ('pn', '5491199999999', 's.whatsapp.net', 'ct-existente', 100);
	`
	if _, err := db.Exec(esquemaAntiguo); err != nil {
		t.Fatalf("db.Exec antiguo: %v", err)
	}
	db.Close()

	// Abrir con identidad.Abrir debe aplicar CREATE TABLE IF NOT EXISTS para baja_de_contacto sin alterar datos
	reg := registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("Abrir sobre base antigua: %v", err)
	}
	defer almacen.Cerrar()

	// El contacto existente debe seguir existiendo y resolverse
	ctx := context.Background()
	dir, err := almacen.DireccionDe(ctx, "ct-existente")
	if err != nil || dir.User != "5491199999999" {
		t.Fatalf("datos existentes corruptos o ausentes: dir=%v, err=%v", dir, err)
	}

	// Y debe poder registrar su baja en la tabla nueva sin migración manual
	creada, err := almacen.RegistrarBaja(ctx, "ct-existente", 2000)
	if err != nil || !creada {
		t.Fatalf("falló registrar baja en tabla recién creada: creada=%v, err=%v", creada, err)
	}
}
