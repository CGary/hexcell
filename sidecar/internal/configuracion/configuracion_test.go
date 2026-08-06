package configuracion_test

import (
	"errors"
	"log/slog"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
)

// entornoFalso construye una función de consulta del entorno a partir de un mapa, sin tocar el
// entorno real del proceso de test.
func entornoFalso(valores map[string]string) func(string) (string, bool) {
	return func(clave string) (string, bool) {
		valor, presente := valores[clave]
		return valor, presente
	}
}

func TestCargarAplicaLosValoresPorOmisionConEntornoVacio(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error con el entorno vacío: %v", err)
	}
	if cfg.RutaSocket != configuracion.RutaSocketPorOmision {
		t.Errorf("ruta del socket = %q, se esperaba %q", cfg.RutaSocket, configuracion.RutaSocketPorOmision)
	}
	if cfg.NivelDeRegistro != slog.LevelInfo {
		t.Errorf("nivel = %v, se esperaba info", cfg.NivelDeRegistro)
	}
	if cfg.IdCelula != configuracion.IdCelulaPorOmision {
		t.Errorf("id de célula = %q, se esperaba %q", cfg.IdCelula, configuracion.IdCelulaPorOmision)
	}
}

func TestCargarLeeLosTresParametrosDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableSocket:          "/tmp/celula/ipc.sock",
		configuracion.VariableNivelDeRegistro: "aviso",
		configuracion.VariableIdCelula:        "piloto-01",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.RutaSocket != "/tmp/celula/ipc.sock" {
		t.Errorf("ruta del socket = %q", cfg.RutaSocket)
	}
	if cfg.NivelDeRegistro != slog.LevelWarn {
		t.Errorf("nivel = %v, se esperaba aviso", cfg.NivelDeRegistro)
	}
	if cfg.IdCelula != "piloto-01" {
		t.Errorf("id de célula = %q", cfg.IdCelula)
	}
}

func TestCargarRechazaUnaRutaDeSocketVacia(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableSocket: "",
	}))
	if !errors.Is(err, configuracion.ErrRutaSocketVacia) {
		t.Fatalf("error = %v, se esperaba ErrRutaSocketVacia", err)
	}
}

func TestCargarRechazaUnNivelDeRegistroDesconocido(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableNivelDeRegistro: "verboso",
	}))
	if !errors.Is(err, configuracion.ErrNivelDeRegistroDesconocido) {
		t.Fatalf("error = %v, se esperaba ErrNivelDeRegistroDesconocido", err)
	}
}

func TestCargarLeeRutaSqlstoreYTelefonoCelula(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRutaSqlstore:   "/tmp/celula/sqlstore.db",
		configuracion.VariableTelefonoCelula: "5491155551234",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.RutaSqlstore != "/tmp/celula/sqlstore.db" {
		t.Errorf("ruta del sqlstore = %q", cfg.RutaSqlstore)
	}
	if cfg.TelefonoCelula != "5491155551234" {
		t.Errorf("teléfono de la célula = %q", cfg.TelefonoCelula)
	}
}

func TestCargarAplicaValorPorOmisionDelSqlstore(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.RutaSqlstore != configuracion.RutaSqlstorePorOmision {
		t.Errorf("ruta del sqlstore = %q, se esperaba %q", cfg.RutaSqlstore, configuracion.RutaSqlstorePorOmision)
	}
	if cfg.TelefonoCelula != "" {
		t.Errorf("teléfono de la célula = %q, se esperaba vacío", cfg.TelefonoCelula)
	}
}

func TestCargarRechazaUnaRutaDeSqlstoreVacia(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRutaSqlstore: "",
	}))
	if !errors.Is(err, configuracion.ErrRutaSqlstoreVacia) {
		t.Fatalf("error = %v, se esperaba ErrRutaSqlstoreVacia", err)
	}
}

