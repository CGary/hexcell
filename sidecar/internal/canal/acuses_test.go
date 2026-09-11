package canal

import (
	"testing"
	"time"

	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
)

// recepcionDePrueba construye un *events.Receipt con el tipo y los MessageIDs dados, dejando el
// MessageSource incrustado en su valor cero (no lo lee ninguna rama de este archivo).
func recepcionDePrueba(tipo types.ReceiptType, ids ...string) *events.Receipt {
	return &events.Receipt{
		MessageIDs: ids,
		Type:       tipo,
		Timestamp:  time.UnixMilli(1_786_083_200_000),
	}
}

// TestClasificarAcuseEntregadoMapeaAEntregado cubre AC-1 en la mitad del filtro: un acuse entregado
// con MessageIDs se clasifica a ipc.EstadoEnvioEntregado. La mitad del sumidero (IdCorrelacion,
// no-emisión IPC) la cubre TestManejarEventoDeAcuseEntregaAlSumidero.
//
// Mutación a la que se enfrenta: invertir la rama Delivered/Read. Si el caso
// ReceiptTypeDelivered devolviera EstadoEnvioLeido (o al revés), este test y el de lectura se
// pondrían rojos.
func TestClasificarAcuseEntregadoMapeaAEntregado(t *testing.T) {
	t.Parallel()

	estado, ok := clasificarAcuse(recepcionDePrueba(types.ReceiptTypeDelivered, "whatsmeow-id-1"))
	if !ok {
		t.Fatalf("acuse entregado con MessageIDs no fue clasificado")
	}
	if estado != ipc.EstadoEnvioEntregado {
		t.Fatalf("estado = %q, se esperaba %q", estado, ipc.EstadoEnvioEntregado)
	}
}

// TestClasificarAcuseLeidoMapeaALeido cubre AC-2 en la mitad del filtro.
func TestClasificarAcuseLeidoMapeaALeido(t *testing.T) {
	t.Parallel()

	estado, ok := clasificarAcuse(recepcionDePrueba(types.ReceiptTypeRead, "whatsmeow-id-2"))
	if !ok {
		t.Fatalf("acuse leído con MessageIDs no fue clasificado")
	}
	if estado != ipc.EstadoEnvioLeido {
		t.Fatalf("estado = %q, se esperaba %q", estado, ipc.EstadoEnvioLeido)
	}
}

// TestClasificarAcuseRechazaElValorCeroFrenteAEntregado cubre AC-3, la trampa de la cadena vacía.
//
// types.ReceiptTypeDelivered es la cadena vacía, de modo que un events.Receipt{} de valor cero y un
// acuse entregado explícito comparten Type=="". El discriminador es len(r.MessageIDs) > 0: todo
// camino de whatsmeow que despacha un *events.Receipt le asigna al menos un MessageIDs, así que un
// struct de valor cero no es un acuse genuino y debe rechazarse.
//
// Mutaciones a las que se enfrenta, las tres probadas:
//   - debilitar o borrar la guarda len(r.MessageIDs) > 0: colapsaría el valor cero en un
//     falso-positivo Entregado y la primera aserción (ok=false) se pondría roja.
//   - invertir la rama Delivered/Read: lo cubre el par de tests de arriba.
func TestClasificarAcuseRechazaElValorCeroFrenteAEntregado(t *testing.T) {
	t.Parallel()

	// La trampa existe de verdad: ambos comparten Type=="".
	if types.ReceiptTypeDelivered != "" {
		t.Fatalf("premisa rota: ReceiptTypeDelivered = %q, se esperaba la cadena vacía", types.ReceiptTypeDelivered)
	}
	var cero events.Receipt
	if cero.Type != types.ReceiptTypeDelivered {
		t.Fatalf("premisa rota: el valor cero tiene Type=%q, se esperaba %q", cero.Type, types.ReceiptTypeDelivered)
	}

	// Un struct de valor cero (Type=="", MessageIDs==nil) NO es un acuse genuino.
	if estado, ok := clasificarAcuse(&cero); ok {
		t.Fatalf("el valor cero fue aceptado como acuse con estado %q", estado)
	}

	// Un acuse entregado explícito CON MessageIDs sí lo es.
	estado, ok := clasificarAcuse(recepcionDePrueba(types.ReceiptTypeDelivered, "whatsmeow-id-3"))
	if !ok || estado != ipc.EstadoEnvioEntregado {
		t.Fatalf("acuse entregado explícito = (%q, %v), se esperaba (%q, true)", estado, ok, ipc.EstadoEnvioEntregado)
	}
}

