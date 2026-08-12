package canal_test

import (
	"bytes"
	"context"
	"database/sql"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// crearOrigen crea un sqlstore real con datos y user_version conocidos, y devuelve la conexión
// de respaldo dedicada ya abierta sobre él (el llamante debe cerrarla).
func crearOrigen(t *testing.T, dirTemp string, userVersion int64) *sql.DB {
	t.Helper()
	rutaOrigen := filepath.Join(dirTemp, "source_sqlstore.db")
	dbOrigen, err := sql.Open("sqlite", fmt.Sprintf("file:%s", rutaOrigen))
	if err != nil {
		t.Fatalf("no se pudo crear la base de origen: %v", err)
	}
	if _, err := dbOrigen.Exec("CREATE TABLE datos (id INTEGER PRIMARY KEY, valor TEXT);"); err != nil {
		t.Fatalf("no se pudo crear la tabla de datos: %v", err)
	}
	if _, err := dbOrigen.Exec("INSERT INTO datos VALUES (1, 'hola mundo');"); err != nil {
		t.Fatalf("no se pudo insertar en la tabla de datos: %v", err)
	}
	if _, err := dbOrigen.Exec(fmt.Sprintf("PRAGMA user_version = %d;", userVersion)); err != nil {
		t.Fatalf("no se pudo establecer user_version: %v", err)
	}
	if err := dbOrigen.Close(); err != nil {
		t.Fatalf("no se pudo cerrar la base de origen: %v", err)
	}
	dbRespaldo, err := canal.AbrirConexionDeRespaldo(rutaOrigen)
	if err != nil {
		t.Fatalf("AbrirConexionDeRespaldo: %v", err)
	}
	return dbRespaldo
}

func nuevoRegistro() *registro.Registro {
	return registro.Nuevo(&bytes.Buffer{}, slog.LevelInfo, "celula-test")
}

// asertarEnvoltorioIpc comprueba que el acuse sobrevive intacto una vuelta de codificación y
// decodificación IPC con la regla de todos-los-campos-presentes.
func asertarEnvoltorioIpc(t *testing.T, acuse ipc.AcuseRespaldoSqlstore) {
	t.Helper()
	sobre := ipc.NuevoSobre(acuse)
	lineaCodificada, err := ipc.Codificar(sobre)
	if err != nil {
		t.Fatalf("no se pudo codificar sobre IPC: %v", err)
	}
	sobreDecodificado, err := ipc.Decodificar(lineaCodificada)
	if err != nil {
		t.Fatalf("no se pudo decodificar línea IPC: %v", err)
	}
	acuseDecodificado, ok := sobreDecodificado.Cuerpo.(ipc.AcuseRespaldoSqlstore)
	if !ok {
		t.Fatalf("el cuerpo decodificado no es AcuseRespaldoSqlstore: %T", sobreDecodificado.Cuerpo)
	}
	if acuseDecodificado != acuse {
		t.Errorf("acuse decodificado no coincide con el original: %+v vs %+v", acuseDecodificado, acuse)
	}
}

func TestManejarOrdenRespaldoSqlstoreExitoso(t *testing.T) {
	dirTemp := t.TempDir()
	dbRespaldo := crearOrigen(t, dirTemp, 42)
	defer dbRespaldo.Close()

	destino := filepath.Join(dirTemp, "copia_destino")
	if err := os.MkdirAll(destino, 0755); err != nil {
		t.Fatalf("no se pudo crear el directorio de destino: %v", err)
	}
	orden := ipc.OrdenRespaldoSqlstore{
		Orden: ipc.OrdenRespaldarSqlstore, Destino: destino, IdentificadorDeRonda: "ronda-test-123",
	}

	acuse := canal.ManejarOrdenRespaldoSqlstore(context.Background(), dbRespaldo, orden, nuevoRegistro())

	rutaEsperada := filepath.Join(destino, canal.NombreCanonicoDeCopiaSqlstore)
	if acuse.Resultado != ipc.ResultadoCompletado || acuse.RutaDeLaCopia != rutaEsperada || acuse.Bytes <= 0 || acuse.Motivo != "" {
		t.Fatalf("acuse de éxito inconsistente: %+v (ruta_esperada=%q)", acuse, rutaEsperada)
	}

	dbCopia, err := sql.Open("sqlite", fmt.Sprintf("file:%s?mode=ro", rutaEsperada))
	if err != nil {
		t.Fatalf("no se pudo abrir la copia generada: %v", err)
	}
	defer dbCopia.Close()
	var userVersionCopia int64
	if err := dbCopia.QueryRow("PRAGMA user_version").Scan(&userVersionCopia); err != nil {
		t.Fatalf("no se pudo leer user_version de la copia: %v", err)
	}
	var valor string
	if err := dbCopia.QueryRow("SELECT valor FROM datos WHERE id = 1").Scan(&valor); err != nil {
		t.Fatalf("no se pudo consultar tabla de la copia: %v", err)
	}
	if userVersionCopia != 42 || valor != "hola mundo" {
		t.Errorf("copia inconsistente: user_version=%d valor=%q", userVersionCopia, valor)
	}
	asertarEnvoltorioIpc(t, acuse)
}

// TestManejarOrdenRespaldoSqlstoreFallos cubre, en tabla, las ramas de fallo del manejador: antes
// de VACUUM INTO (destino inexistente, destino ocupado, Orden inesperado) y después (verificación
// fallida, vía el gancho de prueba que altera el user_version de la copia ya escrita). Comprueba
// resultado=fallido, ruta_de_la_copia vacía, y -- cuando VACUUM INTO llegó a escribir algo -- que
// la copia sin verificar fue eliminada de destino.
func TestManejarOrdenRespaldoSqlstoreFallos(t *testing.T) {
	casos := []struct {
		nombre               string
		ordenValor           string
		destinoOcupado       bool
		sinDirectorioDestino bool
		alterarTrasVacuum    bool
		versionOrigen        int64
		verificarLimpieza    bool
	}{
		{nombre: "destino inexistente", ordenValor: ipc.OrdenRespaldarSqlstore, sinDirectorioDestino: true, versionOrigen: 1},
		{nombre: "destino ocupado", ordenValor: ipc.OrdenRespaldarSqlstore, destinoOcupado: true, versionOrigen: 1},
		{nombre: "orden inesperada", ordenValor: "orden_desconocida", sinDirectorioDestino: true, versionOrigen: 1},
		{
			nombre: "verificacion fallida tras vacuum", ordenValor: ipc.OrdenRespaldarSqlstore,
			alterarTrasVacuum: true, versionOrigen: 42, verificarLimpieza: true,
		},
	}

	for _, c := range casos {
		t.Run(c.nombre, func(t *testing.T) {
			dirTemp := t.TempDir()
			dbRespaldo := crearOrigen(t, dirTemp, c.versionOrigen)
			defer dbRespaldo.Close()

			destino := filepath.Join(dirTemp, "destino")
			if !c.sinDirectorioDestino {
				if err := os.MkdirAll(destino, 0755); err != nil {
					t.Fatalf("no se pudo crear el directorio de destino: %v", err)
				}
			}
			rutaCopia := filepath.Join(destino, canal.NombreCanonicoDeCopiaSqlstore)
			if c.destinoOcupado {
				if err := os.WriteFile(rutaCopia, []byte("contenido previo"), 0644); err != nil {
					t.Fatalf("no se pudo crear el archivo ocupado: %v", err)
				}
			}
			if c.alterarTrasVacuum {
				canal.GanchoDePruebaTrasVacuum = func(ruta string) {
					dbTamper, err := sql.Open("sqlite", fmt.Sprintf("file:%s", ruta))
					if err != nil {
						t.Fatalf("no se pudo abrir la copia para alterarla: %v", err)
					}
					if _, err := dbTamper.Exec("PRAGMA user_version = 12345;"); err != nil {
						t.Fatalf("no se pudo alterar user_version de la copia: %v", err)
					}
					if err := dbTamper.Close(); err != nil {
						t.Fatalf("no se pudo cerrar la conexión de alteración: %v", err)
					}
				}
				defer func() { canal.GanchoDePruebaTrasVacuum = nil }()
			}

			orden := ipc.OrdenRespaldoSqlstore{Orden: c.ordenValor, Destino: destino, IdentificadorDeRonda: "ronda-" + c.nombre}
			acuse := canal.ManejarOrdenRespaldoSqlstore(context.Background(), dbRespaldo, orden, nuevoRegistro())

			if acuse.Resultado != ipc.ResultadoFallido || acuse.RutaDeLaCopia != "" || acuse.Bytes != 0 || acuse.Motivo == "" {
				t.Fatalf("acuse de fallo inconsistente: %+v", acuse)
			}
			if c.verificarLimpieza {
				if _, err := os.Stat(rutaCopia); !os.IsNotExist(err) {
					t.Errorf("la copia sin verificar debía eliminarse de destino, pero sigue presente en %q", rutaCopia)
				}
			}
			asertarEnvoltorioIpc(t, acuse)
		})
	}
}
