package identidad_test

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"path/filepath"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	_ "modernc.org/sqlite"
)

// escenarioDeRestablecimiento es un identidad.db real y temporal con una segunda conexión
// independiente para sembrar y contar filas sin pasar por el almacén bajo prueba.
type escenarioDeRestablecimiento struct {
	almacen *identidad.Almacen
	lateral *sql.DB
}

func nuevoEscenarioDeRestablecimiento(t *testing.T) *escenarioDeRestablecimiento {
	t.Helper()
	ruta := filepath.Join(t.TempDir(), "identidad.db")
	almacen, err := identidad.Abrir(identidad.Opciones{
		Ruta:     ruta,
		Registro: registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test"),
	})
	if err != nil {
		t.Fatalf("Abrir: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })
	lateral, err := sql.Open("sqlite", fmt.Sprintf("file:%s?_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)", ruta))
	if err != nil {
		t.Fatalf("abrir la conexión lateral: %v", err)
	}
	t.Cleanup(func() { lateral.Close() })
	return &escenarioDeRestablecimiento{almacen: almacen, lateral: lateral}
}

// contactoConFilas crea un contacto (identidad y direccion) y le siembra una fila en cada una de
// las tres tablas que el restablecimiento toca.
func (e *escenarioDeRestablecimiento) contactoConFilas(t *testing.T, usuario string) string {
	t.Helper()
	id := e.contactoSinFilas(t, usuario)
	sembrar := []string{
		`INSERT INTO cortacircuitos (id_interno, repeticiones, ultimo_texto_normalizado) VALUES (?, 3, 'hola')`,
		`INSERT INTO presentacion_de_conversacion (id_interno) VALUES (?)`,
		`INSERT INTO baja_de_contacto (id_interno, dada_de_baja_en_ms) VALUES (?, 1000)`,
	}
	for _, q := range sembrar {
		if _, err := e.lateral.Exec(q, id); err != nil {
			t.Fatalf("sembrar %q: %v", q, err)
		}
	}
	return id
}

func (e *escenarioDeRestablecimiento) contactoSinFilas(t *testing.T, usuario string) string {
	t.Helper()
	return crearContactoPrueba(t, e.almacen, usuario)
}

func (e *escenarioDeRestablecimiento) contar(t *testing.T, tabla, id string) int64 {
	t.Helper()
	var n int64
	q := fmt.Sprintf(`SELECT COUNT(*) FROM %s WHERE id_interno = ?`, tabla)
	if err := e.lateral.QueryRow(q, id).Scan(&n); err != nil {
		t.Fatalf("contar %s: %v", tabla, err)
	}
	return n
}

type cuentas struct{ identidad, direccion, cortacircuitos, presentacion, baja int64 }

func (e *escenarioDeRestablecimiento) cuentasDe(t *testing.T, id string) cuentas {
	t.Helper()
	return cuentas{
		identidad:      e.contar(t, "identidad", id),
		direccion:      e.contar(t, "direccion", id),
		cortacircuitos: e.contar(t, "cortacircuitos", id),
		presentacion:   e.contar(t, "presentacion_de_conversacion", id),
		baja:           e.contar(t, "baja_de_contacto", id),
	}
}

func esperarCuentas(t *testing.T, nombre string, obtenidas, esperadas cuentas) {
	t.Helper()
	if obtenidas != esperadas {
		t.Fatalf("%s: filas por tabla = %+v, se esperaba %+v", nombre, obtenidas, esperadas)
	}
}

func TestRestablecerContacto_SinIncluirBajaConservaLaBaja(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t)
	a := e.contactoConFilas(t, "5491100001001")
	b := e.contactoConFilas(t, "5491100001002")

	r, err := e.almacen.RestablecerContacto(context.Background(), a, false)
	if err != nil {
		t.Fatalf("RestablecerContacto: %v", err)
	}
	if !r.Existe || r.BajaIncluida {
		t.Fatalf("Existe=%v BajaIncluida=%v, se esperaba true y false", r.Existe, r.BajaIncluida)
	}
	if r.Cortacircuitos != 1 || r.PresentacionDeConversacion != 1 || r.BajaDeContacto != 0 {
		t.Fatalf("contadores = %d, %d, %d; se esperaba 1, 1, 0", r.Cortacircuitos, r.PresentacionDeConversacion, r.BajaDeContacto)
	}
	esperarCuentas(t, "contacto restablecido", e.cuentasDe(t, a), cuentas{identidad: 1, direccion: 1, cortacircuitos: 0, presentacion: 0, baja: 1})
	esperarCuentas(t, "otro contacto", e.cuentasDe(t, b), cuentas{identidad: 1, direccion: 1, cortacircuitos: 1, presentacion: 1, baja: 1})
}

func TestRestablecerContacto_IncluyendoBajaLaBorraYConservaElResto(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t)
	a := e.contactoConFilas(t, "5491100001011")
	b := e.contactoConFilas(t, "5491100001012")

	r, err := e.almacen.RestablecerContacto(context.Background(), a, true)
	if err != nil {
		t.Fatalf("RestablecerContacto: %v", err)
	}
	if !r.Existe || !r.BajaIncluida {
		t.Fatalf("Existe=%v BajaIncluida=%v, se esperaba true y true", r.Existe, r.BajaIncluida)
	}
	if r.Cortacircuitos != 1 || r.PresentacionDeConversacion != 1 || r.BajaDeContacto != 1 {
		t.Fatalf("contadores = %d, %d, %d; se esperaba 1, 1, 1", r.Cortacircuitos, r.PresentacionDeConversacion, r.BajaDeContacto)
	}
	esperarCuentas(t, "contacto restablecido", e.cuentasDe(t, a), cuentas{identidad: 1, direccion: 1})
	esperarCuentas(t, "otro contacto", e.cuentasDe(t, b), cuentas{identidad: 1, direccion: 1, cortacircuitos: 1, presentacion: 1, baja: 1})
}

