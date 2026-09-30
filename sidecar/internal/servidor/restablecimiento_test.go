package servidor_test

import (
	"bufio"
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"github.com/CGary/hexcell/sidecar/internal/servidor"
	"go.mau.fi/whatsmeow/types"
	_ "modernc.org/sqlite"
)

const plazoDeRestablecimiento = 10 * time.Second

// escenarioDeRestablecimiento levanta un Servidor real sobre un socket temporal, con un
// identidad.Almacen real sobre un identidad.db temporal y una conexión lateral para sembrar y
// contar filas sin pasar por el almacén.
type escenarioDeRestablecimiento struct {
	almacen *identidad.Almacen
	lateral *sql.DB
	conn    net.Conn
	lector  *bufio.Reader
	log     *bufferSeguro
}

func nuevoEscenarioDeRestablecimiento(t *testing.T, conAlmacen bool) *escenarioDeRestablecimiento {
	t.Helper()
	log := &bufferSeguro{}
	reg := registro.Nuevo(log, slog.LevelInfo, "test-cell")

	rutaDB := filepath.Join(t.TempDir(), "identidad.db")
	almacen, err := identidad.Abrir(identidad.Opciones{Ruta: rutaDB, Registro: registro.Nuevo(io.Discard, slog.LevelInfo, "test-cell")})
	if err != nil {
		t.Fatalf("abrir el almacén de identidad: %v", err)
	}
	t.Cleanup(func() { _ = almacen.Cerrar() })
	lateral, err := sql.Open("sqlite", fmt.Sprintf("file:%s?_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)", rutaDB))
	if err != nil {
		t.Fatalf("abrir la conexión lateral: %v", err)
	}
	t.Cleanup(func() { _ = lateral.Close() })

	deps := servidor.Dependencias{
		RutaSocket: filepath.Join(t.TempDir(), "ipc.sock"),
		IdCelula:   "test-cell",
		Registro:   reg,
	}
	if conAlmacen {
		deps.AlmacenIdentidad = almacen
	}
	srv := servidor.NuevoServidor(deps)
	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al escuchar: %v", err)
	}
	t.Cleanup(func() { _ = srv.Cerrar() })
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	go srv.Aceptar(ctx)

	conn, err := net.Dial("unix", deps.RutaSocket)
	if err != nil {
		t.Fatalf("error conectando cliente: %v", err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	e := &escenarioDeRestablecimiento{almacen: almacen, lateral: lateral, conn: conn, lector: bufio.NewReader(conn), log: log}

	saludo, _ := ipc.Codificar(ipc.NuevoSobre(ipc.Saludo{Emisor: ipc.EmisorNucleo, IdCelula: "test-cell"}))
	e.escribir(t, saludo)
	e.leerLinea(t)
	return e
}

func (e *escenarioDeRestablecimiento) escribir(t *testing.T, linea []byte) {
	t.Helper()
	_ = e.conn.SetWriteDeadline(time.Now().Add(plazoDeRestablecimiento))
	if _, err := e.conn.Write(linea); err != nil {
		t.Fatalf("error escribiendo en el socket: %v", err)
	}
}

func (e *escenarioDeRestablecimiento) leerLinea(t *testing.T) []byte {
	t.Helper()
	_ = e.conn.SetReadDeadline(time.Now().Add(plazoDeRestablecimiento))
	linea, err := e.lector.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo del socket: %v", err)
	}
	return linea
}

// restablecer envía la orden cruda (cadenas tal cual, sin validar) y devuelve el acuse decodificado.
func (e *escenarioDeRestablecimiento) restablecer(t *testing.T, contacto, incluirBaja string) ipc.AcuseRestablecerContacto {
	t.Helper()
	orden, err := ipc.Codificar(ipc.NuevoSobre(ipc.OrdenRestablecerContacto{Contacto: contacto, IncluirBaja: incluirBaja}))
	if err != nil {
		t.Fatalf("Codificar la orden: %v", err)
	}
	e.escribir(t, orden)
	sobre, err := ipc.Decodificar(e.leerLinea(t))
	if err != nil || sobre.Tipo != ipc.TipoAcuseRestablecerContacto {
		t.Fatalf("acuse inválido: %v, sobre=%+v", err, sobre)
	}
	acuse, ok := sobre.Cuerpo.(ipc.AcuseRestablecerContacto)
	if !ok {
		t.Fatalf("el cuerpo del acuse no es AcuseRestablecerContacto: %#v", sobre.Cuerpo)
	}
	return acuse
}

