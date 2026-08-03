package outbox

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestConstruirDSN_PragmasRequeridos(t *testing.T) {
	ruta := "/var/lib/hexcell/outbox.db"
	dsn := construirDSN(ruta)

	if !strings.Contains(dsn, "_pragma=journal_mode(WAL)") {
		t.Errorf("el DSN no incluye _pragma=journal_mode(WAL): %s", dsn)
	}
	if !strings.Contains(dsn, "_pragma=synchronous(FULL)") {
		t.Errorf("el DSN no incluye _pragma=synchronous(FULL): %s", dsn)
	}
}

func TestAbrir_PragmaJournalModeEnConexionActiva(t *testing.T) {
	dir := t.TempDir()
	ruta := filepath.Join(dir, "outbox.db")

	ob, err := Abrir(Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error al abrir: %v", err)
	}
	defer ob.Cerrar()

	var journalMode string
	if err := ob.db.QueryRow("PRAGMA journal_mode;").Scan(&journalMode); err != nil {
		t.Fatalf("error al consultar journal_mode: %v", err)
	}
	if journalMode != "wal" {
		t.Errorf("journal_mode esperado 'wal', obtenido '%s'", journalMode)
	}
}
