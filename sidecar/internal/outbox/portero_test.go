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

func TestPorteroDeSalida_AdmitirRechazaContactoDadoDeBaja(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	control := &controlDeBajaEspia{permitido: false}

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")
	portero := outbox.NuevoPorteroDeSalida(cola, control, reg)
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

func TestPorteroDeSalida_AdmitirConsultaControlAntesDeEncolar(t *testing.T) {
	t.Parallel()
	db, _ := abrirDbPruebaSalida(t)
	cola := outbox.NuevaColaDeSalida(db, 1000, 3, nil, nil, nil)
	control := &controlDeBajaEspia{permitido: true}
	portero := outbox.NuevoPorteroDeSalida(cola, control, nil)
	ctx := context.Background()

	err := portero.Admitir(ctx, "msg-1", "conv-permitida", "hola", 100)
	if err != nil {
		t.Fatalf("Admitir falló: %v", err)
	}

	if len(control.ordenLlamadas) != 1 || control.ordenLlamadas[0] != "EnvioPermitido" {
		t.Fatalf("se esperaba que EnvioPermitido se consultara primero, llamadas: %v", control.ordenLlamadas)
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
	portero := outbox.NuevoPorteroDeSalida(cola, control, nil)
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
