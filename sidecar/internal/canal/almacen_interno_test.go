package canal

import (
	"context"
	"crypto/rand"
	"path/filepath"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waAdv"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"

	"github.com/CGary/hexcell/sidecar/internal/registro"

	"bytes"
	"log/slog"
)

// dispositivoSintetico crea un dispositivo con los campos mínimos que sqlstore.PutDevice
// acepta sin panic: Account no nulo, con las firmas de los tamaños que el esquema exige.
func dispositivoSintetico(contenedor *sqlstore.Container) *store.Device {
	disp := contenedor.NewDevice()

	// PutDevice hace dereference de Account sin nil check, así que hay que llenarlo.
	cuenta := &waAdv.ADVSignedDeviceIdentity{}

	// adv_details es NOT NULL sin restricción de longitud; adv_account_sig (64),
	// adv_account_sig_key (32) y adv_device_sig (64) tienen CHECK de longitud exacta.
	cuenta.Details = make([]byte, 16)
	cuenta.AccountSignature = make([]byte, 64)
	cuenta.AccountSignatureKey = make([]byte, 32)
	cuenta.DeviceSignature = make([]byte, 64)
	rand.Read(cuenta.AccountSignature)
	rand.Read(cuenta.AccountSignatureKey)
	rand.Read(cuenta.DeviceSignature)

	disp.Account = cuenta
	disp.PushName = "test"
	disp.ID = &types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	disp.Initialized = true

	return disp
}

func TestDispositivoPersistidoSobreviveAlCierreYReapertura(t *testing.T) {
	ruta := filepath.Join(t.TempDir(), "sqlstore.db")
	ctx := context.Background()
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")

	// Abrir, crear y persistir un dispositivo sintético.
	contenedor1, err := AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("primera apertura: %v", err)
	}
	disp := dispositivoSintetico(contenedor1)
	if err := contenedor1.PutDevice(ctx, disp); err != nil {
		t.Fatalf("PutDevice: %v", err)
	}
	contenedor1.Close()

	// Reabrir y comprobar que el dispositivo sigue ahí.
	contenedor2, err := AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("segunda apertura: %v", err)
	}
	defer contenedor2.Close()

	sesion, err := NuevaSesion(ctx, contenedor2, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	defer sesion.Cerrar()

	if !sesion.EstaEmparejada() {
		t.Errorf("la sesión debería estar emparejada tras reabrir con un dispositivo persistido")
	}
	if sesion.dispositivo.ID == nil {
		t.Errorf("el dispositivo reabierto no tiene ID")
	}
	if !sesion.dispositivo.Initialized {
		t.Errorf("el dispositivo reabierto no está inicializado")
	}
}

// sesionVaciaDePrueba abre un almacén vacío bajo t.TempDir y devuelve la sesión junto al búfer
// donde se acumula todo lo que el registro emite, con el umbral más bajo posible para que ninguna
// línea quede fuera de la observación.
func sesionVaciaDePrueba(t *testing.T) (*Sesion, *bytes.Buffer) {
	t.Helper()

	ruta := filepath.Join(t.TempDir(), "sqlstore.db")
	ctx := context.Background()
	salida := &bytes.Buffer{}
	reg := registro.Nuevo(salida, slog.LevelDebug, "test")

	contenedor, err := AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("abrir almacén: %v", err)
	}
	t.Cleanup(func() { contenedor.Close() })

	sesion, err := NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	t.Cleanup(sesion.Cerrar)

	return sesion, salida
}