func TestCargarAplicaValoresPorOmisionDelRetroceso(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.Retroceso.IntervaloInicial != configuracion.RetrocesoInicialMsPorOmision {
		t.Errorf("IntervaloInicial = %d, se esperaba %d", cfg.Retroceso.IntervaloInicial, configuracion.RetrocesoInicialMsPorOmision)
	}
	if cfg.Retroceso.Factor != configuracion.RetrocesoFactorPorOmision {
		t.Errorf("Factor = %d, se esperaba %d", cfg.Retroceso.Factor, configuracion.RetrocesoFactorPorOmision)
	}
	if cfg.Retroceso.IntervaloMaximo != configuracion.RetrocesoMaximoMsPorOmision {
		t.Errorf("IntervaloMaximo = %d, se esperaba %d", cfg.Retroceso.IntervaloMaximo, configuracion.RetrocesoMaximoMsPorOmision)
	}
	if cfg.Retroceso.BaneoInicial != configuracion.RetrocesoBaneoInicialMsPorOmision {
		t.Errorf("BaneoInicial = %d, se esperaba %d", cfg.Retroceso.BaneoInicial, configuracion.RetrocesoBaneoInicialMsPorOmision)
	}
	if cfg.Retroceso.BaneoMaximo != configuracion.RetrocesoBaneoMaximoMsPorOmision {
		t.Errorf("BaneoMaximo = %d, se esperaba %d", cfg.Retroceso.BaneoMaximo, configuracion.RetrocesoBaneoMaximoMsPorOmision)
	}
}

func TestCargarLeeRetrocesoDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRetrocesoInicialMs:      "500",
		configuracion.VariableRetrocesoFactor:         "3",
		configuracion.VariableRetrocesoMaximoMs:       "30000",
		configuracion.VariableRetrocesoBaneoInicialMs: "10000",
		configuracion.VariableRetrocesoBaneoMaximoMs:  "120000",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.Retroceso.IntervaloInicial != 500 {
		t.Errorf("IntervaloInicial = %d", cfg.Retroceso.IntervaloInicial)
	}
	if cfg.Retroceso.Factor != 3 {
		t.Errorf("Factor = %d", cfg.Retroceso.Factor)
	}
	if cfg.Retroceso.IntervaloMaximo != 30000 {
		t.Errorf("IntervaloMaximo = %d", cfg.Retroceso.IntervaloMaximo)
	}
	if cfg.Retroceso.BaneoInicial != 10000 {
		t.Errorf("BaneoInicial = %d", cfg.Retroceso.BaneoInicial)
	}
	if cfg.Retroceso.BaneoMaximo != 120000 {
		t.Errorf("BaneoMaximo = %d", cfg.Retroceso.BaneoMaximo)
	}
}

func TestCargarRechazaRetrocesoNoNumerico(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRetrocesoInicialMs: "no-un-entero",
	}))
	if !errors.Is(err, configuracion.ErrRetrocesoInvalido) {
		t.Fatalf("error = %v, se esperaba ErrRetrocesoInvalido", err)
	}
}

func TestCargarRechazaRetrocesoCero(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRetrocesoInicialMs: "0",
	}))
	if !errors.Is(err, configuracion.ErrRetrocesoInvalido) {
		t.Fatalf("error = %v, se esperaba ErrRetrocesoInvalido", err)
	}
}

func TestCargarRechazaTechoMenorQueIntervaloInicial(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRetrocesoInicialMs: "5000",
		configuracion.VariableRetrocesoMaximoMs:  "1000",
	}))
	if !errors.Is(err, configuracion.ErrRetrocesoInvalido) {
		t.Fatalf("error = %v, se esperaba ErrRetrocesoInvalido", err)
	}
}

func TestCargarRechazaTechoDeBaneoMenorQueInicialDeBaneo(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRetrocesoBaneoInicialMs: "60000",
		configuracion.VariableRetrocesoBaneoMaximoMs:  "10000",
	}))
	if !errors.Is(err, configuracion.ErrRetrocesoInvalido) {
		t.Fatalf("error = %v, se esperaba ErrRetrocesoInvalido", err)
	}
}
