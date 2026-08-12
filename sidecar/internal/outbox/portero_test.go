package outbox_test

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

type controlDeBajaEspia struct {
	ordenLlamadas       []string
	permitido           bool
	errPermitido        error
	reclamada           bool
	errReclamar         error
	llamadasReclamar    int
	ultimoIdMensajeConf string
}

func (c *controlDeBajaEspia) EnvioPermitido(_ context.Context, _, _ string) (bool, error) {
	c.ordenLlamadas = append(c.ordenLlamadas, "EnvioPermitido")
	return c.permitido, c.errPermitido
}

func (c *controlDeBajaEspia) ReclamarConfirmacionDeBaja(_ context.Context, _, idMensajeConfirmacion string, _ int64) (bool, error) {
	c.ordenLlamadas = append(c.ordenLlamadas, "ReclamarConfirmacionDeBaja")
	c.llamadasReclamar++
	c.ultimoIdMensajeConf = idMensajeConfirmacion
	return c.reclamada, c.errReclamar
}

var _ outbox.ControlDeBaja = (*controlDeBajaEspia)(nil)

type controlDeCortacircuitosEspia struct {
	ordenLlamadas    []string
	permitido        bool
	errPermitido     error
	reclamada        bool
	errReclamar      error
	llamadasReclamar int
	ultimoIdTraspaso string
}

func (c *controlDeCortacircuitosEspia) SalidaPermitida(_ context.Context, _, _ string) (bool, error) {
	c.ordenLlamadas = append(c.ordenLlamadas, "SalidaPermitida")
	return c.permitido, c.errPermitido
}

func (c *controlDeCortacircuitosEspia) ReclamarMensajeDeTraspaso(_ context.Context, _, idMensajeTraspaso string, _ int64) (bool, error) {
	c.ordenLlamadas = append(c.ordenLlamadas, "ReclamarMensajeDeTraspaso")
	c.llamadasReclamar++
	c.ultimoIdTraspaso = idMensajeTraspaso
	return c.reclamada, c.errReclamar
}

var _ outbox.ControlDeCortacircuitos = (*controlDeCortacircuitosEspia)(nil)

func TestPorteroDeSalida_AdmitirRechazaCortacircuitosDisparado(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	corta := &controlDeCortacircuitosEspia{permitido: false}

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	portero := outbox.NuevoPorteroDeSalida(cola, nil, corta, reg)
	ctx := context.Background()

	bloqueadasPrevias := outbox.ContadorBloqueadasPorCortacircuitos.Load()
	err := portero.Admitir(ctx, "msg-1", "conv-tripped", "hola", 100)
	if !errors.Is(err, outbox.ErrConversacionEnTraspaso) {
		t.Fatalf("se esperaba ErrConversacionEnTraspaso, se obtuvo %v", err)
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("cola_salida debía tener 0 filas, tiene %d", cuenta)
	}

	if outbox.ContadorBloqueadasPorCortacircuitos.Load() <= bloqueadasPrevias {
		t.Errorf("ContadorBloqueadasPorCortacircuitos no se incrementó")
	}
	if !strings.Contains(buf.String(), outbox.EventoEnvioBloqueadoPorCortacircuitos) {
		t.Errorf("registro no contiene %s: %s", outbox.EventoEnvioBloqueadoPorCortacircuitos, buf.String())
	}
}

func TestPorteroDeSalida_AdmitirFallaCerradoEnErrorCortacircuitos(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	corta := &controlDeCortacircuitosEspia{errPermitido: errors.New("fallo lectura")}

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	portero := outbox.NuevoPorteroDeSalida(cola, nil, corta, reg)
	ctx := context.Background()

	erroresPrevios := outbox.ContadorErroresCortacircuitos.Load()
	err := portero.Admitir(ctx, "msg-1", "conv-err", "hola", 100)
	if err == nil {
		t.Fatal("se esperaba error al fallar lectura de cortacircuitos (fallo cerrado)")
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("cola_salida debía tener 0 filas, tiene %d", cuenta)
	}

	if outbox.ContadorErroresCortacircuitos.Load() <= erroresPrevios {
		t.Errorf("ContadorErroresCortacircuitos no se incrementó")
	}
	if !strings.Contains(buf.String(), outbox.EventoErrorConsultaCortacircuitos) {
		t.Errorf("registro no contiene %s: %s", outbox.EventoErrorConsultaCortacircuitos, buf.String())
	}
}

func TestPorteroDeSalida_AdmitirRechazaContactoDadoDeBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	control := &controlDeBajaEspia{permitido: false}

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	portero := outbox.NuevoPorteroDeSalida(cola, control, nil, reg)
	ctx := context.Background()

	bloqueadasPrevias := outbox.ContadorBloqueadasPorBaja.Load()
	err := portero.Admitir(ctx, "msg-1", "conv-opted-out", "hola", 100)
	if !errors.Is(err, outbox.ErrContactoDadoDeBaja) {
		t.Fatalf("se esperaba ErrContactoDadoDeBaja, se obtuvo %v", err)
	}

	// cola_salida debe tener 0 filas
	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("cola_salida debía tener 0 filas, tiene %d", cuenta)
	}

	// Contador y log observables
	if outbox.ContadorBloqueadasPorBaja.Load() <= bloqueadasPrevias {
		t.Errorf("ContadorBloqueadasPorBaja no se incrementó")
	}
	if !strings.Contains(buf.String(), outbox.EventoEnvioBloqueadoPorBaja) {
		t.Errorf("registro no contiene %s: %s", outbox.EventoEnvioBloqueadoPorBaja, buf.String())
	}
}

