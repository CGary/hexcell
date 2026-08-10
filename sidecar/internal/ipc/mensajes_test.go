package ipc_test

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
)

// cuerposDeMuestra devuelve un cuerpo poblado por cada uno de los nueve tipos declarados.
func cuerposDeMuestra() map[ipc.TipoMensaje]ipc.Cuerpo {
	return map[ipc.TipoMensaje]ipc.Cuerpo{
		ipc.TipoSaludo: ipc.Saludo{
			Emisor:   ipc.EmisorSidecar,
			IdCelula: "piloto-01",
		},
		ipc.TipoEventoEntrante: ipc.EventoEntrante{
			IdDeduplicacion: "dedup-7",
			IdConversacion:  "conv-3",
			IdRemitente:     "rem-9",
			Contenido:       `hola "mundo" \ y un salto`,
			MarcaTemporalMs: 1_767_225_600_000,
		},
		ipc.TipoConfirmacion: ipc.Confirmacion{
			IdDeduplicacion: "dedup-7",
		},
		ipc.TipoEstadoSesion: ipc.EstadoSesion{
			Estado:     ipc.EstadoReconectando,
			Causa:      "fallo_de_conexion",
			Codigo:     405,
			ExpiraEnMs: 0,
		},
		ipc.TipoOrdenRespaldoSqlstore: ipc.OrdenRespaldoSqlstore{
			Orden:                ipc.OrdenRespaldarSqlstore,
			Destino:              "/respaldos/ronda-42",
			IdentificadorDeRonda: "ronda-42",
		},
		ipc.TipoAcuseRespaldoSqlstore: ipc.AcuseRespaldoSqlstore{
			IdentificadorDeRonda: "ronda-42",
			Resultado:            ipc.ResultadoCompletado,
			RutaDeLaCopia:        "/respaldos/ronda-42/sqlstore.db",
			Bytes:                262144,
			Motivo:               "",
		},
		ipc.TipoOrdenEmparejar: ipc.OrdenEmparejar{
			Metodo: ipc.MetodoQr,
		},
		ipc.TipoCodigoEmparejamiento: ipc.CodigoEmparejamiento{
			Metodo:     ipc.MetodoQr,
			Valor:      "2@abc123,defgh456,ijklmn789",
			ExpiraEnMs: 1_722_816_000_000,
		},
		ipc.TipoAcuseEmparejamiento: ipc.AcuseEmparejamiento{
			Resultado: ipc.ResultadoEmparejamientoCompletado,
			Motivo:    "",
		},
		ipc.TipoMensajeSaliente: ipc.MensajeSaliente{
			IdMensaje:             "msg-123",
			IdConversacion:        "conv-456",
			Contenido:             "texto de prueba",
			MarcaTemporalOrigenMs: 1_767_225_600_000,
		},
		ipc.TipoAcuseEnvio: ipc.AcuseEnvio{
			IdMensaje:       "msg-123",
			Estado:          ipc.EstadoEnvioEntregado,
			IdCorrelacion:   "corr-789",
			Motivo:          "",
			MarcaTemporalMs: 1_767_225_605_000,
		},
	}
}

func TestIdaYVueltaDeTodosLosTiposDeclarados(t *testing.T) {
	t.Parallel()

	muestras := cuerposDeMuestra()
	if len(muestras) != len(ipc.TiposDeclarados()) {
		t.Fatalf("hay %d muestras para %d tipos declarados", len(muestras), len(ipc.TiposDeclarados()))
	}

	for _, tipo := range ipc.TiposDeclarados() {
		cuerpo, hayMuestra := muestras[tipo]
		if !hayMuestra {
			t.Fatalf("falta la muestra del tipo %q", tipo)
		}

		linea, err := ipc.Codificar(ipc.NuevoSobre(cuerpo))
		if err != nil {
			t.Fatalf("Codificar(%q) devolvió error: %v", tipo, err)
		}

		sobre, err := ipc.Decodificar(linea)
		if err != nil {
			t.Fatalf("Decodificar(%q) devolvió error: %v", tipo, err)
		}
		if sobre.Version != ipc.VersionProtocolo {
			t.Errorf("versión de %q = %d", tipo, sobre.Version)
		}
		if sobre.Tipo != tipo {
			t.Errorf("tipo decodificado = %q, se esperaba %q", sobre.Tipo, tipo)
		}
		if sobre.Cuerpo != cuerpo {
			t.Errorf("cuerpo de %q tras la ida y vuelta = %#v, se esperaba %#v", tipo, sobre.Cuerpo, cuerpo)
		}
	}
}

