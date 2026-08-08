package identidad

import (
	"context"
	"encoding/hex"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/registro"
	"go.mau.fi/whatsmeow/types"
)

func abrirAlmacenInternoDePrueba(t *testing.T) *Almacen {
	t.Helper()
	ruta := filepath.Join(t.TempDir(), "identidad_interna.db")
	almacen, err := Abrir(Opciones{
		Ruta:     ruta,
		Registro: registro.Nuevo(io.Discard, slog.LevelInfo, "celula-test"),
	})
	if err != nil {
		t.Fatalf("Abrir: %v", err)
	}
	t.Cleanup(func() { almacen.Cerrar() })
	return almacen
}

func TestConstruirDSN_PragmasRequeridos(t *testing.T) {
	dsn := construirDSN("test.db")

	esperados := []string{
		"_pragma=foreign_keys(1)",
		"_pragma=journal_mode(WAL)",
		"_pragma=synchronous(FULL)",
		"_pragma=busy_timeout(5000)",
	}

	for _, esp := range esperados {
		if !strings.Contains(dsn, esp) {
			t.Errorf("Esperaba que DSN contuviera %q, pero es %q", esp, dsn)
		}
	}
	if strings.Contains(dsn, "?_foreign_keys=on") {
		t.Errorf("No debe usar sintaxis mattn")
	}
}

func TestGenerarIdInterno_Forma(t *testing.T) {
	id, err := generarIdInterno()
	if err != nil {
		t.Fatalf("Error al generar: %v", err)
	}

	if !strings.HasPrefix(id, PrefijoIdentidad) {
		t.Errorf("Id %q no tiene prefijo %q", id, PrefijoIdentidad)
	}
	if len(id) != 35 {
		t.Errorf("Id %q tiene longitud %d, se esperaba 35", id, len(id))
	}
	parteHex := strings.TrimPrefix(id, PrefijoIdentidad)
	if strings.ToLower(parteHex) != parteHex {
		t.Errorf("Id %q debe ser minúscula pura", id)
	}
	if _, err := hex.DecodeString(parteHex); err != nil {
		t.Errorf("Id %q no tiene hex válido: %v", id, err)
	}
}

func TestFusion_SupervivienteEsElMasAntiguo(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	resLid, _ := almacen.Resolver(ctx, Observacion{LID: lid})
	// Actualizamos en BD para forzar que LID sea más antiguo
	_, err := almacen.db.ExecContext(ctx, "UPDATE identidad SET creada_en_ms = 100 WHERE id_interno = ?", resLid.IdInterno)
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	resPn, _ := almacen.Resolver(ctx, Observacion{PN: pn})
	_, err = almacen.db.ExecContext(ctx, "UPDATE identidad SET creada_en_ms = 200 WHERE id_interno = ?", resPn.IdInterno)
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	resFusion, err := almacen.Resolver(ctx, Observacion{PN: pn, LID: lid})
	if err != nil {
		t.Fatalf("Resolver fusión: %v", err)
	}

	if resFusion.IdInterno != resLid.IdInterno {
		t.Errorf("Se esperaba que sobreviviera el más antiguo (LID), sobrevivió %q", resFusion.IdInterno)
	}
}

func TestFusion_ConflictoDePNDistinto(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)
	ctx := context.Background()
	pn1 := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	pn2 := types.JID{User: "5491155559999", Server: types.DefaultUserServer}
	lid := types.JID{User: "abc123", Server: "lid"}

	res1, _ := almacen.Resolver(ctx, Observacion{PN: pn1, LID: lid})
	res2, err := almacen.Resolver(ctx, Observacion{PN: pn2, LID: lid})
	if err != nil {
		t.Fatalf("Resolver: %v", err)
	}

	if !res2.ConflictoDeAlias {
		t.Errorf("Se esperaba ConflictoDeAlias en true")
	}
	if res1.IdInterno == res2.IdInterno {
		t.Errorf("No se debe fusionar con un PN distinto")
	}
}