func TestPorteroDeSalida_AdmitirConsultaCortacircuitosAntesDeBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	baja := &controlDeBajaEspia{permitido: true}
	corta := &controlDeCortacircuitosEspia{permitido: true}
	portero := outbox.NuevoPorteroDeSalida(cola, baja, corta, nil)
	ctx := context.Background()

	err := portero.Admitir(ctx, "msg-1", "conv-permitida", "hola", 100)
	if err != nil {
		t.Fatalf("Admitir falló: %v", err)
	}

	if len(corta.ordenLlamadas) != 1 || corta.ordenLlamadas[0] != "SalidaPermitida" {
		t.Fatalf("se esperaba que SalidaPermitida se consultara primero, llamadas: %v", corta.ordenLlamadas)
	}
	if len(baja.ordenLlamadas) != 1 || baja.ordenLlamadas[0] != "EnvioPermitido" {
		t.Fatalf("se esperaba que EnvioPermitido se consultara después, llamadas: %v", baja.ordenLlamadas)
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&cuenta)
	if cuenta != 1 {
		t.Fatalf("el mensaje admitido debía estar encolado")
	}
}

func TestPorteroDeSalida_AdmitirConfirmacionDeBaja_ReclamoUnico(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	control := &controlDeBajaEspia{reclamada: true}
	portero := outbox.NuevoPorteroDeSalida(cola, control, nil, nil)
	ctx := context.Background()

	// Primer reclamo: gana
	encolada, err := portero.AdmitirConfirmacionDeBaja(ctx, "msg-conf-1", "conv-1", "Texto confirmación", 100)
	if err != nil || !encolada {
		t.Fatalf("primer reclamo debía encolar: encolada=%v, err=%v", encolada, err)
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-conf-1'").Scan(&cuenta)
	if cuenta != 1 {
		t.Fatalf("la confirmación debía estar encolada")
	}

	// Segundo reclamo: ya no gana (reclamada=false)
	control.reclamada = false
	encolada2, err := portero.AdmitirConfirmacionDeBaja(ctx, "msg-conf-2", "conv-1", "Texto confirmación", 200)
	if err != nil {
		t.Fatalf("segundo reclamo no debía dar error: %v", err)
	}
	if encolada2 {
		t.Fatal("segundo reclamo debía retornar encolada=false")
	}

	// La segunda confirmación no se encoló
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-conf-2'").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("la segunda confirmación no debía encolarse")
	}
}

func TestPorteroDeSalida_AdmitirMensajeDeTraspaso_ReclamoUnico(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	corta := &controlDeCortacircuitosEspia{permitido: true, reclamada: true}
	baja := &controlDeBajaEspia{permitido: true}
	portero := outbox.NuevoPorteroDeSalida(cola, baja, corta, nil)
	ctx := context.Background()

	// Primer reclamo: gana
	encolada, err := portero.AdmitirMensajeDeTraspaso(ctx, "msg-traspaso-1", "conv-1", "Texto traspaso", 100)
	if err != nil || !encolada {
		t.Fatalf("primer reclamo de traspaso debía encolar: encolada=%v, err=%v", encolada, err)
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-traspaso-1'").Scan(&cuenta)
	if cuenta != 1 {
		t.Fatalf("el traspaso debía estar encolado")
	}

	// Segundo reclamo: no gana (reclamada=false)
	corta.reclamada = false
	encolada2, err := portero.AdmitirMensajeDeTraspaso(ctx, "msg-traspaso-2", "conv-1", "Texto traspaso", 200)
	if err != nil {
		t.Fatalf("segundo reclamo no debía dar error: %v", err)
	}
	if encolada2 {
		t.Fatal("segundo reclamo debía retornar encolada=false")
	}

	db.QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-traspaso-2'").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("el segundo traspaso no debía encolarse")
	}
}

func TestPorteroDeSalida_AdmitirMensajeDeTraspaso_PrecedenciaDeBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	corta := &controlDeCortacircuitosEspia{permitido: true, reclamada: true}
	baja := &controlDeBajaEspia{permitido: false} // Contacto dado de baja
	portero := outbox.NuevoPorteroDeSalida(cola, baja, corta, nil)
	ctx := context.Background()

	// AdmitirMensajeDeTraspaso debe rebotar por la comprobación de baja
	encolada, err := portero.AdmitirMensajeDeTraspaso(ctx, "msg-traspaso-1", "conv-baja", "Texto traspaso", 100)
	if !errors.Is(err, outbox.ErrContactoDadoDeBaja) {
		t.Fatalf("se esperaba ErrContactoDadoDeBaja por precedencia de STOP, se obtuvo err=%v", err)
	}
	if encolada {
		t.Fatal("no debía encolar el traspaso para un contacto dado de baja")
	}

	var cuenta int
	db.QueryRow("SELECT COUNT(*) FROM cola_salida").Scan(&cuenta)
	if cuenta != 0 {
		t.Fatalf("cola_salida debía tener 0 filas, tiene %d", cuenta)
	}
}