func TestCodificarProduceUnObjetoPlanoDeProfundidadUnoPorLinea(t *testing.T) {
	t.Parallel()

	for _, tipo := range ipc.TiposDeclarados() {
		linea, err := ipc.Codificar(ipc.NuevoSobre(cuerposDeMuestra()[tipo]))
		if err != nil {
			t.Fatalf("Codificar(%q): %v", tipo, err)
		}

		if !bytes.HasSuffix(linea, []byte("\n")) {
			t.Errorf("la línea de %q no termina en salto de línea", tipo)
		}
		if bytes.Count(linea, []byte("\n")) != 1 {
			t.Errorf("la línea de %q contiene más de un salto de línea", tipo)
		}

		decodificador := json.NewDecoder(bytes.NewReader(linea))
		decodificador.UseNumber()
		var objeto map[string]any
		if err := decodificador.Decode(&objeto); err != nil {
			t.Fatalf("la línea de %q no es un objeto JSON: %v", tipo, err)
		}
		for nombre, valor := range objeto {
			switch valor.(type) {
			case string, json.Number:
			default:
				t.Errorf("el campo %q de %q no es cadena ni entero: %T", nombre, tipo, valor)
			}
		}

		// Los campos salen en el orden declarado, `version` y `tipo` los primeros.
		esperados := ipc.CamposDe(tipo)
		if len(objeto) != len(esperados) {
			t.Errorf("%q trae %d campos, se declararon %d", tipo, len(objeto), len(esperados))
		}
		posicion := 0
		texto := string(linea)
		for _, nombre := range esperados {
			indice := strings.Index(texto, `"`+nombre+`":`)
			if indice < posicion {
				t.Errorf("el campo %q de %q no aparece en el orden declarado", nombre, tipo)
			}
			posicion = indice
		}
	}
}

func TestDecodificarRechazaUnaVersionIncompatible(t *testing.T) {
	t.Parallel()

	linea := []byte(`{"version":1,"tipo":"confirmacion","id_deduplicacion":"dedup-7"}` + "\n")
	sobre, err := ipc.Decodificar(linea)
	if !errors.Is(err, ipc.ErrVersionIncompatible) {
		t.Fatalf("error = %v, se esperaba ErrVersionIncompatible", err)
	}
	if sobre.Cuerpo != nil {
		t.Errorf("se devolvió un sobre poblado a medias: %#v", sobre)
	}
}

func TestDecodificarRechazaUnTipoDesconocido(t *testing.T) {
	t.Parallel()

	linea := []byte(`{"version":4,"tipo":"tipo_inexistente","texto":"hola"}` + "\n")
	if _, err := ipc.Decodificar(linea); !errors.Is(err, ipc.ErrTipoDesconocido) {
		t.Fatalf("error = %v, se esperaba ErrTipoDesconocido", err)
	}
}