// contactoConFilas crea un contacto real y le siembra una fila en cada tabla que se restablece.
func (e *escenarioDeRestablecimiento) contactoConFilas(t *testing.T, usuario string) string {
	t.Helper()
	id := e.contactoSinFilas(t, usuario)
	for _, q := range []string{
		`INSERT INTO cortacircuitos (id_interno, repeticiones, ultimo_texto_normalizado) VALUES (?, 3, 'hola')`,
		`INSERT INTO presentacion_de_conversacion (id_interno) VALUES (?)`,
		`INSERT INTO baja_de_contacto (id_interno, dada_de_baja_en_ms) VALUES (?, 1000)`,
	} {
		if _, err := e.lateral.Exec(q, id); err != nil {
			t.Fatalf("sembrar %q: %v", q, err)
		}
	}
	return id
}

func (e *escenarioDeRestablecimiento) contactoSinFilas(t *testing.T, usuario string) string {
	t.Helper()
	ident, err := e.almacen.Resolver(context.Background(), identidad.Observacion{PN: types.JID{User: usuario, Server: types.DefaultUserServer}})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}
	return ident.IdInterno
}

func (e *escenarioDeRestablecimiento) contar(t *testing.T, tabla, id string) int64 {
	t.Helper()
	var n int64
	if err := e.lateral.QueryRow(fmt.Sprintf(`SELECT COUNT(*) FROM %s WHERE id_interno = ?`, tabla), id).Scan(&n); err != nil {
		t.Fatalf("contar %s: %v", tabla, err)
	}
	return n
}

// filas devuelve las cuentas (cortacircuitos, presentacion, baja) del contacto.
func (e *escenarioDeRestablecimiento) filas(t *testing.T, id string) [3]int64 {
	t.Helper()
	return [3]int64{
		e.contar(t, "cortacircuitos", id),
		e.contar(t, "presentacion_de_conversacion", id),
		e.contar(t, "baja_de_contacto", id),
	}
}

// eventos devuelve las líneas de registro cuyo evento es el dado.
func (e *escenarioDeRestablecimiento) eventos(t *testing.T, evento string) []map[string]any {
	t.Helper()
	var coincidencias []map[string]any
	for _, linea := range strings.Split(e.log.String(), "\n") {
		if strings.TrimSpace(linea) == "" {
			continue
		}
		var campos map[string]any
		if err := json.Unmarshal([]byte(linea), &campos); err != nil {
			t.Fatalf("línea de registro ilegible %q: %v", linea, err)
		}
		if campos[registro.CampoEvento] == evento {
			coincidencias = append(coincidencias, campos)
		}
	}
	return coincidencias
}

const (
	eventoRestablecido = "identidad.contacto_restablecido"
	eventoBajaRevivida = "identidad.baja_de_contacto_revivida"
	origenEsperado     = "origen=hexcell-admin contacto restablecer --incluir-baja --confirmar"
)

func TestOrdenRestablecerContactoSinBajaConservaLaBajaDelContacto(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t, true)
	a := e.contactoConFilas(t, "5491100002001")
	b := e.contactoConFilas(t, "5491100002002")

	acuse := e.restablecer(t, a, ipc.ValorNo)

	esperado := ipc.AcuseRestablecerContacto{
		Contacto: a, IncluirBaja: ipc.ValorNo, Resultado: ipc.ResultadoRestablecimientoAplicado, Existe: ipc.ValorSi,
		Cortacircuitos: 1, PresentacionDeConversacion: 1, BajaDeContacto: 0,
	}
	if acuse != esperado {
		t.Fatalf("acuse = %+v, se esperaba %+v", acuse, esperado)
	}
	if obtenidas := e.filas(t, a); obtenidas != [3]int64{0, 0, 1} {
		t.Fatalf("filas del contacto restablecido = %v, se esperaba [0 0 1]", obtenidas)
	}
	if obtenidas := e.filas(t, b); obtenidas != [3]int64{1, 1, 1} {
		t.Fatalf("filas del otro contacto = %v, se esperaba [1 1 1]", obtenidas)
	}
}