func TestRestablecerContacto_EsAtomicoSiFallaUnDelete(t *testing.T) {
	t.Parallel()
	casos := []struct {
		nombre      string
		tabla       string
		incluirBaja bool
	}{
		{"falla el segundo DELETE", "presentacion_de_conversacion", false},
		{"falla el tercer DELETE", "baja_de_contacto", true},
	}
	for _, caso := range casos {
		t.Run(caso.nombre, func(t *testing.T) {
			t.Parallel()
			e := nuevoEscenarioDeRestablecimiento(t)
			a := e.contactoConFilas(t, "5491100001021")
			trigger := fmt.Sprintf(`CREATE TRIGGER impedir_borrado BEFORE DELETE ON %s BEGIN SELECT RAISE(ABORT, 'borrado prohibido'); END`, caso.tabla)
			if _, err := e.lateral.Exec(trigger); err != nil {
				t.Fatalf("crear el disparador: %v", err)
			}

			if _, err := e.almacen.RestablecerContacto(context.Background(), a, caso.incluirBaja); err == nil {
				t.Fatal("se esperaba un error por el disparador, no hubo")
			}
			esperarCuentas(t, "nada debe haber cambiado", e.cuentasDe(t, a), cuentas{identidad: 1, direccion: 1, cortacircuitos: 1, presentacion: 1, baja: 1})
		})
	}
}

func TestRestablecerContacto_AlmacenCerradoDevuelveErrAlmacenCerrado(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t)
	a := e.contactoConFilas(t, "5491100001031")
	if err := e.almacen.Cerrar(); err != nil {
		t.Fatalf("Cerrar: %v", err)
	}

	_, err := e.almacen.RestablecerContacto(context.Background(), a, true)
	if !errors.Is(err, identidad.ErrAlmacenCerrado) {
		t.Fatalf("error = %v, se esperaba ErrAlmacenCerrado", err)
	}
	esperarCuentas(t, "nada debe haber cambiado", e.cuentasDe(t, a), cuentas{identidad: 1, direccion: 1, cortacircuitos: 1, presentacion: 1, baja: 1})
}

func TestRestablecerContacto_RechazaIdsMalformadosSinTocarFilas(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t)
	a := e.contactoConFilas(t, "5491100001041")
	malformados := map[string]string{
		"vacío":                "",
		"sin prefijo":          "0123456789abcdef0123456789abcdef",
		"mayúsculas":           "ct-0123456789ABCDEF0123456789ABCDEF",
		"31 hexadecimales":     "ct-0123456789abcdef0123456789abcde",
		"33 hexadecimales":     "ct-0123456789abcdef0123456789abcdef0",
		"multibyte":            "ct-0123456789abcdef0123456789abcdéé",
		"hexadecimal ajeno":    "ct-0123456789abcdef0123456789abcdeg",
		"prefijo en mayúscula": "CT-0123456789abcdef0123456789abcdef",
	}
	for nombre, id := range malformados {
		if _, err := e.almacen.RestablecerContacto(context.Background(), id, true); !errors.Is(err, identidad.ErrIdInternoInvalido) {
			t.Fatalf("%s: error = %v, se esperaba ErrIdInternoInvalido", nombre, err)
		}
	}
	esperarCuentas(t, "nada debe haber cambiado", e.cuentasDe(t, a), cuentas{identidad: 1, direccion: 1, cortacircuitos: 1, presentacion: 1, baja: 1})
}