func TestFusion_CadenaDeUnSalto(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)
	ctx := context.Background()

	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lid1 := types.JID{User: "abc1", Server: "lid"}
	lid2 := types.JID{User: "abc2", Server: "lid"}

	rLid1, _ := almacen.Resolver(ctx, Observacion{LID: lid1})
	rLid2, _ := almacen.Resolver(ctx, Observacion{LID: lid2})

	// Simular manualmente que LID2 está fusionada en LID1
	almacen.db.ExecContext(ctx, "UPDATE identidad SET estado = ?, fusionada_en = ? WHERE id_interno = ?", EstadoFusionada, rLid1.IdInterno, rLid2.IdInterno)

	rPn, _ := almacen.Resolver(ctx, Observacion{PN: pn})
	almacen.db.ExecContext(ctx, "UPDATE identidad SET creada_en_ms = 100 WHERE id_interno = ?", rPn.IdInterno)
	almacen.db.ExecContext(ctx, "UPDATE identidad SET creada_en_ms = 200 WHERE id_interno = ?", rLid1.IdInterno)

	// Fusión PN y LID1, LID1 será absorbida por PN, LID2 que apuntaba a LID1 debe ser redirigida a PN
	almacen.Resolver(ctx, Observacion{PN: pn, LID: lid1})

	var fusionadaEn string
	err := almacen.db.QueryRowContext(ctx, "SELECT fusionada_en FROM identidad WHERE id_interno = ?", rLid2.IdInterno).Scan(&fusionadaEn)
	if err != nil {
		t.Fatalf("Scan: %v", err)
	}
	if fusionadaEn != rPn.IdInterno {
		t.Errorf("Cadena excede 1 salto, fusionada_en = %q, esperado = %q", fusionadaEn, rPn.IdInterno)
	}
}

func TestConcurrencia_ResolverSimultaneo(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)

	var wg sync.WaitGroup
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}

	n := 20
	resultados := make([]string, n)
	errores := make([]error, n)

	wg.Add(n)
	for i := 0; i < n; i++ {
		go func(i int) {
			defer wg.Done()
			ctx := context.Background()
			res, err := almacen.Resolver(ctx, Observacion{PN: pn})
			if err == nil {
				resultados[i] = res.IdInterno
			}
			errores[i] = err
		}(i)
	}
	wg.Wait()

	for i, err := range errores {
		if err != nil {
			t.Fatalf("Goroutine %d devolvió error: %v", i, err)
		}
	}

	primerRes := resultados[0]
	for i := 1; i < n; i++ {
		if resultados[i] != primerRes {
			t.Errorf("Se esperaba que todas las goroutines devolvieran %q, la %d devolvió %q", primerRes, i, resultados[i])
		}
	}

	var count int
	err := almacen.db.QueryRow("SELECT count(*) FROM identidad").Scan(&count)
	if err != nil {
		t.Fatalf("QueryRow: %v", err)
	}
	if count != 1 {
		t.Errorf("Se esperaba exactamente 1 identidad en BD, hay %d", count)
	}
}

// TestResolver_TransaccionFallidaNoDevuelveIdUsable cierra la conexión subyacente
// sin pasar por Cerrar() (el flag `cerrado` sigue en false) para forzar que la
// transacción falle, y comprueba que Resolver propaga el error en vez de un
// IdInterno con apariencia válida: antes de esta corrección los tx.Commit() de
// Resolver descartaban su error.
func TestResolver_TransaccionFallidaNoDevuelveIdUsable(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}

	if err := almacen.db.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}

	res, err := almacen.Resolver(ctx, Observacion{PN: pn})
	if err == nil {
		t.Fatalf("se esperaba un error al fallar la transacción, Resolver devolvió éxito")
	}
	if res.IdInterno != "" {
		t.Errorf("Resolver devolvió id_interno %q pese al fallo de la transacción", res.IdInterno)
	}
}

// TestFusion_DesempatePorRowidConCreadaEnMsIguales empata creada_en_ms para
// ejercitar el desempate exigido por 02-contract.yaml línea 102: entre dos
// identidades igual de "antiguas", sobrevive la insertada primero (rowid
// menor). Los dos id_interno se siembran a mano (no via generarIdInterno, que
// usa crypto/rand) y a propósito en orden lexicográfico INVERSO al de
// inserción: el insertado primero (idPrimero) es lexicográficamente MAYOR
// que el insertado después (idSegundo). Así, si alguien revierte la regla a
// una comparación lexicográfica de id_interno, el resultado esperado difiere
// del correcto en el 100% de las corridas, no en un volado del 50%.
func TestFusion_DesempatePorRowidConCreadaEnMsIguales(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)
	ctx := context.Background()
	lid := types.JID{User: "abc123", Server: "lid"}
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}

	const idPrimero = PrefijoIdentidad + "ffffffffffffffffffffffffffffffff" // rowid menor, lexicográficamente mayor
	const idSegundo = PrefijoIdentidad + "0000000000000000000000000000000a" // rowid mayor, lexicográficamente menor
	const creadaEnMs = 500

	for _, id := range []string{idPrimero, idSegundo} {
		if _, err := almacen.db.ExecContext(ctx,
			`INSERT INTO identidad (id_interno, estado, creada_en_ms, actualizada_en_ms) VALUES (?, ?, ?, ?)`,
			id, EstadoProvisional, creadaEnMs, creadaEnMs); err != nil {
			t.Fatalf("insertar identidad %q: %v", id, err)
		}
	}
	// idPrimero (insertado primero, rowid menor) queda asociado al LID y
	// idSegundo al PN, para que el desempate correcto elija un superviviente
	// distinto de idPn y así distinga la regla del empate de la regla general.
	if _, err := almacen.db.ExecContext(ctx,
		`INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?)`,
		DireccionLID, lid.User, lid.Server, idPrimero, creadaEnMs); err != nil {
		t.Fatalf("insertar direccion lid: %v", err)
	}
	if _, err := almacen.db.ExecContext(ctx,
		`INSERT INTO direccion (tipo, usuario, servidor, id_interno, observada_en_ms) VALUES (?, ?, ?, ?, ?)`,
		DireccionPN, pn.User, pn.Server, idSegundo, creadaEnMs); err != nil {
		t.Fatalf("insertar direccion pn: %v", err)
	}

	resFusion, err := almacen.Resolver(ctx, Observacion{PN: pn, LID: lid})
	if err != nil {
		t.Fatalf("Resolver fusión: %v", err)
	}
	if resFusion.IdInterno != idPrimero {
		t.Errorf("con creada_en_ms empatado, el desempate por rowid debía preservar %q (insertado primero), sobrevivió %q", idPrimero, resFusion.IdInterno)
	}
}

