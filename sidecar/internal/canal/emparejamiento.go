// Operaciones de emparejamiento del sidecar: QR y código de vinculación de ocho caracteres.
//
// Ambos métodos comparten una restricción: si la sesión ya tiene credenciales emparejadas, no
// se permite iniciar un nuevo emparejamiento. Esto refleja el comportamiento de whatsmeow, que
// devuelve whatsmeow.ErrQRStoreContainsID desde GetQRChannel si el almacén ya tiene un dispositivo.
//
// Ninguno de los dos métodos registra el payload (el QR ni el código): son material de credencial
// y adr-0019 prohíbe que lleguen a un log a cualquier nivel.
package canal

import (
	"context"
	"errors"
	"time"

	"go.mau.fi/whatsmeow"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// Nombres fijos de suceso del emparejamiento, constantes por adr-0019.
const (
	EventoEmparejamientoQrIniciado   = "canal.emparejamiento_qr_iniciado"
	EventoEmparejamientoQrCodigo     = "canal.emparejamiento_qr_codigo"
	EventoEmparejamientoQrExpirado   = "canal.emparejamiento_qr_expirado"
	EventoEmparejamientoQrCompletado = "canal.emparejamiento_qr_completado"
	EventoEmparejamientoPcSolicitado = "canal.emparejamiento_pc_solicitado"
	EventoEmparejamientoPcRecibido   = "canal.emparejamiento_pc_recibido"
	EventoEmparejamientoRechazado    = "canal.emparejamiento_rechazado"
)

// ErrYaEmparejada se devuelve cuando se intenta emparejar una sesión que ya tiene credenciales.
var ErrYaEmparejada = errors.New("canal: la sesión ya está emparejada")

// ErrTelefonoNoConfigurado se devuelve cuando el código de vinculación necesita el número de la
// célula y no está configurado.
var ErrTelefonoNoConfigurado = errors.New("canal: el teléfono de la célula no está configurado")

// DuracionCodigoQr es la duración de validez de un código QR individual emitido por whatsmeow.
// whatsmeow emite un nuevo código cada 20 segundos aproximadamente; la expiración absoluta se
// calcula sumando esta duración al instante de emisión.
const DuracionCodigoQr = 20 * time.Second

// ResultadoQr describe el desenlace de un código QR individual del canal.
type ResultadoQr struct {
	// Codigo es la cadena que el consumidor codifica como imagen QR. Vacía si el QR ha
	// expirado o el emparejamiento se completó.
	Codigo string
	// ExpiraEnMs es el instante absoluto en milisegundos desde la época Unix en que este
	// código deja de ser válido. Cero si no aplica.
	ExpiraEnMs int64
	// Expirado indica que este código QR ya no es válido y el siguiente lo sustituye.
	Expirado bool
	// Completado indica que el emparejamiento se completó con éxito.
	Completado bool
}

// IniciarEmparejamientoQr inicia el flujo de emparejamiento por QR y devuelve un canal de
// resultados. Cada emisión contiene un código QR nuevo que sustituye al anterior.
//
// El canal se cierra cuando el emparejamiento se completa, expira sin que nadie escanee, o se
// produce un error. Nunca se registra el contenido del QR.
func (s *Sesion) IniciarEmparejamientoQr() (<-chan ResultadoQr, error) {
	if s.EstaEmparejada() {
		return nil, ErrYaEmparejada
	}

	canalQr, err := s.cliente.GetQRChannel(s.ctx)
	if err != nil {
		// whatsmeow devuelve ErrQRStoreContainsID si el almacén tiene un ID válido. En esta
		// versión el error vive en el paquete whatsmeow, no en store.
		if errors.Is(err, whatsmeow.ErrQRStoreContainsID) {
			return nil, ErrYaEmparejada
		}
		return nil, err
	}

	s.registro.Info(EventoEmparejamientoQrIniciado, registro.Campos{
		Detalle: "canal QR abierto; esperando códigos de whatsmeow",
	})

	resultados := make(chan ResultadoQr, 4)

	go func() {
		defer close(resultados)
		for item := range canalQr {
			if resultado, hayResultado := s.traducirItemQr(item); hayResultado {
				resultados <- resultado
			}
		}
	}()

	return resultados, nil
}

// traducirItemQr convierte un elemento del canal de whatsmeow en un ResultadoQr y emite la línea
// de registro que le corresponde. Devuelve falso en el segundo valor si el elemento no produce
// ningún resultado para el consumidor.
//
// Es un paso propio, y no código embebido en la goroutine, justamente para que sea invocable de
// forma independiente: sin una conexión real whatsmeow nunca emite un suceso "code", así que la
// única manera de comprobar de verdad que el payload del QR no llega al registro (adr-0019) es
// llamar aquí con un payload centinela desde una prueba interna del paquete. Es el único punto
// donde el emparejamiento por QR escribe en el registro.
func (s *Sesion) traducirItemQr(item whatsmeow.QRChannelItem) (ResultadoQr, bool) {
	switch item.Event {
	case "code":
		ahora := time.Now()
		expiraEn := ahora.Add(DuracionCodigoQr).UnixMilli()
		s.registro.Info(EventoEmparejamientoQrCodigo, registro.Campos{
			Detalle: "código QR emitido; contenido no registrado",
		})
		return ResultadoQr{
			Codigo:     item.Code,
			ExpiraEnMs: expiraEn,
		}, true
	case "timeout":
		s.registro.Info(EventoEmparejamientoQrExpirado, registro.Campos{
			Detalle: "todos los códigos QR han expirado sin escaneo",
		})
		return ResultadoQr{Expirado: true}, true
	case "success":
		s.registro.Info(EventoEmparejamientoQrCompletado, registro.Campos{
			Detalle: "emparejamiento por QR completado",
		})
		return ResultadoQr{Completado: true}, true
	}
	return ResultadoQr{}, false
}

// SolicitarCodigoDeVinculacion solicita un código de vinculación de ocho caracteres para el
// número de teléfono configurado. El código se devuelve como cadena y nunca se registra.
//
// El número de teléfono se lee de la configuración de la célula, no de un campo IPC, porque
// la guardia de mensajes_test.go prohíbe campos con nombres de identificador de transporte.
func (s *Sesion) SolicitarCodigoDeVinculacion(ctx context.Context, telefono string) (string, error) {
	if s.EstaEmparejada() {
		return "", ErrYaEmparejada
	}
	if telefono == "" {
		return "", ErrTelefonoNoConfigurado
	}

	s.registro.Info(EventoEmparejamientoPcSolicitado, registro.Campos{
		Detalle: "código de vinculación solicitado; número no registrado",
	})

	codigo, err := s.cliente.PairPhone(ctx, telefono, true, whatsmeow.PairClientChrome, "Chrome (Linux)")
	if err != nil {
		return "", err
	}

	s.registro.Info(EventoEmparejamientoPcRecibido, registro.Campos{
		Detalle: "código de vinculación recibido; contenido no registrado",
	})

	return codigo, nil
}