// TestElPayloadCentinelaDelQrNuncaLlegaAlRegistro invoca directamente el paso que emite la línea
// del código QR con un payload centinela. Sin conexión real whatsmeow jamás emite un suceso
// "code", así que drenar el canal de GetQRChannel no ejercita nada: esta prueba llama al paso a
// mano para que la afirmación se apoye en una línea realmente emitida.
//
// Si alguien interpola el contenido del QR en `detalle`, esta prueba falla (adr-0019).
func TestElPayloadCentinelaDelQrNuncaLlegaAlRegistro(t *testing.T) {
	const centinela = "2@SENTINELA,clave/de/sesion+secreta,=="

	sesion, salida := sesionVaciaDePrueba(t)

	resultado, hayResultado := sesion.traducirItemQr(whatsmeow.QRChannelItem{
		Event: "code",
		Code:  centinela,
	})
	if !hayResultado {
		t.Fatalf("un suceso \"code\" debe producir un resultado para el consumidor")
	}
	if resultado.Codigo != centinela {
		t.Fatalf("el resultado debe entregar el código intacto al consumidor, no %q", resultado.Codigo)
	}
	if resultado.ExpiraEnMs == 0 {
		t.Errorf("el resultado debe llevar la expiración absoluta del código")
	}

	// La línea del código QR tiene que haberse emitido: sin ella la afirmación sería vacua.
	texto := salida.String()
	if !strings.Contains(texto, EventoEmparejamientoQrCodigo) {
		t.Fatalf("no se emitió la línea del código QR; la prueba no estaría comprobando nada: %s", texto)
	}
	for _, prohibida := range []string{centinela, "SENTINELA", "2@"} {
		if strings.Contains(texto, prohibida) {
			t.Errorf("el registro contiene %q, que es material de credencial: %s", prohibida, texto)
		}
	}

	// Los otros dos sucesos del canal tampoco pueden filtrar nada aunque lleven Code poblado.
	for _, suceso := range []string{"timeout", "success"} {
		salida.Reset()
		if _, hay := sesion.traducirItemQr(whatsmeow.QRChannelItem{Event: suceso, Code: centinela}); !hay {
			t.Fatalf("el suceso %q debe producir un resultado", suceso)
		}
		if strings.Contains(salida.String(), "SENTINELA") {
			t.Errorf("el suceso %q filtró el payload al registro: %s", suceso, salida.String())
		}
	}
}

// TestElNumeroDeTelefonoNuncaLlegaAlRegistro comprueba la otra mitad de la frontera: la línea de
// EventoEmparejamientoPcSolicitado SÍ se ejecuta antes de que whatsmeow rechace el número, así
// que el registro resultante no es un búfer vacío y la afirmación tiene contenido.
func TestElNumeroDeTelefonoNuncaLlegaAlRegistro(t *testing.T) {
	// whatsmeow rechaza los números que empiezan con 0 antes de cualquier I/O.
	const numero = "05491155551234"

	sesion, salida := sesionVaciaDePrueba(t)

	if _, err := sesion.SolicitarCodigoDeVinculacion(context.Background(), numero); err == nil {
		t.Fatalf("se esperaba el rechazo de un número que empieza con 0")
	}

	texto := salida.String()
	if !strings.Contains(texto, EventoEmparejamientoPcSolicitado) {
		t.Fatalf("no se emitió la línea de solicitud; la prueba no estaría comprobando nada: %s", texto)
	}
	for _, prohibida := range []string{numero, strings.TrimPrefix(numero, "0")} {
		if strings.Contains(texto, prohibida) {
			t.Errorf("el registro contiene el número de la célula %q: %s", prohibida, texto)
		}
	}
}

func TestAlmacenConDispositivoPersistidoImpideQrConErrYaEmparejada(t *testing.T) {
	ruta := filepath.Join(t.TempDir(), "sqlstore.db")
	ctx := context.Background()
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "test")

	contenedor, err := AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("abrir almacén: %v", err)
	}
	defer contenedor.Close()

	disp := dispositivoSintetico(contenedor)
	if err := contenedor.PutDevice(ctx, disp); err != nil {
		t.Fatalf("PutDevice: %v", err)
	}

	// Reabrir para que GetFirstDevice encuentre el dispositivo.
	contenedor.Close()
	contenedor, err = AbrirAlmacenDeDispositivo(ctx, ruta, reg)
	if err != nil {
		t.Fatalf("reabrir almacén: %v", err)
	}
	defer contenedor.Close()

	sesion, err := NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		t.Fatalf("NuevaSesion: %v", err)
	}
	defer sesion.Cerrar()

	_, err = sesion.IniciarEmparejamientoQr()
	if err != ErrYaEmparejada {
		t.Errorf("error = %v, se esperaba ErrYaEmparejada", err)
	}
}
