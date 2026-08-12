package configuracion_test

import (
	"errors"
	"log/slog"
	"reflect"
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

func TestCargar_RutaIdentidadPorOmision(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.RutaIdentidad != configuracion.RutaIdentidadPorOmision {
		t.Errorf("ruta de identidad = %q, se esperaba %q", cfg.RutaIdentidad, configuracion.RutaIdentidadPorOmision)
	}
}

func TestCargar_RutaIdentidadPersonalizada(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRutaIdentidad: "/tmp/celula/identidad.db",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.RutaIdentidad != "/tmp/celula/identidad.db" {
		t.Errorf("ruta de identidad = %q, se esperaba %q", cfg.RutaIdentidad, "/tmp/celula/identidad.db")
	}
}

func TestCargar_RutaIdentidadVacia(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableRutaIdentidad: "",
	}))
	if !errors.Is(err, configuracion.ErrRutaIdentidadVacia) {
		t.Fatalf("error = %v, se esperaba ErrRutaIdentidadVacia", err)
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

func TestCargarAplicaValoresPorOmisionDeSalida(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.TtlSalidaMs != configuracion.TtlSalidaMsPorOmision {
		t.Errorf("TtlSalidaMs = %d, se esperaba %d", cfg.TtlSalidaMs, configuracion.TtlSalidaMsPorOmision)
	}
	if cfg.IntentosMaximosSalida != configuracion.IntentosMaximosSalidaPorOmision {
		t.Errorf("IntentosMaximosSalida = %d, se esperaba %d", cfg.IntentosMaximosSalida, configuracion.IntentosMaximosSalidaPorOmision)
	}
}

func TestCargarLeeParametrosDeSalidaDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableTtlSalidaMs:           "60000",
		configuracion.VariableIntentosMaximosSalida: "5",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if cfg.TtlSalidaMs != 60000 {
		t.Errorf("TtlSalidaMs = %d, se esperaba 60000", cfg.TtlSalidaMs)
	}
	if cfg.IntentosMaximosSalida != 5 {
		t.Errorf("IntentosMaximosSalida = %d, se esperaba 5", cfg.IntentosMaximosSalida)
	}
}

func TestCargarRechazaParametrosDeSalidaInvalido(t *testing.T) {
	t.Parallel()

	_, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableTtlSalidaMs: "-1",
	}))
	if !errors.Is(err, configuracion.ErrParametroSalidaInvalido) {
		t.Errorf("error = %v, se esperaba ErrParametroSalidaInvalido", err)
	}

	_, err = configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableIntentosMaximosSalida: "0",
	}))
	if !errors.Is(err, configuracion.ErrParametroSalidaInvalido) {
		t.Errorf("error = %v, se esperaba ErrParametroSalidaInvalido", err)
	}
}

func TestCargarAplicaValoresPorOmisionDeBaja(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if len(cfg.PalabrasDeBaja) != 2 || cfg.PalabrasDeBaja[0] != "baja" || cfg.PalabrasDeBaja[1] != "stop" {
		t.Errorf("PalabrasDeBaja = %v, se esperaba [baja, stop]", cfg.PalabrasDeBaja)
	}
	if cfg.TextoConfirmacionDeBaja != configuracion.TextoConfirmacionDeBajaPorOmision {
		t.Errorf("TextoConfirmacionDeBaja = %q, se esperaba %q", cfg.TextoConfirmacionDeBaja, configuracion.TextoConfirmacionDeBajaPorOmision)
	}
}

func TestCargarLeeParametrosDeBajaDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariablePalabrasDeBaja:          "stop, cancel, salir",
		configuracion.VariableTextoConfirmacionDeBaja: "Confirmación personalizada",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}
	if len(cfg.PalabrasDeBaja) != 3 || cfg.PalabrasDeBaja[0] != "stop" || cfg.PalabrasDeBaja[1] != "cancel" || cfg.PalabrasDeBaja[2] != "salir" {
		t.Errorf("PalabrasDeBaja = %v", cfg.PalabrasDeBaja)
	}
	if cfg.TextoConfirmacionDeBaja != "Confirmación personalizada" {
		t.Errorf("TextoConfirmacionDeBaja = %q", cfg.TextoConfirmacionDeBaja)
	}
}

