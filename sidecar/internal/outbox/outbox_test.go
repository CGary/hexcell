package outbox_test

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"path/filepath"
	"testing"
	"time"

	_ "modernc.org/sqlite"

	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

func TestAbrir_CreacionEIdempotencia(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error inesperado al abrir: %v", err)
	}
	if err := ob.Cerrar(); err != nil {
		t.Fatalf("error inesperado al cerrar: %v", err)
	}

	// Reabrir en la misma ruta debe ser idempotente
	ob2, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error inesperado al reabrir: %v", err)
	}
	defer ob2.Cerrar()
}

func TestAbrir_DirectorioInvalido(t *testing.T) {
	ruta := "/directorio_inexistente_xyz_123/outbox.db"
	_, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err == nil {
		t.Fatal("se esperaba un error al abrir en un directorio inexistente")
	}
}

func TestPragmas_WALYSynchronousFull(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	db, err := sql.Open("sqlite", fmt.Sprintf("file:%s", ruta))
	if err != nil {
		t.Fatalf("error al abrir con sql directo: %v", err)
	}
	defer db.Close()

	var journalMode string
	if err := db.QueryRow("PRAGMA journal_mode;").Scan(&journalMode); err != nil {
		t.Fatalf("error al consultar journal_mode: %v", err)
	}
	if journalMode != "wal" {
		t.Errorf("journal_mode esperado 'wal', obtenido '%s'", journalMode)
	}

	var synchronous int
	if err := db.QueryRow("PRAGMA synchronous;").Scan(&synchronous); err != nil {
		t.Fatalf("error al consultar synchronous: %v", err)
	}
	// 2 equivale a FULL en SQLite
	if synchronous != 2 {
		t.Errorf("synchronous esperado 2 (FULL), obtenido %d", synchronous)
	}
}

func TestPersistir_ExitosoYDurabilidad(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	id := "evt-001"
	carga := []byte("payload de prueba")
	if err := ob.Persistir(ctx, id, carga); err != nil {
		t.Fatalf("error al persistir: %v", err)
	}

	// Verificar durabilidad con una conexión independiente
	dbRaw, err := sql.Open("sqlite", fmt.Sprintf("file:%s", ruta))
	if err != nil {
		t.Fatalf("error al abrir conexión cruda: %v", err)
	}
	defer dbRaw.Close()

	var idLeido string
	var cargaLeida []byte
	err = dbRaw.QueryRow("SELECT id_deduplicacion, carga FROM outbox WHERE id_deduplicacion = ?", id).Scan(&idLeido, &cargaLeida)
	if err != nil {
		t.Fatalf("no se encontró el registro persisido en conexión cruda: %v", err)
	}
	if idLeido != id || string(cargaLeida) != string(carga) {
		t.Errorf("datos persisidos no coinciden")
	}
}

func TestPersistir_IdDeduplicacionVacio(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	err = ob.Persistir(ctx, "", []byte("test"))
	if !errors.Is(err, outbox.ErrIdDeduplicacionVacio) {
		t.Errorf("se esperaba ErrIdDeduplicacionVacio, obtenido: %v", err)
	}
}

func TestPersistir_IdempotenciaDuplicada(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	id := "evt-duplicado"
	cargaOriginal := []byte("original")
	cargaNueva := []byte("nueva")

	if err := ob.Persistir(ctx, id, cargaOriginal); err != nil {
		t.Fatalf("error al persistir original: %v", err)
	}

	if err := ob.Persistir(ctx, id, cargaNueva); err != nil {
		t.Fatalf("error al persistir duplicado: %v", err)
	}

	pendientes, err := ob.Pendientes(ctx)
	if err != nil {
		t.Fatalf("error al consultar pendientes: %v", err)
	}
	if len(pendientes) != 1 {
		t.Fatalf("se esperaba 1 pendiente, obtenidas %d", len(pendientes))
	}
	if string(pendientes[0].Carga) != string(cargaOriginal) {
		t.Errorf("la carga debió mantenerse intacta: %s", string(pendientes[0].Carga))
	}
}

func TestConfirmar_Exitoso(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-1", []byte("1"))
	_ = ob.Persistir(ctx, "evt-2", []byte("2"))

	confirmado, err := ob.Confirmar(ctx, "evt-1")
	if err != nil {
		t.Fatalf("error al confirmar: %v", err)
	}
	if !confirmado {
		t.Error("se esperaba que la confirmación marcara 1 fila")
	}

	pendientes, err := ob.Pendientes(ctx)
	if err != nil {
		t.Fatalf("error al obtener pendientes: %v", err)
	}
	if len(pendientes) != 1 || pendientes[0].IdDeduplicacion != "evt-2" {
		t.Errorf("pendientes no coincide después de confirmar: %+v", pendientes)
	}
}

