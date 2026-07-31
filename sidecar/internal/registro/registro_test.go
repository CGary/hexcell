package registro_test

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"sort"
	"strings"
	"testing"

	waLog "go.mau.fi/whatsmeow/util/log"

	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// decodificarLineas parte la salida capturada en líneas y decodifica cada una como objeto JSON.
func decodificarLineas(t *testing.T, salida string) []map[string]any {
	t.Helper()
	var objetos []map[string]any
	for _, linea := range strings.Split(strings.TrimRight(salida, "\n"), "\n") {
		if linea == "" {
			continue
		}
		var objeto map[string]any
		if err := json.Unmarshal([]byte(linea), &objeto); err != nil {
			t.Fatalf("la línea %q no es un objeto JSON válido: %v", linea, err)
		}
		objetos = append(objetos, objeto)
	}
	return objetos
}

func TestRegistroEmiteUnObjetoJsonPorLineaConElConjuntoCerradoDeAdr0019(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")
	reg.Info("evento_de_canal_recibido", registro.Campos{
		IdEvento:       "evt-1",
		IdConversacion: "conv-1",
		LatenciaMs:     12,
		Detalle:        "sesión construida",
	})
	reg.Aviso("socket_obsoleto_desenlazado", registro.Campos{})

	lineas := decodificarLineas(t, salida.String())
	if len(lineas) != 2 {
		t.Fatalf("se esperaban 2 líneas, salieron %d", len(lineas))
	}

	claves := make([]string, 0, len(lineas[0]))
	for clave := range lineas[0] {
		claves = append(claves, clave)
	}
	sort.Strings(claves)
	esperadas := []string{
		registro.CampoDetalle, registro.CampoEvento, registro.CampoIdCelula,
		registro.CampoIdConversacion, registro.CampoIdEvento, registro.CampoLatenciaMs,
		registro.CampoNivel,
	}
	sort.Strings(esperadas)
	if strings.Join(claves, ",") != strings.Join(esperadas, ",") {
		t.Errorf("conjunto de campos = %v, se esperaba %v", claves, esperadas)
	}

	if lineas[0][registro.CampoNivel] != "info" {
		t.Errorf("nivel = %v, se esperaba info", lineas[0][registro.CampoNivel])
	}
	if lineas[0][registro.CampoEvento] != "evento_de_canal_recibido" {
		t.Errorf("evento = %v", lineas[0][registro.CampoEvento])
	}
	if lineas[0][registro.CampoIdCelula] != "piloto-01" {
		t.Errorf("id_celula = %v", lineas[0][registro.CampoIdCelula])
	}
	if lineas[1][registro.CampoNivel] != "aviso" {
		t.Errorf("nivel de la segunda línea = %v, se esperaba aviso", lineas[1][registro.CampoNivel])
	}
	// La segunda entrada no puebla ningún campo opcional: solo quedan los tres obligatorios.
	if len(lineas[1]) != 3 {
		t.Errorf("la entrada sin campos opcionales tiene %d campos, se esperaban 3", len(lineas[1]))
	}
}

func TestRegistroEscapaComillasYBarrasInvertidas(t *testing.T) {
	t.Parallel()

	const detalle = `ruta "C:\\celula" con "comillas" y \ barra`
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")
	reg.Error("fallo_de_almacenamiento", registro.Campos{Detalle: detalle})

	crudo := salida.String()
	if !strings.Contains(crudo, `\"`) {
		t.Errorf("la línea cruda no escapa las comillas: %s", crudo)
	}
	if !strings.Contains(crudo, `\\`) {
		t.Errorf("la línea cruda no escapa las barras invertidas: %s", crudo)
	}

	lineas := decodificarLineas(t, crudo)
	if lineas[0][registro.CampoDetalle] != detalle {
		t.Errorf("detalle tras la ida y vuelta = %q, se esperaba %q", lineas[0][registro.CampoDetalle], detalle)
	}
}

