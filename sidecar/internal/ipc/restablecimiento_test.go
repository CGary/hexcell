package ipc_test

import (
	"encoding/json"
	"errors"
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
)

const contactoDePrueba = "ct-0123456789abcdef0123456789abcdef"

func TestElParDeRestablecimientoDeContactoHaceIdaYVueltaConCadaCampo(t *testing.T) {
	t.Parallel()

	cuerpos := []ipc.Cuerpo{
		ipc.OrdenRestablecerContacto{Contacto: contactoDePrueba, IncluirBaja: ipc.ValorSi},
		ipc.OrdenRestablecerContacto{Contacto: contactoDePrueba, IncluirBaja: ipc.ValorNo},
		ipc.AcuseRestablecerContacto{
			Contacto: contactoDePrueba, IncluirBaja: ipc.ValorNo, Resultado: ipc.ResultadoRestablecimientoAplicado,
			Existe: ipc.ValorSi, Cortacircuitos: 4, PresentacionDeConversacion: 5, BajaDeContacto: 0,
		},
		ipc.AcuseRestablecerContacto{
			Contacto: contactoDePrueba, IncluirBaja: ipc.ValorSi, Resultado: ipc.ResultadoContactoDesconocido,
			Existe: ipc.ValorNo,
		},
		ipc.AcuseRestablecerContacto{
			Contacto: contactoDePrueba, IncluirBaja: ipc.ValorNo, Resultado: ipc.ResultadoFallido,
			Existe: ipc.ValorNo, Motivo: "almacén de identidad no disponible",
		},
	}
	for indice, cuerpo := range cuerpos {
		linea, err := ipc.Codificar(ipc.NuevoSobre(cuerpo))
		if err != nil {
			t.Fatalf("caso %d: Codificar: %v", indice, err)
		}
		sobre, err := ipc.Decodificar(linea)
		if err != nil {
			t.Fatalf("caso %d: Decodificar: %v", indice, err)
		}
		if !reflect.DeepEqual(sobre.Cuerpo, cuerpo) {
			t.Errorf("caso %d: cuerpo tras la ida y vuelta = %#v, se esperaba %#v", indice, sobre.Cuerpo, cuerpo)
		}
	}
}

func TestLosCamposDelParDeRestablecimientoSalenEnElOrdenDeclarado(t *testing.T) {
	t.Parallel()

	esperados := map[ipc.TipoMensaje][]string{
		ipc.TipoOrdenRestablecerContacto: {"version", "tipo", "contacto", "incluir_baja"},
		ipc.TipoAcuseRestablecerContacto: {
			"version", "tipo", "contacto", "incluir_baja", "resultado", "existe",
			"cortacircuitos", "presentacion_de_conversacion", "baja_de_contacto", "motivo",
		},
	}
	for tipo, campos := range esperados {
		if obtenidos := ipc.CamposDe(tipo); !reflect.DeepEqual(obtenidos, campos) {
			t.Errorf("campos de %q = %v, se esperaba %v", tipo, obtenidos, campos)
		}
	}
}

func TestElConjuntoCerradoDeTiposTieneDiecinueveConElParDeRestablecimiento(t *testing.T) {
	t.Parallel()

	tipos := ipc.TiposDeclarados()
	if len(tipos) != 19 {
		t.Fatalf("hay %d tipos declarados, se esperaban 19", len(tipos))
	}
	presentes := map[ipc.TipoMensaje]bool{}
	for _, tipo := range tipos {
		presentes[tipo] = true
	}
	for _, nombre := range []string{"orden_restablecer_contacto", "acuse_restablecer_contacto"} {
		if !presentes[ipc.TipoMensaje(nombre)] {
			t.Errorf("TiposDeclarados no contiene %q", nombre)
		}
	}
}

