package canal

import (
	"reflect"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
)

const ahoraFijoMs int64 = 1_786_083_200_000

func eventosDeDesconexionDePrueba() map[string]any {
	return map[string]any{
		ipc.CausaDispositivoRemovido: &events.LoggedOut{
			OnConnect: false,
			Reason:    events.ConnectFailureLoggedOut,
		},
		ipc.CausaSesionCerrada: &events.LoggedOut{
			OnConnect: true,
			Reason:    events.ConnectFailureLoggedOut,
		},
		ipc.CausaBaneoTemporal: &events.TemporaryBan{
			Code:   events.TempBanBlockedByUsers,
			Expire: 30 * time.Minute,
		},
		ipc.CausaSesionReemplazada:       &events.StreamReplaced{},
		ipc.CausaFalloDeConexion:         &events.ConnectFailure{Reason: events.ConnectFailureServiceUnavailable},
		ipc.CausaErrorDeFlujo:            &events.StreamError{Code: "599"},
		ipc.CausaDesconexionDeTransporte: &events.Disconnected{},
		ipc.CausaClienteObsoleto:         &events.ClientOutdated{},
	}
}

func TestClasificarDesconexionUsaOnConnectParaDeviceRemoved(t *testing.T) {
	t.Parallel()

	removido, ok := clasificarDesconexion(&events.LoggedOut{
		OnConnect: false,
		Reason:    events.ConnectFailureLoggedOut,
	}, ahoraFijoMs)
	if !ok {
		t.Fatalf("LoggedOut OnConnect=false no fue clasificado")
	}
	cerrada, ok := clasificarDesconexion(&events.LoggedOut{
		OnConnect: true,
		Reason:    events.ConnectFailureLoggedOut,
	}, ahoraFijoMs)
	if !ok {
		t.Fatalf("LoggedOut OnConnect=true no fue clasificado")
	}
	if removido.Causa != ipc.CausaDispositivoRemovido {
		t.Fatalf("causa OnConnect=false = %q", removido.Causa)
	}
	if cerrada.Causa != ipc.CausaSesionCerrada {
		t.Fatalf("causa OnConnect=true = %q", cerrada.Causa)
	}
	if removido.Causa == cerrada.Causa {
		t.Fatalf("OnConnect no distingue ramas: ambas producen %q", removido.Causa)
	}
	if removido.Codigo != int64(events.ConnectFailureLoggedOut) || cerrada.Codigo != int64(events.ConnectFailureLoggedOut) {
		t.Fatalf("las razones no viajaron como codigo: removido=%d cerrada=%d", removido.Codigo, cerrada.Codigo)
	}
}

func TestClasificarDesconexionProduceUnaCausaPorVariante(t *testing.T) {
	t.Parallel()

	producidas := make(map[string]struct{})
	for causaEsperada, evento := range eventosDeDesconexionDePrueba() {
		causaEsperada := causaEsperada
		evento := evento
		t.Run(causaEsperada, func(t *testing.T) {
			desc, ok := clasificarDesconexion(evento, ahoraFijoMs)
			if !ok {
				t.Fatalf("evento %T no fue clasificado", evento)
			}
			if desc.Causa != causaEsperada {
				t.Fatalf("causa = %q, se esperaba %q", desc.Causa, causaEsperada)
			}
			if _, repetida := producidas[desc.Causa]; repetida {
				t.Fatalf("causa duplicada: %q", desc.Causa)
			}
			producidas[desc.Causa] = struct{}{}
		})
	}
}

func TestClasificarBaneoTemporalConvierteExpiracionRelativa(t *testing.T) {
	t.Parallel()

	desc, ok := clasificarDesconexion(&events.TemporaryBan{
		Code:   events.TempBanBlockedByUsers,
		Expire: 30 * time.Minute,
	}, ahoraFijoMs)
	if !ok {
		t.Fatalf("TemporaryBan no fue clasificado")
	}
	if desc.Causa != ipc.CausaBaneoTemporal {
		t.Fatalf("causa = %q", desc.Causa)
	}
	if desc.Codigo != int64(events.TempBanBlockedByUsers) {
		t.Fatalf("codigo = %d", desc.Codigo)
	}
	if desc.ExpiraEnMs != ahoraFijoMs+(30*time.Minute).Milliseconds() {
		t.Fatalf("expira_en_ms = %d", desc.ExpiraEnMs)
	}

	sinExpiracion, ok := clasificarDesconexion(&events.TemporaryBan{
		Code:   events.TempBanSentToTooManyPeople,
		Expire: 0,
	}, ahoraFijoMs)
	if !ok {
		t.Fatalf("TemporaryBan sin expiracion no fue clasificado")
	}
	if sinExpiracion.ExpiraEnMs != 0 {
		t.Fatalf("expira_en_ms desconocida = %d, se esperaba 0", sinExpiracion.ExpiraEnMs)
	}
}

func TestProyectarEstadoCubreTodasLasCausasDeclaradas(t *testing.T) {
	t.Parallel()

	for _, causa := range ipc.CausasDeclaradas() {
		causa := causa
		t.Run(causa, func(t *testing.T) {
			estado := proyectarEstado(causa)
			if estado == "" {
				t.Fatalf("estado vacío para causa %q", causa)
			}
			switch causa {
			case ipc.CausaBaneoTemporal:
				if estado != ipc.EstadoPausada {
					t.Fatalf("estado = %q", estado)
				}
			case ipc.CausaDispositivoRemovido, ipc.CausaSesionCerrada:
				if estado != ipc.EstadoDesvinculada {
					t.Fatalf("estado = %q", estado)
				}
			default:
				if estado != ipc.EstadoReconectando {
					t.Fatalf("estado = %q", estado)
				}
			}
		})
	}
}

func TestCausasDeclaradasSonProduciblesPorElClasificador(t *testing.T) {
	t.Parallel()

	producidas := make([]string, 0, len(eventosDeDesconexionDePrueba()))
	for _, evento := range eventosDeDesconexionDePrueba() {
		desc, ok := clasificarDesconexion(evento, ahoraFijoMs)
		if !ok {
			t.Fatalf("evento %T no fue clasificado", evento)
		}
		producidas = append(producidas, desc.Causa)
	}
	if !reflect.DeepEqual(conjunto(producidas), conjunto(ipc.CausasDeclaradas())) {
		t.Fatalf("causas producibles = %v, declaradas = %v", conjunto(producidas), conjunto(ipc.CausasDeclaradas()))
	}
}

func conjunto(valores []string) map[string]struct{} {
	resultado := make(map[string]struct{}, len(valores))
	for _, valor := range valores {
		resultado[valor] = struct{}{}
	}
	return resultado
}
