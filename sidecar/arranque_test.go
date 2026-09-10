package main

import (
	"context"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// entornoFalsoParaArranque construye una función de consulta del entorno a partir de un mapa,
// sin tocar el entorno real del proceso de test. La variable HEXCELL_VENTANA_ZONA es la única
// sin valor por omisión (HEX-066, blueprint paso 5), así que el helper le aplica un fallback
// fijo: el mismo patrón que `configuracion_test.go` ya usa para el resto de los tests del
// paquete de configuración.
func entornoFalsoParaArranque(valores map[string]string) func(string) (string, bool) {
	return func(clave string) (string, bool) {
		valor, presente := valores[clave]
		if !presente && clave == configuracion.VariableVentanaZona {
			return "America/La_Paz", true
		}
		return valor, presente
	}
}

// TestAbrirRecursosDeArranqueContraDirectorioVacioCreaIdentidadAntesDeSuRespaldo ejercita
// la secuencia extraída en arranque.go contra un t.TempDir() recién creado, sin archivos
// preexistentes. Es el criterio de aceptación AC-1 de HEX-066: el primer arranque real de
// cada célula debe crear identidad.db antes de que la conexión de respaldo de solo lectura
// intente abrirla.
//
// La guarda por mutación (AC-2) vive fuera de este archivo: el script de verify invierte
// el orden de los dos marcadores grep-matchables (identidad.Abrir y
// canal.AbrirConexionDeRespaldo(cfg.RutaIdentidad)) y exige que el test se vea fallar.
// Para que esa guarda sea real —y no un falso verde de un awk que solo reubique líneas sin
// invertir el orden efectivo de creación/consumo— el test verifica además que las dos
// conexiones de respaldo devueltas sean utilizables: un Ping sobre cada una. Si el orden
// estuviera invertido, AbrirConexionDeRespaldo(cfg.RutaIdentidad) intentaría modo=ro sobre
// un archivo inexistente y devolvería una conexión nula; Ping sobre una *sql.DB nula hace
// panicar al test, y la guarda queda demostrada por efecto observable del defecto.
func TestAbrirRecursosDeArranqueContraDirectorioVacioCreaIdentidadAntesDeSuRespaldo(t *testing.T) {
	directorio := t.TempDir()

	rutaSqlstore := filepath.Join(directorio, "sqlstore.db")
	rutaIdentidad := filepath.Join(directorio, "identidad.db")
	rutaOutbox := filepath.Join(directorio, "outbox.db")
	rutaSocket := filepath.Join(directorio, "sidecar.sock")

	entorno := map[string]string{
		configuracion.VariableRutaSqlstore:  rutaSqlstore,
		configuracion.VariableRutaIdentidad: rutaIdentidad,
		configuracion.VariableRutaOutbox:    rutaOutbox,
		configuracion.VariableSocket:        rutaSocket,
	}

	cfg, err := configuracion.Cargar(entornoFalsoParaArranque(entorno))
	if err != nil {
		t.Fatalf("configuracion.Cargar: %v", err)
	}

	reg := registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test")

	recursos, err := abrirRecursosDeArranque(context.Background(), cfg, reg)
	if err != nil {
		t.Fatalf("abrirRecursosDeArranque: %v", err)
	}
	if recursos == nil {
		t.Fatalf("abrirRecursosDeArranque devolvió struct nulo")
	}

	// Cierra todo lo devuelto para no dejar descriptores abiertos durante el resto del test;
	// un Ping posterior abriría conexiones nuevas que el cleanup no recoge.
	t.Cleanup(func() {
		if recursos.Buzon != nil {
			_ = recursos.Buzon.Cerrar()
		}
		if recursos.AlmacenIdentidad != nil {
			_ = recursos.AlmacenIdentidad.Cerrar()
		}
		if recursos.DBRespaldoIdentidad != nil {
			_ = recursos.DBRespaldoIdentidad.Close()
		}
		if recursos.DBRespaldo != nil {
			_ = recursos.DBRespaldo.Close()
		}
		if recursos.Contenedor != nil {
			_ = recursos.Contenedor.Close()
		}
	})

	if recursos.DBRespaldo == nil {
		t.Fatalf("DBRespaldo es nulo: la conexión de respaldo de sqlstore no se abrió")
	}
	if err := recursos.DBRespaldo.Ping(); err != nil {
		t.Fatalf("Ping sobre DBRespaldo (sqlstore) falló: %v", err)
	}
	if recursos.DBRespaldoIdentidad == nil {
		// Esta comprobación NO es la que acredita el orden. Con el orden invertido,
		// AbrirConexionDeRespaldo hace su propio Ping contra un archivo inexistente, falla,
		// y abrirRecursosDeArranque propaga ese error: el test muere antes, en la
		// comprobación de err de más arriba, y esta rama queda inalcanzable para ese caso.
		// Lo que sí cubre es lo otro: que un futuro cambio devuelva recursos incompletos
		// SIN error, que es un modo de fallo distinto y silencioso.
		t.Fatalf("DBRespaldoIdentidad es nulo pese a que el arranque no devolvió error: recursos incompletos en silencio")
	}
	if err := recursos.DBRespaldoIdentidad.Ping(); err != nil {
		t.Fatalf("Ping sobre DBRespaldoIdentidad (identidad) falló: %v", err)
	}

	info, err := os.Stat(rutaIdentidad)
	if err != nil {
		t.Fatalf("identidad.db no existe tras el arranque en frío: %v", err)
	}
	if !info.Mode().IsRegular() {
		t.Fatalf("identidad.db existe pero no es un archivo regular: %v", info.Mode())
	}
}