func TestCargarRechazaParametrosDeBajaInvalidos(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre  string
		entorno map[string]string
	}{
		{"palabras_vacia", map[string]string{configuracion.VariablePalabrasDeBaja: ""}},
		{"palabras_solo_espacios", map[string]string{configuracion.VariablePalabrasDeBaja: "   "}},
		{"palabras_solo_comas", map[string]string{configuracion.VariablePalabrasDeBaja: " , , "}},
		{"texto_vacio", map[string]string{configuracion.VariableTextoConfirmacionDeBaja: ""}},
		{"texto_solo_espacios", map[string]string{configuracion.VariableTextoConfirmacionDeBaja: "   "}},
	}

	for _, c := range casos {
		t.Run(c.nombre, func(t *testing.T) {
			_, err := configuracion.Cargar(entornoFalso(c.entorno))
			if !errors.Is(err, configuracion.ErrParametroDeBajaInvalido) {
				t.Errorf("caso %s: se esperaba ErrParametroDeBajaInvalido, se obtuvo %v", c.nombre, err)
			}
		})
	}
}

func TestCargarAplicaValoresPorOmisionDeDisciplina(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	d := cfg.Disciplina
	if d.LatenciaMinimaMs != configuracion.LatenciaMinimaMsPorOmision {
		t.Errorf("LatenciaMinimaMs = %d, se esperaba %d", d.LatenciaMinimaMs, configuracion.LatenciaMinimaMsPorOmision)
	}
	if d.IntervaloDrenajeMs != configuracion.IntervaloDrenajeMsPorOmision {
		t.Errorf("IntervaloDrenajeMs = %d, se esperaba %d", d.IntervaloDrenajeMs, configuracion.IntervaloDrenajeMsPorOmision)
	}
	if d.Ventana.HoraApertura != 9 || d.Ventana.MinutoApertura != 0 {
		t.Errorf("Ventana Apertura = %02d:%02d, se esperaba 09:00", d.Ventana.HoraApertura, d.Ventana.MinutoApertura)
	}
	if d.Ventana.HoraCierre != 19 || d.Ventana.MinutoCierre != 0 {
		t.Errorf("Ventana Cierre = %02d:%02d, se esperaba 19:00", d.Ventana.HoraCierre, d.Ventana.MinutoCierre)
	}
	if len(d.Ventana.Dias) != 5 || d.Ventana.Dias[0] != 1 || d.Ventana.Dias[4] != 5 {
		t.Errorf("Ventana Dias = %v, se esperaba [1,2,3,4,5]", d.Ventana.Dias)
	}
	if d.Ventana.Zona == nil || d.Ventana.Zona.String() != configuracion.VentanaZonaPorOmision {
		t.Errorf("Ventana Zona = %v, se esperaba %s", d.Ventana.Zona, configuracion.VentanaZonaPorOmision)
	}
	if d.Rampa.DiariaInicial != configuracion.RampaDiariaInicialPorOmision {
		t.Errorf("Rampa DiariaInicial = %d, se esperaba %d", d.Rampa.DiariaInicial, configuracion.RampaDiariaInicialPorOmision)
	}
	if d.Rampa.IncrementoSemanal != configuracion.RampaIncrementoSemanalPorOmision {
		t.Errorf("Rampa IncrementoSemanal = %d, se esperaba %d", d.Rampa.IncrementoSemanal, configuracion.RampaIncrementoSemanalPorOmision)
	}
	if d.Rampa.Semanas != configuracion.RampaSemanasPorOmision {
		t.Errorf("Rampa Semanas = %d, se esperaba %d", d.Rampa.Semanas, configuracion.RampaSemanasPorOmision)
	}
}

func TestCargarLeeParametrosDeDisciplinaDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableLatenciaMinimaMs:       "5000",
		configuracion.VariableIntervaloDrenajeMs:     "1000",
		configuracion.VariableVentanaApertura:        "08:30",
		configuracion.VariableVentanaCierre:          "18:00",
		configuracion.VariableVentanaDias:            "1, 2, 3, 4, 5, 6",
		configuracion.VariableVentanaZona:            "America/Sao_Paulo",
		configuracion.VariableRampaDiariaInicial:     "30",
		configuracion.VariableRampaIncrementoSemanal: "15",
		configuracion.VariableRampaSemanas:           "6",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	d := cfg.Disciplina
	if d.LatenciaMinimaMs != 5000 {
		t.Errorf("LatenciaMinimaMs = %d, se esperaba 5000", d.LatenciaMinimaMs)
	}
	if d.IntervaloDrenajeMs != 1000 {
		t.Errorf("IntervaloDrenajeMs = %d, se esperaba 1000", d.IntervaloDrenajeMs)
	}
	if d.Ventana.HoraApertura != 8 || d.Ventana.MinutoApertura != 30 {
		t.Errorf("Ventana Apertura = %02d:%02d, se esperaba 08:30", d.Ventana.HoraApertura, d.Ventana.MinutoApertura)
	}
	if d.Ventana.HoraCierre != 18 || d.Ventana.MinutoCierre != 0 {
		t.Errorf("Ventana Cierre = %02d:%02d, se esperaba 18:00", d.Ventana.HoraCierre, d.Ventana.MinutoCierre)
	}
	if len(d.Ventana.Dias) != 6 || d.Ventana.Dias[5] != 6 {
		t.Errorf("Ventana Dias = %v", d.Ventana.Dias)
	}
	if d.Ventana.Zona == nil || d.Ventana.Zona.String() != "America/Sao_Paulo" {
		t.Errorf("Ventana Zona = %v", d.Ventana.Zona)
	}
	if d.Rampa.DiariaInicial != 30 {
		t.Errorf("Rampa DiariaInicial = %d, se esperaba 30", d.Rampa.DiariaInicial)
	}
	if d.Rampa.IncrementoSemanal != 15 {
		t.Errorf("Rampa IncrementoSemanal = %d, se esperaba 15", d.Rampa.IncrementoSemanal)
	}
	if d.Rampa.Semanas != 6 {
		t.Errorf("Rampa Semanas = %d, se esperaba 6", d.Rampa.Semanas)
	}
}

func TestCargarRechazaParametrosDeDisciplinaDegeneradosOInvalidos(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre  string
		entorno map[string]string
	}{
		{"latencia_cero", map[string]string{configuracion.VariableLatenciaMinimaMs: "0"}},
		{"latencia_negativa", map[string]string{configuracion.VariableLatenciaMinimaMs: "-100"}},
		{"latencia_no_numerica", map[string]string{configuracion.VariableLatenciaMinimaMs: "invalido"}},
		{"latencia_excede_techo", map[string]string{configuracion.VariableLatenciaMinimaMs: "300001"}},
		{"intervalo_drenaje_cero", map[string]string{configuracion.VariableIntervaloDrenajeMs: "0"}},
		{"intervalo_drenaje_negativo", map[string]string{configuracion.VariableIntervaloDrenajeMs: "-1"}},
		{"intervalo_drenaje_excede_techo", map[string]string{configuracion.VariableIntervaloDrenajeMs: "60001"}},
		{"apertura_vacia", map[string]string{configuracion.VariableVentanaApertura: ""}},
		{"apertura_invalida", map[string]string{configuracion.VariableVentanaApertura: "25:00"}},
		{"cierre_vacio", map[string]string{configuracion.VariableVentanaCierre: ""}},
		{"cierre_invalido", map[string]string{configuracion.VariableVentanaCierre: "09:65"}},
		{"cierre_anterior_a_apertura", map[string]string{configuracion.VariableVentanaApertura: "19:00", configuracion.VariableVentanaCierre: "09:00"}},
		{"cierre_igual_a_apertura", map[string]string{configuracion.VariableVentanaApertura: "10:00", configuracion.VariableVentanaCierre: "10:00"}},
		{"anti_24x7_duracion_mayor_a_16h", map[string]string{configuracion.VariableVentanaApertura: "06:00", configuracion.VariableVentanaCierre: "23:00"}},
		{"dias_vacio", map[string]string{configuracion.VariableVentanaDias: ""}},
		{"dias_invalido_cero", map[string]string{configuracion.VariableVentanaDias: "0,1,2"}},
		{"dias_invalido_ocho", map[string]string{configuracion.VariableVentanaDias: "1,2,8"}},
		{"zona_vacia", map[string]string{configuracion.VariableVentanaZona: ""}},
		{"zona_desconocida", map[string]string{configuracion.VariableVentanaZona: "Planeta/Marte"}},
		{"rampa_inicial_cero", map[string]string{configuracion.VariableRampaDiariaInicial: "0"}},
		{"rampa_inicial_excede_techo", map[string]string{configuracion.VariableRampaDiariaInicial: "10001"}},
		{"rampa_incremento_cero", map[string]string{configuracion.VariableRampaIncrementoSemanal: "0"}},
		{"rampa_incremento_excede_techo", map[string]string{configuracion.VariableRampaIncrementoSemanal: "10001"}},
		{"rampa_semanas_cero", map[string]string{configuracion.VariableRampaSemanas: "0"}},
		{"rampa_semanas_excede_techo", map[string]string{configuracion.VariableRampaSemanas: "53"}},
	}

	for _, c := range casos {
		t.Run(c.nombre, func(t *testing.T) {
			_, err := configuracion.Cargar(entornoFalso(c.entorno))
			if !errors.Is(err, configuracion.ErrParametroDeDisciplinaInvalido) {
				t.Errorf("caso %s: se esperaba ErrParametroDeDisciplinaInvalido, se obtuvo %v", c.nombre, err)
			}
		})
	}
}