func TestAdaptadorWaLogSatisfaceLaInterfazDeWhatsmeow(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")
	var registrador waLog.Logger = registro.NuevoAdaptadorWaLog(reg, "Cliente")

	registrador.Infof("conectado a %s", "servidor")
	registrador.Warnf("reintento %d", 2)
	registrador.Errorf("fallo: %v", "socket cerrado")

	lineas := decodificarLineas(t, salida.String())
	if len(lineas) != 3 {
		t.Fatalf("se esperaban 3 líneas, salieron %d", len(lineas))
	}
	for indice, nivel := range []string{"info", "aviso", "error"} {
		if lineas[indice][registro.CampoNivel] != nivel {
			t.Errorf("nivel de la línea %d = %v, se esperaba %s", indice, lineas[indice][registro.CampoNivel], nivel)
		}
		if lineas[indice][registro.CampoEvento] != registro.EventoWhatsmeow {
			t.Errorf("evento de la línea %d = %v, se esperaba %s", indice, lineas[indice][registro.CampoEvento], registro.EventoWhatsmeow)
		}
	}
	if detalle, _ := lineas[0][registro.CampoDetalle].(string); !strings.HasPrefix(detalle, "Cliente: ") {
		t.Errorf("el detalle no lleva el prefijo del módulo: %q", detalle)
	}
}

func TestAdaptadorWaLogDescartaLaDepuracionPorEncimaDelUmbral(t *testing.T) {
	t.Parallel()

	// Umbral por omisión del sidecar: info. La depuración de whatsmeow puede llevar contenido
	// de mensaje, así que no debe salir ni una línea.
	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")
	adaptador := registro.NuevoAdaptadorWaLog(reg, "Recv")
	adaptador.Debugf("nodo recibido: %s", "hola, quiero un turno")

	if salida.Len() != 0 {
		t.Fatalf("la depuración de whatsmeow llegó al registro: %s", salida.String())
	}

	// Con el umbral bajado a depuración, la línea sí sale: el corte es el umbral configurado,
	// no un descarte incondicional que ocultaría el diagnóstico cuando se pide de verdad.
	var salidaDepuracion bytes.Buffer
	regDepuracion := registro.Nuevo(&salidaDepuracion, slog.LevelDebug, "piloto-01")
	registro.NuevoAdaptadorWaLog(regDepuracion, "Recv").Debugf("nodo recibido: %s", "traza")

	lineas := decodificarLineas(t, salidaDepuracion.String())
	if len(lineas) != 1 {
		t.Fatalf("se esperaba 1 línea de depuración, salieron %d", len(lineas))
	}
	if lineas[0][registro.CampoNivel] != "depuracion" {
		t.Errorf("nivel = %v, se esperaba depuracion", lineas[0][registro.CampoNivel])
	}
}

func TestAdaptadorWaLogSubAcumulaLaRutaDeModulos(t *testing.T) {
	t.Parallel()

	var salida bytes.Buffer
	reg := registro.Nuevo(&salida, slog.LevelInfo, "piloto-01")
	raiz := registro.NuevoAdaptadorWaLog(reg, "Cliente")

	hijo, esAdaptador := raiz.Sub("Recv").(*registro.AdaptadorWaLog)
	if !esAdaptador {
		t.Fatalf("Sub no devolvió un *AdaptadorWaLog")
	}
	if hijo.Modulo() != "Cliente/Recv" {
		t.Errorf("módulo del hijo = %q, se esperaba %q", hijo.Modulo(), "Cliente/Recv")
	}

	sinRaiz := registro.NuevoAdaptadorWaLog(reg, "")
	nieto, _ := sinRaiz.Sub("AppState").(*registro.AdaptadorWaLog)
	if nieto.Modulo() != "AppState" {
		t.Errorf("módulo sin raíz = %q, se esperaba %q", nieto.Modulo(), "AppState")
	}
}
