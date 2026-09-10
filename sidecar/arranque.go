// Package main: extracción de la secuencia de apertura de recursos del sidecar.
//
// # Por qué este archivo existe
//
// En el orden histórico de arranque, la conexión de respaldo de solo lectura al almacén de
// identidad (su llamada más abajo) se abría ANTES de que
// identidad.Abrir creara el archivo `identidad.db` en una carpeta de datos vacía. El DSN
// `mode=ro` falla con "unable to open database file (14)" cuando el destino no existe todavía,
// así que el primer arranque real de cada célula sobre el canal propio fallaba cerrado.
// Esta función corrige ese orden: identidad.Abrir corre siempre antes de la conexión de respaldo
// de identidad.
//
// # Por qué main() no hace este cableado directamente
//
// El cableado de los seis recursos vive aquí para que main() se reduzca a cargar configuración,
// llamar esta función y manejar su error — y para que la secuencia completa, hoy sin cobertura de
// pruebas porque vive en main(), pueda ejercitarse contra un directorio temporal. La auditoría
// de la secuencia está documentada como hallazgo en la bitácora de descartes D-43.
//
// # Por qué no se extrae más allá del buzón
//
// Tras el buzón, la construcción del servidor IPC y la cola de salida es circular: la cola
// necesita `srv.EnviarAcuseEnvio` (ConSumideroDeAcuse) y el servidor necesita `colaSalida` para
// `Portero` (Dependencias.Portero). Romper ese ciclo aquí fue evaluado y descartado: es un
// movimiento estructural más grande que el que esta tarea compra, y se anota en D-43.
package main

import (
	"context"
	"database/sql"

	"github.com/CGary/hexcell/sidecar/internal/canal"
	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/identidad"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"go.mau.fi/whatsmeow/store/sqlstore"
)

// recursosDeArranque agrupa los seis recursos que main() abre durante el arranque.
//
// Los cinco primeros son los devueltos por los `defer` de main() — su LIFO debe coincidir
// exactamente con el orden en que se declararon los defers para no alterar el apagado ordenado.
// `Sesion` también viaja en este struct porque se construye en la misma fase y main() la
// necesita más adelante, pero no se deferea: se cierra explícitamente tras la señal de parada.
type recursosDeArranque struct {
	// Contenedor es el sqlstore.Container de whatsmeow. Dueño del archivo sqlstore.db.
	Contenedor *sqlstore.Container
	// Sesion es el cliente whatsmeow sobre el contenedor.
	Sesion *canal.Sesion
	// DBRespaldo es la conexión dedicada de solo lectura a sqlstore.db para VACUUM INTO.
	// La posee este proceso; el núcleo nunca abre sqlstore.db directamente.
	DBRespaldo *sql.DB
	// DBRespaldoIdentidad es la conexión dedicada de solo lectura a identidad.db para
	// VACUUM INTO (adr-0022: el núcleo nunca abre identidad.db; la copia la produce el
	// proceso dueño desde una conexión suya).
	DBRespaldoIdentidad *sql.DB
	// AlmacenIdentidad es el almacén de identidad del adaptador, dueño de identidad.db.
	AlmacenIdentidad *identidad.Almacen
	// Buzon es el buzón de salida durable sobre outbox.db.
	Buzon *outbox.Outbox
}

// abrirRecursosDeArranque reproduce la secuencia de apertura que vivía inline en main(),
// con el orden corregido para que el archivo `identidad.db` exista antes de que su conexión
// de respaldo de solo lectura intente abrirlo. Devuelve (nil, err) ante el primer fallo y
// no añade limpieza nueva sobre los recursos ya abiertos en ese camino: el contrato de
// apagado de main() sigue siendo os.Exit(1), que reclame descriptores sin cooperar con la
// jerarquía de defer. Esa característica se preserva deliberadamente y se documenta en D-43.
//
// Orden de apertura, con anotación de creador/consumidor (los nombres reales viven más abajo):
//
//  1. AbrirAlmacenDeDispositivo sobre la ruta del sqlstore  — crea sqlstore.db
//  2. NuevaSesion(contenedor)                               — consume contenedor (sin E/S de archivo)
//  3. AbrirConexionDeRespaldo sobre la ruta del sqlstore    — consume sqlstore.db (lector)
//  4. identidad.Abrir sobre la ruta de identidad            — crea identidad.db  ← paso corregido
//  5. AbrirConexionDeRespaldo sobre la ruta de identidad    — consume identidad.db (lector) ← depende de 4
//  6. outbox.Abrir sobre la ruta del outbox                  — crea outbox.db
//
// Auditoría completa de la secuencia (incluyendo el resto de main()) registrada en D-43
// junto con los descartes de scope de esta tarea.
func abrirRecursosDeArranque(ctx context.Context, cfg configuracion.Configuracion, reg *registro.Registro) (*recursosDeArranque, error) {
	contenedor, err := canal.AbrirAlmacenDeDispositivo(ctx, cfg.RutaSqlstore, reg)
	if err != nil {
		return nil, err
	}

	sesion, err := canal.NuevaSesion(ctx, contenedor, reg)
	if err != nil {
		return nil, err
	}

	dbRespaldo, err := canal.AbrirConexionDeRespaldo(cfg.RutaSqlstore)
	if err != nil {
		return nil, err
	}

	// Crea identidad.db ANTES de que la conexión de respaldo de solo lectura intente abrirla.
	// Esta línea y la siguiente deben permanecer en instrucciones separadas para que la
	// guarda por mutación (HEX-066, AC-2) pueda localizarlas por coincidencia de contenido.
	almacenIdentidad, err := identidad.Abrir(identidad.Opciones{Ruta: cfg.RutaIdentidad, Registro: reg})
	if err != nil {
		return nil, err
	}

	dbRespaldoIdentidad, err := canal.AbrirConexionDeRespaldo(cfg.RutaIdentidad)
	if err != nil {
		return nil, err
	}

	buzon, err := outbox.Abrir(outbox.Opciones{Ruta: cfg.RutaOutbox, Registro: reg})
	if err != nil {
		return nil, err
	}

	return &recursosDeArranque{
		Contenedor:          contenedor,
		Sesion:              sesion,
		DBRespaldo:          dbRespaldo,
		DBRespaldoIdentidad: dbRespaldoIdentidad,
		AlmacenIdentidad:    almacenIdentidad,
		Buzon:               buzon,
	}, nil
}
