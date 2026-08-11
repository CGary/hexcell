package outbox_test

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	_ "modernc.org/sqlite"
)

func abrirDbPruebaVolumen(t *testing.T) *sql.DB {
	ruta := filepath.Join(t.TempDir(), "test_volumen.db")
	dsn := "file:" + ruta + "?_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("error abriendo bd: %v", err)
	}
	esquema := `
	CREATE TABLE IF NOT EXISTS volumen_diario (
		dia TEXT PRIMARY KEY,
		envios INTEGER DEFAULT 0
	);
	`
	if _, err := db.Exec(esquema); err != nil {
		t.Fatalf("error creando esquema: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

func TestDisciplinaDecidir_LatenciaMinima(t *testing.T) {
	t.Parallel()

	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{
			DiariaInicial:     100,
			IncrementoSemanal: 10,
			Semanas:           4,
		},
	}
	d := outbox.NuevaDisciplinaDeSalida(cfg)

	// Martes 2026-08-11 12:00:00 UTC = 1786449600000 ms
	ahoraMs := int64(1786449600000)
	origenMs := ahoraMs - 1500 // solo 1500 ms de antigüedad (< 3000 ms)

	transmitir, motivo := d.Decidir(ahoraMs, origenMs, 0, 0)
	if transmitir {
		t.Errorf("se esperaba aplazamiento por latencia")
	}
	if motivo != outbox.MotivoLatenciaMinima {
		t.Errorf("motivo = %v, se esperaba MotivoLatenciaMinima", motivo)
	}

	// Con 3000 ms o más transcurridos
	origenMs = ahoraMs - 3000
	transmitir, motivo = d.Decidir(ahoraMs, origenMs, 0, 0)
	if !transmitir || motivo != outbox.MotivoNinguno {
		t.Errorf("se esperaba transmitir=true, motivo=MotivoNinguno; obtenido %v, %v", transmitir, motivo)
	}
}

func TestDisciplinaDecidir_VentanaDeAtencion(t *testing.T) {
	t.Parallel()

	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 3000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 9, MinutoApertura: 0,
			HoraCierre: 19, MinutoCierre: 0,
			Dias: []int{1, 2, 3, 4, 5}, // lunes a viernes
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{
			DiariaInicial:     100,
			IncrementoSemanal: 10,
			Semanas:           4,
		},
	}
	d := outbox.NuevaDisciplinaDeSalida(cfg)

	// Martes 2026-08-11 08:30:00 en Buenos Aires (antes de apertura)
	// Buenos Aires es UTC-3 -> 08:30 local es 11:30 UTC
	tAntes := time.Date(2026, 8, 11, 11, 30, 0, 0, time.UTC).UnixMilli()
	transmitir, motivo := d.Decidir(tAntes, tAntes-5000, 0, 0)
	if transmitir || motivo != outbox.MotivoFueraDeHorario {
		t.Errorf("antes de apertura: se esperaba false, MotivoFueraDeHorario; obtenido %v, %v", transmitir, motivo)
	}

	// Martes 2026-08-11 12:00:00 en Buenos Aires (dentro de ventana)
	// 12:00 local es 15:00 UTC
	tDentro := time.Date(2026, 8, 11, 15, 0, 0, 0, time.UTC).UnixMilli()
	transmitir, motivo = d.Decidir(tDentro, tDentro-5000, 0, 0)
	if !transmitir || motivo != outbox.MotivoNinguno {
		t.Errorf("dentro de horario: se esperaba true, MotivoNinguno; obtenido %v, %v", transmitir, motivo)
	}

	// Martes 2026-08-11 19:00:00 en Buenos Aires (en cierre, fuera)
	// 19:00 local es 22:00 UTC
	tCierre := time.Date(2026, 8, 11, 22, 0, 0, 0, time.UTC).UnixMilli()
	transmitir, motivo = d.Decidir(tCierre, tCierre-5000, 0, 0)
	if transmitir || motivo != outbox.MotivoFueraDeHorario {
		t.Errorf("al cierre: se esperaba false, MotivoFueraDeHorario; obtenido %v, %v", transmitir, motivo)
	}

	// Domingo 2026-08-16 12:00:00 en Buenos Aires (día no hábil)
	tDomingo := time.Date(2026, 8, 16, 15, 0, 0, 0, time.UTC).UnixMilli()
	transmitir, motivo = d.Decidir(tDomingo, tDomingo-5000, 0, 0)
	if transmitir || motivo != outbox.MotivoFueraDeHorario {
		t.Errorf("en fin de semana: se esperaba false, MotivoFueraDeHorario; obtenido %v, %v", transmitir, motivo)
	}
}