func TestCargarAplicaValoresPorOmisionDeCortacircuitos(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	c := cfg.Cortacircuitos
	if c.UmbralRepeticion != configuracion.CortacircuitosUmbralRepeticionPorOmision {
		t.Errorf("UmbralRepeticion = %d, se esperaba %d", c.UmbralRepeticion, configuracion.CortacircuitosUmbralRepeticionPorOmision)
	}
	if len(c.PalabrasFrustracion) != 4 || c.PalabrasFrustracion[0] != "humano" || c.PalabrasFrustracion[1] != "persona" || c.PalabrasFrustracion[2] != "agente" || c.PalabrasFrustracion[3] != "operador" {
		t.Errorf("PalabrasFrustracion = %v, se esperaba [humano, persona, agente, operador]", c.PalabrasFrustracion)
	}
	if c.TextoTraspaso != configuracion.CortacircuitosTextoTraspasoPorOmision {
		t.Errorf("TextoTraspaso = %q, se esperaba %q", c.TextoTraspaso, configuracion.CortacircuitosTextoTraspasoPorOmision)
	}
}

func TestCargarLeeParametrosDeCortacircuitosDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableCortacircuitosUmbralRepeticion:    "5",
		configuracion.VariableCortacircuitosPalabrasFrustracion: "persona, operador",
		configuracion.VariableCortacircuitosTextoTraspaso:       "Traspaso personalizado a humano.",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	c := cfg.Cortacircuitos
	if c.UmbralRepeticion != 5 {
		t.Errorf("UmbralRepeticion = %d, se esperaba 5", c.UmbralRepeticion)
	}
	if len(c.PalabrasFrustracion) != 2 || c.PalabrasFrustracion[0] != "persona" || c.PalabrasFrustracion[1] != "operador" {
		t.Errorf("PalabrasFrustracion = %v", c.PalabrasFrustracion)
	}
	if c.TextoTraspaso != "Traspaso personalizado a humano." {
		t.Errorf("TextoTraspaso = %q", c.TextoTraspaso)
	}
}

func TestCargarRechazaParametrosDeCortacircuitosInvalidos(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre  string
		entorno map[string]string
	}{
		{"umbral_cero", map[string]string{configuracion.VariableCortacircuitosUmbralRepeticion: "0"}},
		{"umbral_negativo", map[string]string{configuracion.VariableCortacircuitosUmbralRepeticion: "-1"}},
		{"umbral_no_numerico", map[string]string{configuracion.VariableCortacircuitosUmbralRepeticion: "tres"}},
		{"umbral_excede_techo", map[string]string{configuracion.VariableCortacircuitosUmbralRepeticion: "101"}},
		{"palabras_vacia", map[string]string{configuracion.VariableCortacircuitosPalabrasFrustracion: ""}},
		{"palabras_solo_espacios", map[string]string{configuracion.VariableCortacircuitosPalabrasFrustracion: "   "}},
		{"palabras_solo_comas", map[string]string{configuracion.VariableCortacircuitosPalabrasFrustracion: " , , "}},
		{"texto_vacio", map[string]string{configuracion.VariableCortacircuitosTextoTraspaso: ""}},
	}

	for _, c := range casos {
		t.Run(c.nombre, func(t *testing.T) {
			_, err := configuracion.Cargar(entornoFalso(c.entorno))
			if !errors.Is(err, configuracion.ErrParametroDeDisciplinaInvalido) {
				t.Errorf("caso %s: se esperaba ErrParametroDeDisciplinaInvalido, se obtuvo %v", c.nombre, err)
			}
		})
	}
}

func TestCargarAplicaValoresPorOmisionDePresentacion(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	p := cfg.Presentacion
	if p.TextoIdentificacion != configuracion.TextoIdentificacionPorOmision {
		t.Errorf("TextoIdentificacion = %q, se esperaba %q", p.TextoIdentificacion, configuracion.TextoIdentificacionPorOmision)
	}
	if len(p.Variantes) != 3 {
		t.Fatalf("se esperaban 3 variantes por omisión, se obtuvieron %d: %v", len(p.Variantes), p.Variantes)
	}
	if p.Variantes[0] != "¡Hola! Gracias por escribir." || p.Variantes[1] != "Hola, ¿en qué te puedo ayudar?" || p.Variantes[2] != "Buenas, gracias por tu mensaje." {
		t.Errorf("variantes por omisión incorrectas: %v", p.Variantes)
	}
}

