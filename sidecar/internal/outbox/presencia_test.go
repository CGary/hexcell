package outbox_test

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"strings"
	"sync"
	"testing"

	"go.mau.fi/whatsmeow/types"

	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

type clientePresenciaFalso struct {
	mu           sync.Mutex
	llamadas     int
	fallar       bool
	ultimoJID    types.JID
	ultimoEstado types.ChatPresence
}

func (c *clientePresenciaFalso) SendChatPresence(_ context.Context, jid types.JID, state types.ChatPresence, _ types.ChatPresenceMedia) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.llamadas++
	c.ultimoJID = jid
	c.ultimoEstado = state
	if c.fallar {
		return errors.New("fallo simulado de presencia whatsmeow")
	}
	return nil
}

type resolutorFalso struct {
	mu     sync.Mutex
	fallar bool
	jid    types.JID
}

func (r *resolutorFalso) DireccionDe(_ context.Context, _ string) (types.JID, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.fallar {
		return types.EmptyJID, errors.New("contacto desconocido")
	}
	return r.jid, nil
}

func TestEmisorDePresencia_Exitoso(t *testing.T) {
	t.Parallel()

	jidEsperado := types.NewJID("5491155551234", types.DefaultUserServer)
	cli := &clientePresenciaFalso{}
	res := &resolutorFalso{jid: jidEsperado}
	emisor := outbox.NuevoEmisorDePresenciaWhatsmeow(cli, res, nil)

	ctx := context.Background()
	err := emisor.EmitirEscribiendo(ctx, "conv-1")
	if err != nil {
		t.Fatalf("no se esperaba error: %v", err)
	}

	cli.mu.Lock()
	defer cli.mu.Unlock()
	if cli.llamadas != 1 {
		t.Errorf("llamadas = %d, se esperaba 1", cli.llamadas)
	}
	if cli.ultimoJID != jidEsperado {
		t.Errorf("jid = %v, se esperaba %v", cli.ultimoJID, jidEsperado)
	}
	if cli.ultimoEstado != types.ChatPresenceComposing {
		t.Errorf("estado = %v, se esperaba %v", cli.ultimoEstado, types.ChatPresenceComposing)
	}
}

func TestEmisorDePresencia_FalloResolucion(t *testing.T) {
	t.Parallel()

	cli := &clientePresenciaFalso{}
	res := &resolutorFalso{fallar: true}
	emisor := outbox.NuevoEmisorDePresenciaWhatsmeow(cli, res, nil)

	ctx := context.Background()
	err := emisor.EmitirEscribiendo(ctx, "conv-1")
	if err == nil {
		t.Fatal("se esperaba error ante fallo de resolución")
	}
	if cli.llamadas != 0 {
		t.Errorf("no se debió invocar el cliente de presencia")
	}
}

func TestEmisorDePresencia_FalloTransporteRegistraAviso(t *testing.T) {
	t.Parallel()

	var buf bytes.Buffer
	reg := registro.Nuevo(&buf, slog.LevelInfo, "celula-test")

	jidEsperado := types.NewJID("5491155551234", types.DefaultUserServer)
	cli := &clientePresenciaFalso{fallar: true}
	res := &resolutorFalso{jid: jidEsperado}
	emisor := outbox.NuevoEmisorDePresenciaWhatsmeow(cli, res, reg)

	ctx := context.Background()
	err := emisor.EmitirEscribiendo(ctx, "conv-1")
	if err == nil {
		t.Fatal("se esperaba error del transporte")
	}

	salida := buf.String()
	if !strings.Contains(salida, "outbox.error_presencia") {
		t.Errorf("se esperaba evento outbox.error_presencia en el registro, obtenido: %s", salida)
	}
}