func TestOrdenRestablecerContactoConBajaLaBorra(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t, true)
	a := e.contactoConFilas(t, "5491100002011")
	b := e.contactoConFilas(t, "5491100002012")

	acuse := e.restablecer(t, a, ipc.ValorSi)

	esperado := ipc.AcuseRestablecerContacto{
		Contacto: a, IncluirBaja: ipc.ValorSi, Resultado: ipc.ResultadoRestablecimientoAplicado, Existe: ipc.ValorSi,
		Cortacircuitos: 1, PresentacionDeConversacion: 1, BajaDeContacto: 1,
	}
	if acuse != esperado {
		t.Fatalf("acuse = %+v, se esperaba %+v", acuse, esperado)
	}
	if obtenidas := e.filas(t, a); obtenidas != [3]int64{0, 0, 0} {
		t.Fatalf("filas del contacto restablecido = %v, se esperaba [0 0 0]", obtenidas)
	}
	if obtenidas := e.filas(t, b); obtenidas != [3]int64{1, 1, 1} {
		t.Fatalf("filas del otro contacto = %v, se esperaba [1 1 1]", obtenidas)
	}
}

func TestOrdenRestablecerContactoFallaCerradoSinTocarFilas(t *testing.T) {
	t.Parallel()
	casos := []struct {
		nombre      string
		conAlmacen  bool
		contacto    func(a string) string
		incluirBaja string
	}{
		{"almacén ausente", false, func(a string) string { return a }, ipc.ValorSi},
		{"contacto vacío", true, func(string) string { return "" }, ipc.ValorSi},
		{"contacto sin prefijo", true, func(a string) string { return strings.TrimPrefix(a, "ct-") }, ipc.ValorSi},
		{"contacto en mayúsculas", true, func(a string) string { return strings.ToUpper(a) }, ipc.ValorSi},
		{"incluir_baja en mayúsculas", true, func(a string) string { return a }, "SI"},
		{"incluir_baja true", true, func(a string) string { return a }, "true"},
		{"incluir_baja uno", true, func(a string) string { return a }, "1"},
		{"incluir_baja vacío", true, func(a string) string { return a }, ""},
	}
	for _, caso := range casos {
		t.Run(caso.nombre, func(t *testing.T) {
			t.Parallel()
			e := nuevoEscenarioDeRestablecimiento(t, caso.conAlmacen)
			a := e.contactoConFilas(t, "5491100002021")

			acuse := e.restablecer(t, caso.contacto(a), caso.incluirBaja)

			if acuse.Resultado != ipc.ResultadoFallido {
				t.Fatalf("resultado = %q, se esperaba fallido (acuse %+v)", acuse.Resultado, acuse)
			}
			if acuse.Motivo == "" {
				t.Fatal("un acuse fallido debe llevar motivo")
			}
			if acuse.Existe != ipc.ValorNo || acuse.Cortacircuitos != 0 || acuse.PresentacionDeConversacion != 0 || acuse.BajaDeContacto != 0 {
				t.Fatalf("un fallo no debe informar existencia ni contadores: %+v", acuse)
			}
			if obtenidas := e.filas(t, a); obtenidas != [3]int64{1, 1, 1} {
				t.Fatalf("filas tras el rechazo = %v, se esperaba [1 1 1]", obtenidas)
			}
			if n := len(e.eventos(t, eventoRestablecido)); n != 0 {
				t.Fatalf("un rechazo no debe emitir el evento de restablecimiento, emitió %d", n)
			}
		})
	}
}

