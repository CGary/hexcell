package canal_test

import (
	"bytes"
	"context"
	"log/slog"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

func TestIniciarEmparejamientoQrSobreAlmacenVacioDevuelveErrorAlNoPoderConectar(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx, cancel := context.WithCancel(context.Background())
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	// Cancelamos DESPUÉS de construir la sesión, no antes: GetQRChannel ignora el contexto que
	// recibe, así que el fallo tiene que venir de Conectar(s.ctx). Cancelando aquí, el mismo
	// valor de contexto que NuevaSesion guardó ya está cancelado cuando IniciarEmparejamientoQr
	// llega a Conectar, y esa es la llamada que realmente falla — sin tocar la red y sin
	// necesidad de ningún cambio de producción para lograrlo.
	cancel()

	canalQr, err := sesion.IniciarEmparejamientoQr()
	if err == nil {
		t.Fatalf("se esperaba un error al intentar conectar con contexto cancelado")
	}
	const mensajeEsperado = "canal: no se pudo conectar para iniciar emparejamiento QR"
	if !strings.Contains(err.Error(), mensajeEsperado) {
		t.Fatalf("error = %q, no contiene %q; una afirmación solo de err != nil no distingue esta falla de la del código pre-corrección", err.Error(), mensajeEsperado)
	}
	if canalQr != nil {
		t.Fatalf("se esperaba un canal QR nil en caso de fallo de conexión")
	}
}

func TestSolicitarCodigoDeVinculacionRechazaTelefonoVacio(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	ctx := context.Background()
	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "")
	if err != canal.ErrTelefonoNoConfigurado {
		t.Fatalf("error = %v, se esperaba ErrTelefonoNoConfigurado", err)
	}
}

func TestSolicitarCodigoDeVinculacionDevuelveErrorAlNoPoderConectarTelefonoCorto(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	sesion, err := canal.NuevaSesion(context.Background(), contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	// Con Conectar corriendo antes que PairPhone, el contexto cancelado hace fallar la conexión
	// antes de que whatsmeow llegue a inspeccionar el número: este "123" corto ya no ejercita la
	// validación de formato de whatsmeow (ver la nota de limitación honesta más abajo), solo la
	// identidad del fallo de conexión, que es lo que la afirmación siguiente comprueba.
	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "123")
	if err == nil {
		t.Fatalf("se esperaba un error al intentar conectar con contexto cancelado")
	}
	const mensajeEsperado = "canal: no se pudo conectar para solicitar código de vinculación"
	if !strings.Contains(err.Error(), mensajeEsperado) {
		t.Fatalf("error = %q, no contiene %q; una afirmación solo de err != nil no distingue esta falla de la del código pre-corrección", err.Error(), mensajeEsperado)
	}
}

func TestSolicitarCodigoDeVinculacionDevuelveErrorAlNoPoderConectarTelefonoConCero(t *testing.T) {
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")
	contenedor := abrirAlmacenDePrueba(t, reg)

	sesion, err := canal.NuevaSesion(context.Background(), contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	// Misma razón que en el caso del teléfono corto: el fallo viene de Conectar, no de la
	// validación de formato de whatsmeow, que ya no es alcanzable desde este contexto cancelado.
	_, err = sesion.SolicitarCodigoDeVinculacion(ctx, "05491155551234")
	if err == nil {
		t.Fatalf("se esperaba un error al intentar conectar con contexto cancelado")
	}
	const mensajeEsperado = "canal: no se pudo conectar para solicitar código de vinculación"
	if !strings.Contains(err.Error(), mensajeEsperado) {
		t.Fatalf("error = %q, no contiene %q; una afirmación solo de err != nil no distingue esta falla de la del código pre-corrección", err.Error(), mensajeEsperado)
	}
}

// Limitaciones documentadas honestamente (mismo patrón centinela que traducirItemQr, arriba en
// canal.go, y que TestElPayloadCentinelaDelQrNuncaLlegaAlRegistro en almacen_interno_test.go):
//
//   - Ninguna prueba de este paquete ejercita la rama verdadera de la guarda
//     `!s.cliente.IsConnected()` en SolicitarCodigoDeVinculacion, es decir, un cliente que YA
//     está conectado y por lo tanto NO vuelve a invocar Conectar. Llegar a esa rama exige una
//     conexión real contra la infraestructura de WhatsApp, que este paquete tiene prohibido
//     marcar en una prueba unitaria (forbid.behaviors de HEX-026). Existió una prueba con ese
//     nombre que nunca conectaba el cliente y afirmaba "no reconecta": probaba exactamente lo
//     contrario de lo que su nombre decía, porque ejercitaba la rama falsa de la guarda, no la
//     verdadera, y duplicaba el mismo camino de fallo de conexión que las dos pruebas de arriba.
//     Se eliminó en lugar de mantenerla en verde sin cobertura real.
//   - La validación de formato propia de whatsmeow (ErrPhoneNumberTooShort,
//     ErrPhoneNumberIsNotInternational, dentro de PairPhone) tampoco es alcanzable desde una
//     prueba sin red: ahora Conectar corre antes que PairPhone, así que un contexto cancelado
//     hace fallar la conexión antes de que PairPhone examine el número.
//   - La emisión real de un código QR (evento "code" del canal de whatsmeow) tampoco es
//     alcanzable sin conexión real. Su cobertura de no-filtrado de payload vive en
//     TestElPayloadCentinelaDelQrNuncaLlegaAlRegistro (almacen_interno_test.go), que invoca el
//     paso traducirItemQr directamente con un payload centinela, por el mismo motivo que ese
//     archivo documenta para el número de teléfono.

// La comprobación de que ningún payload de emparejamiento llega al registro NO vive aquí: desde
// fuera del paquete solo se puede drenar el canal de GetQRChannel, y sin websocket ese canal nunca
// emite un suceso "code", así que la afirmación inspeccionaría un búfer vacío y pasaría incluso con
// el payload interpolado en `detalle`. Vive en `almacen_interno_test.go`, que es `package canal` y
// puede invocar el paso emisor con un payload centinela.
