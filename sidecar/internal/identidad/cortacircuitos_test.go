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

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	_ "modernc.org/sqlite"
)

// Cortacircuitos conversacional [causa documentada]

func TestCortacircuitos_DisparoPorRepeticion_Umbral3(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000010")

	const umbral = int64(3)

	// Primer mensaje: no dispara
	disp1, err := almacen.RegistrarObservacion(ctx, idInterno, "hola", false, umbral, 1000)
	if err != nil || disp1 {
		t.Fatalf("primera observacion: disp=%v, err=%v", disp1, err)
	}

	// Segundo mensaje idéntico: no dispara
	disp2, err := almacen.RegistrarObservacion(ctx, idInterno, "hola", false, umbral, 2000)
	if err != nil || disp2 {
		t.Fatalf("segunda observacion: disp=%v, err=%v", disp2, err)
	}

	// Tercer mensaje idéntico: dispara
	disp3, err := almacen.RegistrarObservacion(ctx, idInterno, "hola", false, umbral, 3000)
	if err != nil || !disp3 {
		t.Fatalf("tercera observacion debia disparar: disp=%v, err=%v", disp3, err)
	}

	// Cuarto mensaje (idempotencia tras disparo): no dispara de nuevo
	disp4, err := almacen.RegistrarObservacion(ctx, idInterno, "hola", false, umbral, 4000)
	if err != nil || disp4 {
		t.Fatalf("cuarta observacion tras disparo: disp=%v, err=%v", disp4, err)
	}
}

func TestCortacircuitos_DisparoInmediatoPorFrustracion(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000011")

	// Dispara de inmediato en el primer mensaje
	disp, err := almacen.RegistrarObservacion(ctx, idInterno, "humano", true, 3, 1000)
	if err != nil || !disp {
		t.Fatalf("observacion con frustracion debia disparar inmediatamente: disp=%v, err=%v", disp, err)
	}
}

func TestCortacircuitos_TextoDistintoReiniciaContador(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000012")

	const umbral = int64(3)

	// Dos veces "hola"
	almacen.RegistrarObservacion(ctx, idInterno, "hola", false, umbral, 1000)
	almacen.RegistrarObservacion(ctx, idInterno, "hola", false, umbral, 2000)

	// Texto distinto: reinicia contador a 1
	disp, err := almacen.RegistrarObservacion(ctx, idInterno, "chau", false, umbral, 3000)
	if err != nil || disp {
		t.Fatalf("texto distinto no debia disparar: disp=%v, err=%v", disp, err)
	}

	// Otra vez "chau" (2da repetición de chau): no dispara
	disp, err = almacen.RegistrarObservacion(ctx, idInterno, "chau", false, umbral, 4000)
	if err != nil || disp {
		t.Fatalf("segunda repeticion de chau no debia disparar: disp=%v, err=%v", disp, err)
	}

	// Tercera repetición de "chau": dispara
	disp, err = almacen.RegistrarObservacion(ctx, idInterno, "chau", false, umbral, 5000)
	if err != nil || !disp {
		t.Fatalf("tercera repeticion de chau debia disparar: disp=%v, err=%v", disp, err)
	}
}