func TestConfirmar_InexistenteOVacio(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	confirmado, err := ob.Confirmar(ctx, "no-existe")
	if err != nil || confirmado {
		t.Errorf("se esperaba false sin error para id inexistente, obtenido: %v, %v", confirmado, err)
	}

	confirmadoVacio, err := ob.Confirmar(ctx, "")
	if err != nil || confirmadoVacio {
		t.Errorf("se esperaba false sin error para id vacío, obtenido: %v, %v", confirmadoVacio, err)
	}
}

func TestConfirmar_Idempotente(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-1", []byte("payload"))

	c1, err := ob.Confirmar(ctx, "evt-1")
	if err != nil || !c1 {
		t.Fatalf("primera confirmación falló")
	}

	c2, err := ob.Confirmar(ctx, "evt-1")
	if err != nil || c2 {
		t.Errorf("segunda confirmación debió devolver false, obtenido: %v, %v", c2, err)
	}
}

func TestPendientes_IdempotenteYEsLecturaPura(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-1", []byte("1"))
	_ = ob.Persistir(ctx, "evt-2", []byte("2"))

	p1, _ := ob.Pendientes(ctx)
	p2, _ := ob.Pendientes(ctx)

	if len(p1) != len(p2) || len(p1) != 2 {
		t.Fatalf("lecturas subsecuentes de pendientes difieren en tamaño")
	}
	for i := range p1 {
		if p1[i].IdDeduplicacion != p2[i].IdDeduplicacion {
			t.Errorf("las entradas difieren en la posición %d", i)
		}
	}
}

func TestReinicio_RedeliveryEnOrdenOriginal(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}

	_ = ob.Persistir(ctx, "evt-1", []byte("1"))
	_ = ob.Persistir(ctx, "evt-2", []byte("2"))
	_ = ob.Persistir(ctx, "evt-3", []byte("3"))
	_, _ = ob.Confirmar(ctx, "evt-2")

	_ = ob.Cerrar()

	// Simular reinicio abriendo el buzón nuevamente
	obReabierto, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al reabrir tras reinicio: %v", err)
	}
	defer obReabierto.Cerrar()

	pendientes, err := obReabierto.Pendientes(ctx)
	if err != nil {
		t.Fatalf("error al obtener pendientes tras reinicio: %v", err)
	}

	if len(pendientes) != 2 {
		t.Fatalf("se esperaban 2 pendientes tras reinicio, obtenidas %d", len(pendientes))
	}
	if pendientes[0].IdDeduplicacion != "evt-1" || pendientes[1].IdDeduplicacion != "evt-3" {
		t.Errorf("orden o contenido de redelivery incorrecto: %+v", pendientes)
	}
}

func TestPurgar_RespetoDeVentanaYPreservacionSinConfirmar(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()
	retencion := 1 * time.Hour

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta, Retencion: retencion})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-antiguo-confirmado", []byte("1"))
	_ = ob.Persistir(ctx, "evt-reciente-confirmado", []byte("2"))
	_ = ob.Persistir(ctx, "evt-sin-confirmar", []byte("3"))

	ahora := time.Now()

	// Confirmar ambos
	_, _ = ob.Confirmar(ctx, "evt-antiguo-confirmado")
	_, _ = ob.Confirmar(ctx, "evt-reciente-confirmado")

	// Modificar manualmente el timestamp de confirmación del antiguo a 2 horas en el pasado
	dbRaw, err := sql.Open("sqlite", fmt.Sprintf("file:%s", ruta))
	if err != nil {
		t.Fatalf("error abrir sql raw: %v", err)
	}
	defer dbRaw.Close()

	hace2HorasMs := ahora.Add(-2 * time.Hour).UnixMilli()
	_, err = dbRaw.Exec("UPDATE outbox SET confirmado_en_ms = ? WHERE id_deduplicacion = ?", hace2HorasMs, "evt-antiguo-confirmado")
	if err != nil {
		t.Fatalf("error al actualizar timestamp en db raw: %v", err)
	}

	purgadas, err := ob.Purgar(ctx, ahora)
	if err != nil {
		t.Fatalf("error al purgar: %v", err)
	}
	if purgadas != 1 {
		t.Errorf("se esperaba 1 fila purgada, obtenidas %d", purgadas)
	}

	// Verificar sobrevivientes
	var total int
	err = dbRaw.QueryRow("SELECT COUNT(*) FROM outbox").Scan(&total)
	if err != nil {
		t.Fatalf("error al contar filas restantes: %v", err)
	}
	if total != 2 {
		t.Errorf("se esperaban 2 filas restantes en la db, obtenidas %d", total)
	}
}

