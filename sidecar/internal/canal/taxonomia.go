// Taxonomía de desconexión de whatsmeow: clasificador puro que proyecta los eventos de la
// biblioteca a los valores cerrados de causa y estado del protocolo IPC v1.2.
//
// Las dos funciones de este archivo son PURAS y NO EXPORTADAS a propósito:
//   - clasificarDesconexion acepta un evento de whatsmeow y un ahoraMs explícito (la misma
//     costura de reloj que Outbox.Purgar(ctx, ahora)) y devuelve un objeto de valor desconexion.
//   - proyectarEstado mapea cada causa a su estado, total sobre el vocabulario cerrado.
//
// Puras significa: sin cliente, sin socket, sin red, sin lectura de reloj de pared interna.
// La costura ahoraMs existe para convertir la duración RELATIVA de events.TemporaryBan.Expire
// a los milisegundos absolutos Unix epoch que el campo expira_en_ms del protocolo exige.
package canal

import (
	"strconv"

	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
)

// desconexion es el objeto de valor que el clasificador produce: la tripleta causa/codigo/expira
// que viaja en estado_sesion.
type desconexion struct {
	Causa      string
	Codigo     int64
	ExpiraEnMs int64
}

// clasificarDesconexion clasifica un evento de whatsmeow en una desconexion.
// Devuelve false en el segundo valor si el evento no es un tipo de desconexión reconocido.
//
// TRAMPAS PROBADAS contra whatsmeow v0.0.0-20260722203353-e9a033b24933:
//   - device_removed NO es un campo ni un valor de Reason. Su ÚNICA firma observable es
//     events.LoggedOut{OnConnect:false}. El discriminador es OnConnect, NO Reason.
//   - events.TemporaryBan.Expire es una duración RELATIVA. Conversión:
//     ahoraMs + Expire.Milliseconds(), con Expire==0 → expiraEnMs==0.
//   - events.ConnectFailure.Raw y events.StreamError.Raw son *waBinary.Node. NUNCA se
//     desreferencian: los tests pasan Raw:nil.
func clasificarDesconexion(evento any, ahoraMs int64) (desconexion, bool) {
	switch e := evento.(type) {
	case *events.LoggedOut:
		if !e.OnConnect {
			return desconexion{
				Causa:  ipc.CausaDispositivoRemovido,
				Codigo: int64(e.Reason),
			}, true
		}
		return desconexion{
			Causa:  ipc.CausaSesionCerrada,
			Codigo: int64(e.Reason),
		}, true
	case *events.TemporaryBan:
		var expiraEnMs int64
		if e.Expire > 0 {
			expiraEnMs = ahoraMs + e.Expire.Milliseconds()
		}
		return desconexion{
			Causa:      ipc.CausaBaneoTemporal,
			Codigo:     int64(e.Code),
			ExpiraEnMs: expiraEnMs,
		}, true
	case *events.StreamReplaced:
		return desconexion{Causa: ipc.CausaSesionReemplazada}, true
	case *events.ConnectFailure:
		// Raw *waBinary.Node NO se desreferencia: los tests pasan nil.
		return desconexion{
			Causa:  ipc.CausaFalloDeConexion,
			Codigo: int64(e.Reason),
		}, true
	case *events.StreamError:
		// Raw *waBinary.Node NO se desreferencia: los tests pasan nil.
		codigo, _ := strconv.ParseInt(e.Code, 10, 64)
		return desconexion{
			Causa:  ipc.CausaErrorDeFlujo,
			Codigo: codigo,
		}, true
	case *events.Disconnected:
		return desconexion{Causa: ipc.CausaDesconexionDeTransporte}, true
	case *events.ClientOutdated:
		return desconexion{Causa: ipc.CausaClienteObsoleto}, true
	default:
		return desconexion{}, false
	}
}

// proyectarEstado mapea una causa a su estado. La función es TOTAL sobre el vocabulario
// cerrado de CausasDeclaradas(): toda causa declarada tiene exactamente un estado.
//
// baneo_temporal → pausada (la célula espera, no reintenta, no reactiva automáticamente).
// dispositivo_removido, sesion_cerrada → desvinculada (la sesión no existe).
// Todas las demás (transitorias) → reconectando.
func proyectarEstado(causa string) string {
	switch causa {
	case ipc.CausaBaneoTemporal:
		return ipc.EstadoPausada
	case ipc.CausaDispositivoRemovido, ipc.CausaSesionCerrada:
		return ipc.EstadoDesvinculada
	case ipc.CausaSesionReemplazada, ipc.CausaFalloDeConexion, ipc.CausaErrorDeFlujo,
		ipc.CausaDesconexionDeTransporte, ipc.CausaClienteObsoleto:
		return ipc.EstadoReconectando
	default:
		// Valor desconocido: tratarlo como transitorio. El switch debe ser exhaustivo sobre
		// CausasDeclaradas(), pero el default evita un panic con un valor no previsto.
		return ipc.EstadoReconectando
	}
}