func TestDecodificarRechazaLosParesDeRestablecimientoMalformados(t *testing.T) {
	t.Parallel()

	casos := []struct {
		nombre   string
		linea    string
		esperado error
	}{
		{
			"orden con un campo de más",
			`{"version":7,"tipo":"orden_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":"no","extra":"x"}`,
			ipc.ErrCampoDesconocido,
		},
		{
			"orden sin incluir_baja",
			`{"version":7,"tipo":"orden_restablecer_contacto","contacto":"` + contactoDePrueba + `"}`,
			ipc.ErrCampoAusente,
		},
		{
			"orden con incluir_baja booleano",
			`{"version":7,"tipo":"orden_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":true}`,
			ipc.ErrValorNoEscalar,
		},
		{
			"orden con incluir_baja nulo",
			`{"version":7,"tipo":"orden_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":null}`,
			ipc.ErrValorNoEscalar,
		},
		{
			"acuse con existe booleano",
			`{"version":7,"tipo":"acuse_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":"no","resultado":"aplicado","existe":false,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0,"motivo":""}`,
			ipc.ErrValorNoEscalar,
		},
		{
			"acuse con existe nulo",
			`{"version":7,"tipo":"acuse_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":"no","resultado":"aplicado","existe":null,"cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0,"motivo":""}`,
			ipc.ErrValorNoEscalar,
		},
		{
			"acuse sin motivo",
			`{"version":7,"tipo":"acuse_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":"no","resultado":"aplicado","existe":"si","cortacircuitos":0,"presentacion_de_conversacion":0,"baja_de_contacto":0}`,
			ipc.ErrCampoAusente,
		},
		{
			"acuse con contador decimal",
			`{"version":7,"tipo":"acuse_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":"no","resultado":"aplicado","existe":"si","cortacircuitos":1.5,"presentacion_de_conversacion":0,"baja_de_contacto":0,"motivo":""}`,
			ipc.ErrValorNoEscalar,
		},
		{
			"orden con la versión anterior",
			`{"version":6,"tipo":"orden_restablecer_contacto","contacto":"` + contactoDePrueba + `","incluir_baja":"no"}`,
			ipc.ErrVersionIncompatible,
		},
	}
	for _, caso := range casos {
		t.Run(caso.nombre, func(t *testing.T) {
			t.Parallel()
			_, err := ipc.Decodificar([]byte(caso.linea + "\n"))
			if !errors.Is(err, caso.esperado) {
				t.Fatalf("error = %v, se esperaba %v", err, caso.esperado)
			}
		})
	}
}

var (
	cabeceraDeVersionDelDocumento = regexp.MustCompile(`\*\*Versión de este protocolo:\*\* (\d+\.\d+), fijada el \d{4}-\d{2}-\d{2}\.`)
	filaDeCorrespondencia         = regexp.MustCompile(`(?m)^\| (\d+\.\d+) \| ` + "`" + `(\d+)` + "`" + ` \|$`)
)

// versionDeCableSegunElDocumento lee la versión del documento de su cabecera y el número de cable
// que la tabla de correspondencia le asigna. Falla si la fila de esa versión no existe.
func versionDeCableSegunElDocumento(t *testing.T, documento string) (string, int64) {
	t.Helper()
	cabecera := cabeceraDeVersionDelDocumento.FindStringSubmatch(documento)
	if cabecera == nil {
		t.Fatal("el documento no lleva la cabecera de versión con fecha absoluta")
	}
	for _, fila := range filaDeCorrespondencia.FindAllStringSubmatch(documento, -1) {
		if fila[1] != cabecera[1] {
			continue
		}
		cable, err := strconv.ParseInt(fila[2], 10, 64)
		if err != nil {
			t.Fatalf("número de cable ilegible en la fila %q: %v", fila[0], err)
		}
		return cabecera[1], cable
	}
	t.Fatalf("la tabla de correspondencia no tiene la fila de la versión %s del documento", cabecera[1])
	return "", 0
}

func TestLaVersionDeCableDelSobreCoincideConElDocumentoYEsSiete(t *testing.T) {
	t.Parallel()

	documento := leerDocumento(t)
	versionDelDocumento, cableSegunElDocumento := versionDeCableSegunElDocumento(t, documento)
	if versionDelDocumento != "1.7" {
		t.Errorf("versión del documento = %s, se esperaba 1.7", versionDelDocumento)
	}
	if !strings.Contains(documento, "| 1.7 | `7` |") {
		t.Errorf("el documento no declara la correspondencia 1.7 → cable 7")
	}

	linea, err := ipc.Codificar(ipc.NuevoSobre(ipc.OrdenRestablecerContacto{Contacto: contactoDePrueba, IncluirBaja: ipc.ValorNo}))
	if err != nil {
		t.Fatalf("Codificar: %v", err)
	}
	var crudo map[string]json.RawMessage
	if err := json.Unmarshal(linea, &crudo); err != nil {
		t.Fatalf("la línea codificada no es JSON: %v", err)
	}
	cableEnElSobre, err := strconv.ParseInt(string(crudo["version"]), 10, 64)
	if err != nil {
		t.Fatalf("el campo version del sobre codificado no es un entero: %v", err)
	}

	if cableEnElSobre != cableSegunElDocumento {
		t.Errorf("version del sobre codificado = %d, el documento asigna %d", cableEnElSobre, cableSegunElDocumento)
	}
	if cableEnElSobre != 7 {
		t.Errorf("version del sobre codificado = %d, se esperaba 7", cableEnElSobre)
	}
	if cableSegunElDocumento != 7 {
		t.Errorf("cable según el documento = %d, se esperaba 7", cableSegunElDocumento)
	}
}