func TestCortacircuitos_SalidaPermitida_Y_ReclamoUnicoTraspaso(t *testing.T) {
	t.Parallel()
	almacen := abrirAlmacenDePrueba(t)
	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000013")

	// 1. Sin disparo: salida permitida
	permitido, err := almacen.SalidaPermitida(ctx, idInterno, "msg-1")
	if err != nil || !permitido {
		t.Fatalf("sin disparo debia permitir salida: permitido=%v, err=%v", permitido, err)
	}

	// Reclamar sin disparo devuelve ErrCortacircuitosNoDisparado
	_, err = almacen.ReclamarMensajeDeTraspaso(ctx, idInterno, "msg-traspaso-1", 1000)
	if !errors.Is(err, identidad.ErrCortacircuitosNoDisparado) {
		t.Fatalf("se esperaba ErrCortacircuitosNoDisparado, se obtuvo %v", err)
	}

	// 2. Disparar cortacircuitos
	disp, err := almacen.RegistrarObservacion(ctx, idInterno, "operador", true, 3, 2000)
	if err != nil || !disp {
		t.Fatalf("disparo falló: disp=%v, err=%v", disp, err)
	}

	// Salida bloqueada para mensajes ordinarios
	permitido, err = almacen.SalidaPermitida(ctx, idInterno, "msg-ordinario")
	if err != nil || permitido {
		t.Fatalf("con cortacircuitos disparado debia bloquear mensaje ordinario: permitido=%v, err=%v", permitido, err)
	}

	// 3. Contienda concurrente para reclamar mensaje de traspaso (50 llamadas)
	const concurrencia = 50
	var wg sync.WaitGroup
	resultados := make([]bool, concurrencia)
	errores := make([]error, concurrencia)

	wg.Add(concurrencia)
	for i := 0; i < concurrencia; i++ {
		idx := i
		go func() {
			defer wg.Done()
			rec, err := almacen.ReclamarMensajeDeTraspaso(ctx, idInterno, "msg-traspaso-win", int64(3000+idx))
			resultados[idx] = rec
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
		t.Fatalf("se esperaba exactamente 1 ganador de traspaso, hubo %d", ganadores)
	}

	// El mensaje de traspaso asignado está permitido
	permitidoTraspaso, err := almacen.SalidaPermitida(ctx, idInterno, "msg-traspaso-win")
	if err != nil || !permitidoTraspaso {
		t.Fatalf("el mensaje de traspaso asignado debia estar permitido: permitido=%v, err=%v", permitidoTraspaso, err)
	}

	// Otro mensaje sigue bloqueado
	permitidoOtro, err := almacen.SalidaPermitida(ctx, idInterno, "msg-otro")
	if err != nil || permitidoOtro {
		t.Fatalf("otro mensaje debia estar bloqueado: permitido=%v, err=%v", permitidoOtro, err)
	}

	// 4. Restablecer
	if err := almacen.Restablecer(ctx, idInterno); err != nil {
		t.Fatalf("Restablecer falló: %v", err)
	}

	// Tras restablecer, la salida vuelve a estar permitida para cualquier mensaje
	permitidoPostReset, err := almacen.SalidaPermitida(ctx, idInterno, "msg-ordinario-post-reset")
	if err != nil || !permitidoPostReset {
		t.Fatalf("tras restablecer la salida debia estar permitida: permitido=%v, err=%v", permitidoPostReset, err)
	}
}

func TestCortacircuitos_PersistenciaTrasCierreYReapertura(t *testing.T) {
	t.Parallel()
	ruta := filepath.Join(t.TempDir(), "identidad_cortacircuitos_persistencia.db")
	reg := registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("Abrir: %v", err)
	}

	ctx := context.Background()
	idInterno := crearContactoPrueba(t, almacen, "5491100000014")

	// Disparar y reclamar
	disp, err := almacen.RegistrarObservacion(ctx, idInterno, "humano", true, 3, 1000)
	if err != nil || !disp {
		t.Fatalf("disparo: disp=%v, err=%v", disp, err)
	}
	reclamada, err := almacen.ReclamarMensajeDeTraspaso(ctx, idInterno, "msg-traspaso-durable", 1000)
	if err != nil || !reclamada {
		t.Fatalf("reclamo traspaso: rec=%v, err=%v", reclamada, err)
	}

	almacen.Cerrar()

	// Reabrir almacén
	almacen2, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("Reabrir: %v", err)
	}
	defer almacen2.Cerrar()

	// Mensaje común debe seguir bloqueado tras reinicio
	permitidoComun, err := almacen2.SalidaPermitida(ctx, idInterno, "msg-comun")
	if err != nil || permitidoComun {
		t.Fatalf("mensaje común debe permanecer bloqueado tras reapertura: permitido=%v, err=%v", permitidoComun, err)
	}

	// Mensaje de traspaso reclamado debe seguir permitido
	permitidoTraspaso, err := almacen2.SalidaPermitida(ctx, idInterno, "msg-traspaso-durable")
	if err != nil || !permitidoTraspaso {
		t.Fatalf("mensaje de traspaso debe permanecer permitido tras reapertura: permitido=%v, err=%v", permitidoTraspaso, err)
	}

	// Segundo reclamo de traspaso debe fallar (ya reclamado)
	reclamada2, err := almacen2.ReclamarMensajeDeTraspaso(ctx, idInterno, "msg-traspaso-nuevo", 2000)
	if err != nil {
		t.Fatalf("segundo reclamo no debia dar error: %v", err)
	}
	if reclamada2 {
		t.Fatal("segundo reclamo tras reapertura debia ser false")
	}
}

func TestCortacircuitos_AperturaBaseExistenteGanaTablaCortacircuitos(t *testing.T) {
	t.Parallel()
	ruta := filepath.Join(t.TempDir(), "identidad_sin_cortacircuitos.db")
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
	CREATE TABLE IF NOT EXISTS baja_de_contacto (
	  id_interno TEXT PRIMARY KEY REFERENCES identidad(id_interno) ON DELETE CASCADE,
	  dada_de_baja_en_ms INTEGER NOT NULL,
	  id_mensaje_confirmacion TEXT NULL UNIQUE,
	  confirmacion_encolada_en_ms INTEGER NULL
	);
	INSERT INTO identidad (id_interno, estado, creada_en_ms, actualizada_en_ms) VALUES ('ct-existente', 'anclada', 100, 100);
	INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES ('pn', '5491199999999', 's.whatsapp.net', 'ct-existente', 100);
	`
	if _, err := db.Exec(esquemaAntiguo); err != nil {
		t.Fatalf("db.Exec antiguo: %v", err)
	}
	db.Close()

	reg := registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("Abrir sobre base antigua: %v", err)
	}
	defer almacen.Cerrar()

	ctx := context.Background()
	disp, err := almacen.RegistrarObservacion(ctx, "ct-existente", "humano", true, 3, 2000)
	if err != nil || !disp {
		t.Fatalf("RegistrarObservacion en tabla recién creada: disp=%v, err=%v", disp, err)
	}
}
