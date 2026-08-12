// Package canal gestiona el respaldo del sqlstore del sidecar mediante VACUUM INTO
// sobre su propia conexión dedicada de solo lectura, según lo estipulado en
// `docs/contrato-ipc-respaldo-del-sqlstore.md` y `docs/protocolo-ipc-nucleo-sidecar.md` sección 7.
package canal

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"path/filepath"

	_ "modernc.org/sqlite"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// NombreCanonicoDeCopiaSqlstore es el nombre de archivo canónico bajo el directorio de destino.
const NombreCanonicoDeCopiaSqlstore = "sqlstore.db"

// Nombres fijos de eventos de registro estructurado para el respaldo del sqlstore.
const (
	EventoRespaldoSqlstoreCompletado = "canal.respaldo_sqlstore_completado"
	EventoRespaldoSqlstoreFallido    = "canal.respaldo_sqlstore_fallido"
)

// GanchoDePruebaTrasVacuum, cuando no es nil, se invoca justo después de que VACUUM INTO escribe
// la copia y antes de que el manejador la abra para verificarla. Existe solo para que los tests
// de este paquete puedan simular una copia corrupta sin tocar el código de producción; en
// producción permanece siempre nil.
var GanchoDePruebaTrasVacuum func(rutaCopia string)

// AbrirConexionDeRespaldo abre una conexión dedicada de solo lectura al sqlstore.
//
// No utiliza la conexión interna de whatsmeow (que está encapsulada y no exportada en sqlstore.Container),
// asegurando que la operación de respaldo nunca bloquee ni compita con el protocolo en curso.
func AbrirConexionDeRespaldo(rutaSqlstore string) (*sql.DB, error) {
	dsn := fmt.Sprintf("file:%s?mode=ro&_pragma=busy_timeout(5000)", rutaSqlstore)
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("canal: no se pudo abrir la conexión de respaldo: %w", err)
	}
	if err := db.Ping(); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("canal: no se pudo verificar la conexión de respaldo: %w", err)
	}
	return db, nil
}

// verificarDestinoDisponible comprueba que el directorio de destino existe y que el archivo
// de destino aún no existe, devolviendo la ruta completa del archivo de copia.
func verificarDestinoDisponible(destino string) (string, error) {
	info, err := os.Stat(destino)
	if err != nil {
		return "", fmt.Errorf("el directorio de destino no es accesible: %w", err)
	}
	if !info.IsDir() {
		return "", fmt.Errorf("el destino no es un directorio: %s", destino)
	}
	rutaCopia := filepath.Join(destino, NombreCanonicoDeCopiaSqlstore)
	if _, err := os.Stat(rutaCopia); err == nil {
		return "", fmt.Errorf("el archivo de destino ya existe: %s", rutaCopia)
	}
	return rutaCopia, nil
}

// fallar registra el fallo con su motivo y construye el acuse fallido correspondiente.
//
// Si rutaCopia no está vacía, VACUUM INTO ya escribió una copia en esa ruta: fallar intenta
// eliminarla (best-effort, con el propio intento registrado si falla) antes de responder, para
// que nunca quede una copia sin verificar bajo el nombre canónico -- eso bloquearía además
// cualquier reintento sobre el mismo destino, ya que verificarDestinoDisponible rechaza un
// archivo de destino ya existente.
func fallar(reg *registro.Registro, ronda, rutaCopia, motivo string) ipc.AcuseRespaldoSqlstore {
	if rutaCopia != "" {
		if errEliminar := os.Remove(rutaCopia); errEliminar != nil && !os.IsNotExist(errEliminar) {
			if reg != nil {
				reg.Error(EventoRespaldoSqlstoreFallido, registro.Campos{
					Detalle: fmt.Sprintf("ronda=%s no se pudo eliminar la copia sin verificar: %v", ronda, errEliminar),
				})
			}
		}
	}
	if reg != nil {
		reg.Error(EventoRespaldoSqlstoreFallido, registro.Campos{
			Detalle: fmt.Sprintf("ronda=%s motivo=%s", ronda, motivo),
		})
	}
	return ipc.AcuseRespaldoSqlstore{
		IdentificadorDeRonda: ronda,
		Resultado:            ipc.ResultadoFallido,
		RutaDeLaCopia:        "",
		Bytes:                0,
		Motivo:               motivo,
	}
}