func TestDecodificarRechazaLineasMalformadasSinEntrarEnPanico(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre   string
		linea    string
		esperado error
	}{
		{"vacía", "", ipc.ErrLineaVacia},
		{"solo salto de línea", "\n", ipc.ErrLineaVacia},
		{"no es JSON", "esto no es json\n", ipc.ErrLineaMalformada},
		{"objeto sin cerrar", `{"version":4,"tipo":"saludo"` + "\n", ipc.ErrLineaMalformada},
		{"no es un objeto", `["version",2]` + "\n", ipc.ErrLineaMalformada},
		{"dos objetos en la línea", `{"version":4,"tipo":"saludo","emisor":"nucleo","id_celula":"c"} {"version":4}` + "\n", ipc.ErrLineaMalformada},
		{"valor anidado", `{"version":4,"tipo":"saludo","emisor":{"quien":"nucleo"},"id_celula":"c"}` + "\n", ipc.ErrValorNoEscalar},
		{"valor en lista", `{"version":4,"tipo":"saludo","emisor":["nucleo"],"id_celula":"c"}` + "\n", ipc.ErrValorNoEscalar},
		{"valor booleano", `{"version":4,"tipo":"saludo","emisor":true,"id_celula":"c"}` + "\n", ipc.ErrValorNoEscalar},
		{"valor nulo", `{"version":4,"tipo":"saludo","emisor":null,"id_celula":"c"}` + "\n", ipc.ErrValorNoEscalar},
		{"entero con coma", `{"version":4,"tipo":"confirmacion","id_deduplicacion":"d","extra":1.5}` + "\n", ipc.ErrValorNoEscalar},
		{"campo ausente", `{"version":4,"tipo":"saludo","emisor":"nucleo"}` + "\n", ipc.ErrCampoAusente},
		{"campo desconocido", `{"version":4,"tipo":"confirmacion","id_deduplicacion":"d","secuencia":7}` + "\n", ipc.ErrCampoDesconocido},
	}

	for _, caso := range casos {
		t.Run(caso.nombre, func(t *testing.T) {
			if _, err := ipc.Decodificar([]byte(caso.linea)); !errors.Is(err, caso.esperado) {
				t.Fatalf("error = %v, se esperaba %v", err, caso.esperado)
			}
		})
	}
}

func TestElEventoEntranteNoTransportaNingunIdentificadorDeTransporte(t *testing.T) {
	t.Parallel()

	campos := ipc.CamposDe(ipc.TipoEventoEntrante)
	esperados := []string{
		"version", "tipo", "id_deduplicacion", "id_conversacion",
		"id_remitente", "contenido", "marca_temporal_ms",
	}
	if strings.Join(campos, ",") != strings.Join(esperados, ",") {
		t.Fatalf("conjunto de campos = %v, se esperaba %v", campos, esperados)
	}

	// El conjunto cerrado es lo que hace verificable que el JID no cruza el puerto: ninguna de
	// estas subcadenas puede aparecer en un nombre de campo de ninguno de los nueve tipos.
	prohibidas := []string{"jid", "telefono", "phone", "dispositivo", "device", "numero"}
	for _, tipo := range ipc.TiposDeclarados() {
		for _, nombre := range ipc.CamposDe(tipo) {
			for _, prohibida := range prohibidas {
				if strings.Contains(nombre, prohibida) {
					t.Errorf("el tipo %q declara el campo %q, que huele a identificador de transporte", tipo, nombre)
				}
			}
		}
	}
}

func TestElRespaldoDelSqlstoreEncajaConElContratoDeLaEtapaA2(t *testing.T) {
	t.Parallel()

	orden := ipc.CamposDe(ipc.TipoOrdenRespaldoSqlstore)
	esperadosOrden := []string{"version", "tipo", "orden", "destino", "identificador_de_ronda"}
	if strings.Join(orden, ",") != strings.Join(esperadosOrden, ",") {
		t.Errorf("campos de la orden = %v, se esperaba %v", orden, esperadosOrden)
	}

	acuse := ipc.CamposDe(ipc.TipoAcuseRespaldoSqlstore)
	esperadosAcuse := []string{
		"version", "tipo", "identificador_de_ronda", "resultado",
		"ruta_de_la_copia", "bytes", "motivo",
	}
	if strings.Join(acuse, ",") != strings.Join(esperadosAcuse, ",") {
		t.Errorf("campos del acuse = %v, se esperaba %v", acuse, esperadosAcuse)
	}

	if ipc.OrdenRespaldarSqlstore != "respaldar_sqlstore" {
		t.Errorf("la cadena fija de la orden = %q", ipc.OrdenRespaldarSqlstore)
	}

	// Un acuse fallido codifica la ausencia con el valor vacío, no omitiendo el campo: la
	// condicionalidad del contrato de A-2 se conserva sin campos opcionales en el cable.
	linea, err := ipc.Codificar(ipc.NuevoSobre(ipc.AcuseRespaldoSqlstore{
		IdentificadorDeRonda: "ronda-42",
		Resultado:            ipc.ResultadoFallido,
		Motivo:               "el destino no existe",
	}))
	if err != nil {
		t.Fatalf("Codificar: %v", err)
	}
	if !strings.Contains(string(linea), `"ruta_de_la_copia":""`) {
		t.Errorf("el acuse fallido omite ruta_de_la_copia en vez de vaciarla: %s", linea)
	}
	if !strings.Contains(string(linea), `"bytes":0`) {
		t.Errorf("el acuse fallido omite bytes en vez de ponerlo a cero: %s", linea)
	}
}