func TestRestablecerContacto_ContactoExistenteSinFilasEsExitoConContadoresEnCero(t *testing.T) {
	t.Parallel()
	for _, incluirBaja := range []bool{false, true} {
		t.Run(fmt.Sprintf("incluir_baja=%v", incluirBaja), func(t *testing.T) {
			t.Parallel()
			e := nuevoEscenarioDeRestablecimiento(t)
			c := e.contactoSinFilas(t, "5491100001051")

			r, err := e.almacen.RestablecerContacto(context.Background(), c, incluirBaja)
			if err != nil {
				t.Fatalf("RestablecerContacto: %v", err)
			}
			if !r.Existe {
				t.Fatal("un contacto presente en identidad debe informar Existe=true aunque no tenga filas")
			}
			if r.Cortacircuitos != 0 || r.PresentacionDeConversacion != 0 || r.BajaDeContacto != 0 {
				t.Fatalf("contadores = %d, %d, %d; se esperaba 0, 0, 0", r.Cortacircuitos, r.PresentacionDeConversacion, r.BajaDeContacto)
			}
			esperarCuentas(t, "contacto sin filas", e.cuentasDe(t, c), cuentas{identidad: 1, direccion: 1})
		})
	}
}

func TestRestablecerContacto_IdAusenteDeIdentidadNoExisteYNoTocaNada(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t)
	otro := e.contactoConFilas(t, "5491100001061")
	const ausente = "ct-ffffffffffffffffffffffffffffffff"

	r, err := e.almacen.RestablecerContacto(context.Background(), ausente, true)
	if err != nil {
		t.Fatalf("RestablecerContacto: %v", err)
	}
	if r.Existe {
		t.Fatal("un id ausente de identidad debe informar Existe=false")
	}
	if r.Cortacircuitos != 0 || r.PresentacionDeConversacion != 0 || r.BajaDeContacto != 0 {
		t.Fatalf("contadores = %d, %d, %d; se esperaba 0, 0, 0", r.Cortacircuitos, r.PresentacionDeConversacion, r.BajaDeContacto)
	}
	esperarCuentas(t, "otro contacto", e.cuentasDe(t, otro), cuentas{identidad: 1, direccion: 1, cortacircuitos: 1, presentacion: 1, baja: 1})
}

func TestRestablecerContacto_EsIdempotente(t *testing.T) {
	t.Parallel()
	e := nuevoEscenarioDeRestablecimiento(t)
	a := e.contactoConFilas(t, "5491100001071")

	primera, err := e.almacen.RestablecerContacto(context.Background(), a, true)
	if err != nil {
		t.Fatalf("primera ejecución: %v", err)
	}
	if !primera.Existe || primera.Cortacircuitos != 1 || primera.PresentacionDeConversacion != 1 || primera.BajaDeContacto != 1 {
		t.Fatalf("primera ejecución = %+v, se esperaba Existe con 1, 1, 1", primera)
	}
	segunda, err := e.almacen.RestablecerContacto(context.Background(), a, true)
	if err != nil {
		t.Fatalf("segunda ejecución: %v", err)
	}
	if !segunda.Existe {
		t.Fatal("la segunda ejecución sobre un contacto real debe seguir informando Existe=true")
	}
	if segunda.Cortacircuitos != 0 || segunda.PresentacionDeConversacion != 0 || segunda.BajaDeContacto != 0 {
		t.Fatalf("segunda ejecución = %+v, se esperaban contadores 0, 0, 0", segunda)
	}
}

func TestEsIdInternoValido(t *testing.T) {
	t.Parallel()
	if !identidad.EsIdInternoValido("ct-0123456789abcdef0123456789abcdef") {
		t.Fatal("un id bien formado debe ser válido")
	}
	if identidad.EsIdInternoValido("ct-0123456789abcdef0123456789abcdeF") {
		t.Fatal("una mayúscula debe invalidar el id")
	}
}