func TestPurgar_RetencionPorOmisionSinCambios(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-reciente", []byte("1"))
	_, _ = ob.Confirmar(ctx, "evt-reciente")

	purgadas, err := ob.Purgar(ctx, time.Now())
	if err != nil {
		t.Fatalf("error al purgar: %v", err)
	}
	if purgadas != 0 {
		t.Errorf("no se debió purgar ningún evento reciente, purgadas: %d", purgadas)
	}
}

func TestSecuenciaMonotonica_ConIntercalado(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-1", []byte("1"))
	_ = ob.Persistir(ctx, "evt-2", []byte("2"))

	_, _ = ob.Confirmar(ctx, "evt-1")
	_, _ = ob.Purgar(ctx, time.Now().Add(10*time.Hour))

	_ = ob.Persistir(ctx, "evt-3", []byte("3"))

	pendientes, err := ob.Pendientes(ctx)
	if err != nil {
		t.Fatalf("error al consultar pendientes: %v", err)
	}
	if len(pendientes) != 2 {
		t.Fatalf("se esperaban 2 pendientes, obtenidas %d", len(pendientes))
	}
	if pendientes[0].IdDeduplicacion != "evt-2" || pendientes[1].IdDeduplicacion != "evt-3" {
		t.Errorf("orden secuencial no preservado tras purga: %+v", pendientes)
	}
	if pendientes[0].Secuencia >= pendientes[1].Secuencia {
		t.Errorf("secuencia debe ser estrictamente creciente: %d >= %d", pendientes[0].Secuencia, pendientes[1].Secuencia)
	}
}

func TestOperacionesEnBuzonCerrado(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}

	if err := ob.Cerrar(); err != nil {
		t.Fatalf("error al cerrar: %v", err)
	}

	if err := ob.Persistir(ctx, "evt-1", []byte("1")); !errors.Is(err, outbox.ErrOutboxCerrado) {
		t.Errorf("Persistir en cerrado debió devolver ErrOutboxCerrado, obtenido: %v", err)
	}

	if _, err := ob.Pendientes(ctx); !errors.Is(err, outbox.ErrOutboxCerrado) {
		t.Errorf("Pendientes en cerrado debió devolver ErrOutboxCerrado, obtenido: %v", err)
	}

	if _, err := ob.Confirmar(ctx, "evt-1"); !errors.Is(err, outbox.ErrOutboxCerrado) {
		t.Errorf("Confirmar en cerrado debió devolver ErrOutboxCerrado, obtenido: %v", err)
	}

	if _, err := ob.Purgar(ctx, time.Now()); !errors.Is(err, outbox.ErrOutboxCerrado) {
		t.Errorf("Purgar en cerrado debió devolver ErrOutboxCerrado, obtenido: %v", err)
	}

	if err := ob.Cerrar(); err != nil {
		t.Errorf("segundo Cerrar debió ser nil, obtenido: %v", err)
	}
}

func TestRegistroEstructurado(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")
	ctx := context.Background()

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")

	ob, err := outbox.Abrir(outbox.Opciones{Ruta: ruta, Registro: reg})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	_ = ob.Persistir(ctx, "evt-reg", []byte("secreto"))
	_, _ = ob.Confirmar(ctx, "evt-reg")
	_, _ = ob.Purgar(ctx, time.Now().Add(10*24*time.Hour))

	salida := buf.String()
	if !bytes.Contains(buf.Bytes(), []byte(outbox.EventoPersistido)) {
		t.Errorf("registro no contiene EventoPersistido: %s", salida)
	}
	if !bytes.Contains(buf.Bytes(), []byte(outbox.EventoConfirmado)) {
		t.Errorf("registro no contiene EventoConfirmado: %s", salida)
	}
	if !bytes.Contains(buf.Bytes(), []byte(outbox.EventoPurgado)) {
		t.Errorf("registro no contiene EventoPurgado: %s", salida)
	}
	if bytes.Contains(buf.Bytes(), []byte("secreto")) {
		t.Errorf("EL REGISTRO VIOLÓ LA PRIVACIDAD E INCLUYÓ LA CARGA: %s", salida)
	}
}