// ManejarOrdenRespaldoSqlstore procesa la orden de respaldo: valida la orden y el destino,
// captura user_version del origen, ejecuta VACUUM INTO, verifica integridad y user_version en la
// copia, y emite el acuse. Cualquier fallo posterior a la escritura de la copia la elimina antes
// de responder (ver fallar), de modo que el sidecar nunca deja una copia sin verificar bajo el
// nombre canónico.
func ManejarOrdenRespaldoSqlstore(
	ctx context.Context,
	dbRespaldo *sql.DB,
	orden ipc.OrdenRespaldoSqlstore,
	reg *registro.Registro,
) ipc.AcuseRespaldoSqlstore {
	if orden.Orden != ipc.OrdenRespaldarSqlstore {
		return fallar(reg, orden.IdentificadorDeRonda, "",
			fmt.Sprintf("orden inesperada: %q", orden.Orden))
	}

	rutaCopia, err := verificarDestinoDisponible(orden.Destino)
	if err != nil {
		return fallar(reg, orden.IdentificadorDeRonda, "", fmt.Sprintf("destino no disponible: %v", err))
	}

	if dbRespaldo == nil {
		return fallar(reg, orden.IdentificadorDeRonda, "", "conexión de base de datos de respaldo nula")
	}

	var userVersionOrigen int64
	if err := dbRespaldo.QueryRowContext(ctx, "PRAGMA user_version").Scan(&userVersionOrigen); err != nil {
		return fallar(reg, orden.IdentificadorDeRonda, "",
			fmt.Sprintf("error al leer user_version del origen: %v", err))
	}

	if _, err := dbRespaldo.ExecContext(ctx, "VACUUM INTO ?", rutaCopia); err != nil {
		return fallar(reg, orden.IdentificadorDeRonda, "", fmt.Sprintf("error al ejecutar VACUUM INTO: %v", err))
	}

	if GanchoDePruebaTrasVacuum != nil {
		GanchoDePruebaTrasVacuum(rutaCopia)
	}

	dbCopia, err := sql.Open("sqlite", fmt.Sprintf("file:%s?mode=ro", rutaCopia))
	if err != nil {
		return fallar(reg, orden.IdentificadorDeRonda, rutaCopia,
			fmt.Sprintf("error al abrir copia para verificación: %v", err))
	}

	var integridad string
	if err := dbCopia.QueryRowContext(ctx, "PRAGMA integrity_check").Scan(&integridad); err != nil {
		_ = dbCopia.Close()
		return fallar(reg, orden.IdentificadorDeRonda, rutaCopia,
			fmt.Sprintf("error al verificar integridad de la copia: %v", err))
	}
	if integridad != "ok" {
		_ = dbCopia.Close()
		return fallar(reg, orden.IdentificadorDeRonda, rutaCopia,
			fmt.Sprintf("integridad de copia no es ok: %s", integridad))
	}

	var userVersionCopia int64
	if err := dbCopia.QueryRowContext(ctx, "PRAGMA user_version").Scan(&userVersionCopia); err != nil {
		_ = dbCopia.Close()
		return fallar(reg, orden.IdentificadorDeRonda, rutaCopia,
			fmt.Sprintf("error al leer user_version de la copia: %v", err))
	}
	if userVersionCopia != userVersionOrigen {
		_ = dbCopia.Close()
		return fallar(reg, orden.IdentificadorDeRonda, rutaCopia,
			fmt.Sprintf("desajuste de user_version: origen=%d copia=%d", userVersionOrigen, userVersionCopia))
	}

	_ = dbCopia.Close()

	infoCopia, err := os.Stat(rutaCopia)
	if err != nil {
		return fallar(reg, orden.IdentificadorDeRonda, rutaCopia,
			fmt.Sprintf("error al obtener tamaño de la copia: %v", err))
	}

	if reg != nil {
		reg.Info(EventoRespaldoSqlstoreCompletado, registro.Campos{
			Detalle: fmt.Sprintf("ronda=%s bytes=%d", orden.IdentificadorDeRonda, infoCopia.Size()),
		})
	}

	return ipc.AcuseRespaldoSqlstore{
		IdentificadorDeRonda: orden.IdentificadorDeRonda,
		Resultado:            ipc.ResultadoCompletado,
		RutaDeLaCopia:        rutaCopia,
		Bytes:                infoCopia.Size(),
		Motivo:               "",
	}
}