func TestDisciplinaDecidir_RampaDeVolumen(t *testing.T) {
	t.Parallel()

	loc, _ := time.LoadLocation("America/Argentina/Buenos_Aires")
	cfg := configuracion.Disciplina{
		LatenciaMinimaMs: 1000,
		Ventana: configuracion.VentanaDeAtencion{
			HoraApertura: 0, MinutoApertura: 0,
			HoraCierre: 23, MinutoCierre: 59,
			Dias: []int{1, 2, 3, 4, 5, 6, 7},
			Zona: loc,
		},
		Rampa: configuracion.RampaDeVolumen{
			DiariaInicial:     20,
			IncrementoSemanal: 20,
			Semanas:           4,
		},
	}
	d := outbox.NuevaDisciplinaDeSalida(cfg)

	// Día 1 (semana 0): cupo = 20
	primerDia := time.Date(2026, 8, 1, 0, 0, 0, 0, time.UTC).UnixMilli()
	ahoraDia1 := primerDia + 10*3600*1000 // mismo día

	// Con 19 enviados hoy: permitido
	transmitir, motivo := d.Decidir(ahoraDia1, ahoraDia1-2000, 19, primerDia)
	if !transmitir || motivo != outbox.MotivoNinguno {
		t.Errorf("semana 0 con 19 envíos: se esperaba true; obtenido %v, %v", transmitir, motivo)
	}

	// Con 20 enviados hoy: cupo alcanzado
	transmitir, motivo = d.Decidir(ahoraDia1, ahoraDia1-2000, 20, primerDia)
	if transmitir || motivo != outbox.MotivoRampaDeVolumen {
		t.Errorf("semana 0 con 20 envíos: se esperaba false, MotivoRampaDeVolumen; obtenido %v, %v", transmitir, motivo)
	}

	// Semana 2 (día 15): cupo = 20 + 2*20 = 60
	ahoraSemana2 := primerDia + 15*24*3600*1000
	transmitir, motivo = d.Decidir(ahoraSemana2, ahoraSemana2-2000, 59, primerDia)
	if !transmitir || motivo != outbox.MotivoNinguno {
		t.Errorf("semana 2 con 59 envíos: se esperaba true; obtenido %v, %v", transmitir, motivo)
	}
	transmitir, motivo = d.Decidir(ahoraSemana2, ahoraSemana2-2000, 60, primerDia)
	if transmitir || motivo != outbox.MotivoRampaDeVolumen {
		t.Errorf("semana 2 con 60 envíos: se esperaba false, MotivoRampaDeVolumen; obtenido %v, %v", transmitir, motivo)
	}

	// Semana 6 (después de semana 4): cupo = 20 + 4*20 = 100 (tope permanente)
	ahoraSemana6 := primerDia + 45*24*3600*1000
	transmitir, motivo = d.Decidir(ahoraSemana6, ahoraSemana6-2000, 99, primerDia)
	if !transmitir || motivo != outbox.MotivoNinguno {
		t.Errorf("semana 6 con 99 envíos: se esperaba true; obtenido %v, %v", transmitir, motivo)
	}
	transmitir, motivo = d.Decidir(ahoraSemana6, ahoraSemana6-2000, 100, primerDia)
	if transmitir || motivo != outbox.MotivoRampaDeVolumen {
		t.Errorf("semana 6 con 100 envíos: se esperaba false, MotivoRampaDeVolumen; obtenido %v, %v", transmitir, motivo)
	}
}

func TestPersistenciaVolumenDiario(t *testing.T) {
	t.Parallel()
	db := abrirDbPruebaVolumen(t)
	ctx := context.Background()

	// Inicialmente vacía
	primerDia, err := outbox.PrimerDiaConEnvios(ctx, db)
	if err != nil {
		t.Fatalf("error consultando primer día: %v", err)
	}
	if primerDia != 0 {
		t.Errorf("primer día inicial = %d, se esperaba 0", primerDia)
	}

	envios, err := outbox.EnviosDelDia(ctx, db, "2026-08-11")
	if err != nil {
		t.Fatalf("error consultando envíos del día: %v", err)
	}
	if envios != 0 {
		t.Errorf("envíos iniciales = %d, se esperaba 0", envios)
	}

	// Registrar envíos
	if err := outbox.RegistrarEnvioDelDia(ctx, db, "2026-08-11"); err != nil {
		t.Fatalf("error registrando envío: %v", err)
	}
	if err := outbox.RegistrarEnvioDelDia(ctx, db, "2026-08-11"); err != nil {
		t.Fatalf("error registrando segundo envío: %v", err)
	}

	envios, err = outbox.EnviosDelDia(ctx, db, "2026-08-11")
	if err != nil {
		t.Fatalf("error consultando envíos: %v", err)
	}
	if envios != 2 {
		t.Errorf("envíos = %d, se esperaba 2", envios)
	}

	primerDia, err = outbox.PrimerDiaConEnvios(ctx, db)
	if err != nil {
		t.Fatalf("error consultando primer día: %v", err)
	}
	esperadoMs := time.Date(2026, 8, 11, 0, 0, 0, 0, time.UTC).UnixMilli()
	if primerDia != esperadoMs {
		t.Errorf("primer día = %d, se esperaba %d", primerDia, esperadoMs)
	}
}