// TestClasificarAcuseRechazaTodoTipoQueNoSeaEntregadoNiLeido cubre la otra mitad de AC-3: la
// comprobación de Type sigue viva y no queda subsumida por la de MessageIDs. Un Receipt con un tipo
// excluido Y MessageIDs no vacíos devuelve ok=false, y nunca se inventa vocabulario nuevo.
func TestClasificarAcuseRechazaTodoTipoQueNoSeaEntregadoNiLeido(t *testing.T) {
	t.Parallel()

	excluidos := []types.ReceiptType{
		types.ReceiptTypeSender,
		types.ReceiptTypeRetry,
		types.ReceiptTypeReadSelf,
		types.ReceiptTypePlayed,
		types.ReceiptTypePlayedSelf,
		types.ReceiptTypeServerError,
		types.ReceiptTypeInactive,
		types.ReceiptTypePeerMsg,
		types.ReceiptTypeHistorySync,
	}
	for _, tipo := range excluidos {
		tipo := tipo
		t.Run(string(tipo), func(t *testing.T) {
			estado, ok := clasificarAcuse(recepcionDePrueba(tipo, "whatsmeow-id-excluido"))
			if ok {
				t.Fatalf("tipo %q con MessageIDs fue clasificado como %q", tipo, estado)
			}
		})
	}
}

// TestManejarEventoDeAcuseEntregaAlSumideroSinEmitirIpc cubre AC-1 y AC-2 en la mitad del sumidero:
// un acuse clasificable llega al sumidero con IdCorrelacion igual al MessageID del Receipt, el
// estado correcto y la marca temporal. La no-emisión IPC es estructural (ningún archivo de este
// conjunto puede alcanzar el socket), no algo que un test pueda afirmar aquí.
func TestManejarEventoDeAcuseEntregaAlSumideroSinEmitirIpc(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre          string
		tipo            types.ReceiptType
		estadoEsperado  string
		idCorrelacion   string
		marcaEsperadaMs int64
	}{
		{
			nombre:          "entregado",
			tipo:            types.ReceiptTypeDelivered,
			estadoEsperado:  ipc.EstadoEnvioEntregado,
			idCorrelacion:   "whatsmeow-id-entregado",
			marcaEsperadaMs: 1_786_083_200_000,
		},
		{
			nombre:          "leido",
			tipo:            types.ReceiptTypeRead,
			estadoEsperado:  ipc.EstadoEnvioLeido,
			idCorrelacion:   "whatsmeow-id-leido",
			marcaEsperadaMs: 1_786_083_200_000,
		},
	}

	for _, caso := range casos {
		caso := caso
		t.Run(caso.nombre, func(t *testing.T) {
			var recibidos []Acuse
			sumidero := func(acuse Acuse) {
				recibidos = append(recibidos, acuse)
			}

			manejarEventoDeAcuse(recepcionDePrueba(caso.tipo, caso.idCorrelacion), sumidero, nil)

			if len(recibidos) != 1 {
				t.Fatalf("acuses recibidos = %d, se esperaba 1", len(recibidos))
			}
			acuse := recibidos[0]
			if acuse.IdCorrelacion != caso.idCorrelacion {
				t.Fatalf("IdCorrelacion = %q, se esperaba %q", acuse.IdCorrelacion, caso.idCorrelacion)
			}
			if acuse.Estado != caso.estadoEsperado {
				t.Fatalf("Estado = %q, se esperaba %q", acuse.Estado, caso.estadoEsperado)
			}
			if acuse.MarcaTemporalMs != caso.marcaEsperadaMs {
				t.Fatalf("MarcaTemporalMs = %d, se esperaba %d", acuse.MarcaTemporalMs, caso.marcaEsperadaMs)
			}
		})
	}
}

// TestManejarEventoDeAcuseIgnoraEventosQueNoSonReceipt cubre la seguridad del type-assert: el
// manejador comparte el despacho no tipado de AddEventHandler con todo tipo de eventos, y un
// *events.Connected{} no debe provocar pánico ni invocar el sumidero.
//
// Mutación a la que se enfrenta: cambiar la aserción de tipo por un pase incondicional. Si
// manejarEventoDeAcuse no verificara evento.(*events.Receipt), clasificarAcuse recibiría un
// puntero a Connected y este test fallaría al ver invocado el sumidero.
func TestManejarEventoDeAcuseIgnoraEventosQueNoSonReceipt(t *testing.T) {
	t.Parallel()

	var llamadas int
	sumidero := func(Acuse) {
		llamadas++
	}

	manejarEventoDeAcuse(&events.Connected{}, sumidero, nil)

	if llamadas != 0 {
		t.Fatalf("el sumidero fue invocado %d veces con un evento que no es un Receipt", llamadas)
	}
}

// TestManejarEventoDeAcuseEsNilSeguroConSumideroNulo cubre la misma disciplina nil-segura de
// outbox.ColaDeSalida.ConSumideroDeAcuse: un acuse clasificable con sumidero nil no debe provocar
// pánico. Se ejercita llamando a manejarEventoDeAcuse directamente, sin un whatsmeow.Client real.
func TestManejarEventoDeAcuseEsNilSeguroConSumideroNulo(t *testing.T) {
	t.Parallel()

	manejarEventoDeAcuse(recepcionDePrueba(types.ReceiptTypeRead, "whatsmeow-id-nil"), nil, nil)
}