func TestCargarLeeParametrosDePresentacionDelEntorno(t *testing.T) {
	t.Parallel()

	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariableTextoIdentificacion:    "Soy un bot. Escribí agente para humano.",
		configuracion.VariablePlantillasPresentacion: "Saludo 1; Saludo 2; Saludo 3",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	p := cfg.Presentacion
	if p.TextoIdentificacion != "Soy un bot. Escribí agente para humano." {
		t.Errorf("TextoIdentificacion = %q", p.TextoIdentificacion)
	}
	if len(p.Variantes) != 3 || p.Variantes[0] != "Saludo 1" || p.Variantes[1] != "Saludo 2" || p.Variantes[2] != "Saludo 3" {
		t.Errorf("Variantes = %v", p.Variantes)
	}
}

func TestCargarPreservaComasEnPlantillasPresentacion(t *testing.T) {
	t.Parallel()

	// Probar que el separador ';' permite comas literales dentro de cada variante
	cfg, err := configuracion.Cargar(entornoFalso(map[string]string{
		configuracion.VariablePlantillasPresentacion: "Hola, ¿cómo estás?; Buenas, un gusto saludarte.",
	}))
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	if len(cfg.Presentacion.Variantes) != 2 {
		t.Fatalf("se esperaban 2 variantes, se obtuvieron %d: %v", len(cfg.Presentacion.Variantes), cfg.Presentacion.Variantes)
	}
	if cfg.Presentacion.Variantes[0] != "Hola, ¿cómo estás?" {
		t.Errorf("variante 0 = %q, se esperaba 'Hola, ¿cómo estás?'", cfg.Presentacion.Variantes[0])
	}
	if cfg.Presentacion.Variantes[1] != "Buenas, un gusto saludarte." {
		t.Errorf("variante 1 = %q, se esperaba 'Buenas, un gusto saludarte.'", cfg.Presentacion.Variantes[1])
	}
}

func TestCargarRechazaParametrosDePresentacionInvalidos(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre  string
		entorno map[string]string
	}{
		{"texto_identificacion_vacio", map[string]string{configuracion.VariableTextoIdentificacion: ""}},
		{"texto_identificacion_espacios", map[string]string{configuracion.VariableTextoIdentificacion: "   "}},
		{"plantillas_vacia", map[string]string{configuracion.VariablePlantillasPresentacion: ""}},
		{"plantillas_espacios", map[string]string{configuracion.VariablePlantillasPresentacion: "   "}},
		{"plantillas_una_sola_variante", map[string]string{configuracion.VariablePlantillasPresentacion: "Solo una variante"}},
		{"plantillas_variantes_vacias", map[string]string{configuracion.VariablePlantillasPresentacion: "; ; ;"}},
		{"plantillas_una_valida_una_vacia", map[string]string{configuracion.VariablePlantillasPresentacion: "Una valida; "}},
	}

	for _, c := range casos {
		t.Run(c.nombre, func(t *testing.T) {
			_, err := configuracion.Cargar(entornoFalso(c.entorno))
			if !errors.Is(err, configuracion.ErrParametroDeDisciplinaInvalido) {
				t.Errorf("caso %s: se esperaba ErrParametroDeDisciplinaInvalido, se obtuvo %v", c.nombre, err)
			}
		})
	}
}

func TestDisciplinaNoContieneCamposBooleanos(t *testing.T) {
	t.Parallel()

	tipos := []reflect.Type{
		reflect.TypeOf(configuracion.Disciplina{}),
		reflect.TypeOf(configuracion.VentanaDeAtencion{}),
		reflect.TypeOf(configuracion.RampaDeVolumen{}),
		reflect.TypeOf(configuracion.Cortacircuitos{}),
		reflect.TypeOf(configuracion.Presentacion{}),
		reflect.TypeOf(configuracion.Configuracion{}),
	}

	for _, tp := range tipos {
		for i := 0; i < tp.NumField(); i++ {
			f := tp.Field(i)
			if f.Type.Kind() == reflect.Bool {
				t.Errorf("el tipo %s contiene un campo booleano (%s): la disciplina no debe admitir apagado booleano", tp.Name(), f.Name)
			}
		}
	}
}