func TestOrdenRestablecerContactoDesconocidoYExistenteSinFilas(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t, true)
	otro := e.contactoConFilas(t, "5491100002031")
	vacio := e.contactoSinFilas(t, "5491100002032")
	const ausente = "ct-ffffffffffffffffffffffffffffffff"

	desconocido := e.restablecer(t, ausente, ipc.ValorSi)
	esperadoDesconocido := ipc.AcuseRestablecerContacto{Contacto: ausente, IncluirBaja: ipc.ValorSi, Resultado: ipc.ResultadoContactoDesconocido, Existe: ipc.ValorNo}
	if desconocido != esperadoDesconocido {
		t.Fatalf("acuse del desconocido = %+v, se esperaba %+v", desconocido, esperadoDesconocido)
	}
	if obtenidas := e.filas(t, otro); obtenidas != [3]int64{1, 1, 1} {
		t.Fatalf("filas de otro contacto tras un id desconocido = %v, se esperaba [1 1 1]", obtenidas)
	}
	// Un contacto desconocido con incluir_baja=si no revivió nada: ni aviso ni evento informativo.
	if n := len(e.eventos(t, eventoBajaRevivida)); n != 0 {
		t.Fatalf("un contacto desconocido no debe emitir el aviso de baja revivida, emitió %d", n)
	}
	if n := len(e.eventos(t, eventoRestablecido)); n != 0 {
		t.Fatalf("un contacto desconocido no debe emitir el evento de restablecimiento, emitió %d", n)
	}

	for repeticion := 1; repeticion <= 2; repeticion++ {
		existente := e.restablecer(t, vacio, ipc.ValorNo)
		esperadoExistente := ipc.AcuseRestablecerContacto{Contacto: vacio, IncluirBaja: ipc.ValorNo, Resultado: ipc.ResultadoRestablecimientoAplicado, Existe: ipc.ValorSi}
		if existente != esperadoExistente {
			t.Fatalf("ejecución %d: acuse del existente sin filas = %+v, se esperaba %+v", repeticion, existente, esperadoExistente)
		}
	}
}

func TestOrdenRestablecerContactoRegistraEventosSegunLaBaja(t *testing.T) {
	t.Parallel()
	casos := []struct {
		nombre         string
		sembrar        bool
		incluirBaja    string
		avisosEsperado int
	}{
		{"sin baja", true, ipc.ValorNo, 0},
		{"con baja y con fila de baja", true, ipc.ValorSi, 1},
		{"con baja y sin fila de baja", false, ipc.ValorSi, 0},
	}
	for _, caso := range casos {
		t.Run(caso.nombre, func(t *testing.T) {
			t.Parallel()
			e := nuevoEscenarioDeRestablecimiento(t, true)
			var contacto string
			if caso.sembrar {
				contacto = e.contactoConFilas(t, "5491100002041")
			} else {
				contacto = e.contactoSinFilas(t, "5491100002041")
			}

			acuse := e.restablecer(t, contacto, caso.incluirBaja)
			if acuse.Resultado != ipc.ResultadoRestablecimientoAplicado {
				t.Fatalf("acuse = %+v, se esperaba aplicado", acuse)
			}

			informativos := e.eventos(t, eventoRestablecido)
			if len(informativos) != 1 {
				t.Fatalf("eventos de restablecimiento = %d, se esperaba 1", len(informativos))
			}
			if informativos[0][registro.CampoIdEvento] != contacto {
				t.Errorf("el evento no lleva el id del contacto: %v", informativos[0])
			}
			if detalle, _ := informativos[0][registro.CampoDetalle].(string); !strings.Contains(detalle, "incluir_baja="+caso.incluirBaja) {
				t.Errorf("el evento no lleva el valor de incluir_baja %q: %v", caso.incluirBaja, informativos[0])
			}
			avisos := e.eventos(t, eventoBajaRevivida)
			if len(avisos) != caso.avisosEsperado {
				t.Fatalf("avisos de baja revivida = %d, se esperaban %d", len(avisos), caso.avisosEsperado)
			}
			if caso.avisosEsperado == 1 {
				if avisos[0][registro.CampoIdEvento] != contacto {
					t.Errorf("el aviso no lleva el id del contacto: %v", avisos[0])
				}
				if avisos[0][registro.CampoDetalle] != origenEsperado {
					t.Errorf("origen del aviso = %q, se esperaba %q", avisos[0][registro.CampoDetalle], origenEsperado)
				}
				if avisos[0][registro.CampoEvento] != eventoBajaRevivida {
					t.Errorf("evento del aviso = %q, se esperaba %q", avisos[0][registro.CampoEvento], eventoBajaRevivida)
				}
				if avisos[0][registro.CampoNivel] != "aviso" {
					t.Errorf("nivel del aviso = %v, se esperaba aviso", avisos[0][registro.CampoNivel])
				}
				if avisos[0][registro.CampoNivel] == informativos[0][registro.CampoNivel] {
					t.Errorf("el aviso de baja revivida debe ser más ruidoso que el evento informativo: %v", avisos[0])
				}
			}
		})
	}
}