// TestIdentidadGo_TodosLosCommitsDeTransaccionSeComprueban es un tripwire a
// nivel de fuente (mismo patrón que sidecar/internal/ipc/documento_test.go):
// lee identidad.go y exige que todo tx.Commit() aparezca en su forma
// comprobada. Un tx.Commit() descartado (`_ = tx.Commit()` o una sentencia
// suelta `tx.Commit()`) no tiene cobertura funcional en esta suite porque
// ningún test fuerza un fallo en Commit, así que esta prueba es la única
// barrera contra esa regresión.
func TestIdentidadGo_TodosLosCommitsDeTransaccionSeComprueban(t *testing.T) {
	_, archivo, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("no se pudo determinar la ruta de este archivo de prueba")
	}
	rutaFuente := filepath.Join(filepath.Dir(archivo), "identidad.go")
	contenido, err := os.ReadFile(rutaFuente)
	if err != nil {
		t.Fatalf("no se pudo leer %s: %v", rutaFuente, err)
	}
	fuente := string(contenido)

	reDescartado := regexp.MustCompile(`(?m)^\s*_\s*=\s*tx\.Commit\(\)|(?m)^\s*tx\.Commit\(\)\s*$`)
	if hallazgos := reDescartado.FindAllString(fuente, -1); len(hallazgos) > 0 {
		t.Errorf("tx.Commit() descartado sin comprobar su error: %v", hallazgos)
	}

	reComprobado := regexp.MustCompile(`tx\.Commit\(\)`)
	total := len(reComprobado.FindAllString(fuente, -1))
	if total == 0 {
		t.Fatal("no se encontró ningún tx.Commit() en identidad.go; el tripwire perdió su objeto de vigilancia")
	}
	if total != strings.Count(fuente, "if err := tx.Commit(); err != nil {") {
		t.Errorf("hay tx.Commit() que no están en la forma comprobada `if err := tx.Commit(); err != nil {`")
	}
}

// TestResolver_LIDReemplazaSinConflictoEnIdentidadAnclada pina el comportamiento
// aceptado en Caso 2: el PN es el ancla y rechaza un segundo valor distinto, pero
// el LID es un alias de dispositivo que puede reemplazarse sin ConflictoDeAlias.
func TestResolver_LIDReemplazaSinConflictoEnIdentidadAnclada(t *testing.T) {
	almacen := abrirAlmacenInternoDePrueba(t)
	ctx := context.Background()
	pn := types.JID{User: "5491155551234", Server: types.DefaultUserServer}
	lidNuevo := types.JID{User: "dispositivoNuevo", Server: "lid"}

	resAncla, err := almacen.Resolver(ctx, Observacion{PN: pn})
	if err != nil {
		t.Fatalf("Resolver ancla: %v", err)
	}

	resConLid, err := almacen.Resolver(ctx, Observacion{PN: pn, LID: lidNuevo})
	if err != nil {
		t.Fatalf("Resolver con LID nuevo: %v", err)
	}
	if resConLid.ConflictoDeAlias {
		t.Errorf("adjuntar un LID nuevo a una identidad anclada no debe marcar ConflictoDeAlias")
	}
	if resConLid.IdInterno != resAncla.IdInterno {
		t.Errorf("el id_interno no debe cambiar al adjuntar un LID nuevo, obtuve %q, esperaba %q", resConLid.IdInterno, resAncla.IdInterno)
	}
}