func TestCodificarRechazaUnSobreIncoherenteOSinCuerpo(t *testing.T) {
	t.Parallel()

	if _, err := ipc.Codificar(ipc.Sobre{Version: ipc.VersionProtocolo, Tipo: ipc.TipoSaludo}); !errors.Is(err, ipc.ErrCuerpoNoEspecificado) {
		t.Errorf("error = %v, se esperaba ErrCuerpoNoEspecificado", err)
	}

	incoherente := ipc.Sobre{
		Version: ipc.VersionProtocolo,
		Tipo:    ipc.TipoConfirmacion,
		Cuerpo:  ipc.Saludo{Emisor: ipc.EmisorNucleo, IdCelula: "piloto-01"},
	}
	if _, err := ipc.Codificar(incoherente); !errors.Is(err, ipc.ErrCuerpoIncoherente) {
		t.Errorf("error = %v, se esperaba ErrCuerpoIncoherente", err)
	}
}

func TestElLimiteDeLineaSeAplicaEnLasDosDirecciones(t *testing.T) {
	t.Parallel()

	desmesurado := ipc.EventoEntrante{
		IdDeduplicacion: "dedup-1",
		IdConversacion:  "conv-1",
		IdRemitente:     "rem-1",
		Contenido:       strings.Repeat("a", ipc.LongitudMaximaDeLinea),
		MarcaTemporalMs: 1,
	}
	if _, err := ipc.Codificar(ipc.NuevoSobre(desmesurado)); !errors.Is(err, ipc.ErrLineaDemasiadoLarga) {
		t.Errorf("Codificar devolvió %v, se esperaba ErrLineaDemasiadoLarga", err)
	}

	larga := append(bytes.Repeat([]byte("x"), ipc.LongitudMaximaDeLinea+1), '\n')
	if _, err := ipc.Decodificar(larga); !errors.Is(err, ipc.ErrLineaDemasiadoLarga) {
		t.Errorf("Decodificar devolvió %v, se esperaba ErrLineaDemasiadoLarga", err)
	}
}

func TestCausasDeclaradas_OchoElementosEnOrdenAlfabetico(t *testing.T) {
	t.Parallel()
	causas := ipc.CausasDeclaradas()
	if len(causas) != 8 {
		t.Fatalf("CausasDeclaradas() tiene %d elementos, se esperaban 8", len(causas))
	}
	for i := 1; i < len(causas); i++ {
		if causas[i] < causas[i-1] {
			t.Fatalf("CausasDeclaradas() no está en orden: %q antes de %q", causas[i-1], causas[i])
		}
	}
}

func TestEstadosDeclarados_CuatroElementosEnOrdenAlfabetico(t *testing.T) {
	t.Parallel()
	estados := ipc.EstadosDeclarados()
	if len(estados) != 4 {
		t.Fatalf("EstadosDeclarados() tiene %d elementos, se esperaban 4", len(estados))
	}
	for i := 1; i < len(estados); i++ {
		if estados[i] < estados[i-1] {
			t.Fatalf("EstadosDeclarados() no está en orden: %q antes de %q", estados[i-1], estados[i])
		}
	}
}

func TestEstadosDeEnvioDeclarados_CuatroElementosEnOrdenAlfabetico(t *testing.T) {
	t.Parallel()
	estados := ipc.EstadosDeEnvioDeclarados()
	if len(estados) != 4 {
		t.Fatalf("EstadosDeEnvioDeclarados() tiene %d elementos, se esperaban 4", len(estados))
	}
	for i := 1; i < len(estados); i++ {
		if estados[i] < estados[i-1] {
			t.Fatalf("EstadosDeEnvioDeclarados() no está en orden: %q antes de %q", estados[i-1], estados[i])
		}
	}
}
