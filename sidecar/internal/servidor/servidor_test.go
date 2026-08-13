package servidor_test

import (
	"bufio"
	"bytes"
	"context"
	"database/sql"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"testing"

	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/outbox"
	"github.com/CGary/hexcell/sidecar/internal/registro"
	"github.com/CGary/hexcell/sidecar/internal/servidor"
	_ "modernc.org/sqlite"
)

// bufferSeguro permite capturar logs en tests concurrentes con el detector de carreras activo.
type bufferSeguro struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *bufferSeguro) Write(p []byte) (n int, err error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *bufferSeguro) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

func abrirBuzonPrueba(t *testing.T) *outbox.Outbox {
	ruta := filepath.Join(t.TempDir(), "test_outbox.db")
	buzon, err := outbox.Abrir(outbox.Opciones{Ruta: ruta})
	if err != nil {
		t.Fatalf("error abriendo outbox de prueba: %v", err)
	}
	t.Cleanup(func() { _ = buzon.Cerrar() })
	return buzon
}

func abrirDbRespaldoPrueba(t *testing.T) (*sql.DB, string) {
	ruta := filepath.Join(t.TempDir(), "sqlstore.db")
	dsn := fmt.Sprintf("file:%s?mode=ro&_pragma=busy_timeout(5000)", ruta)

	// Crear el archivo con esquema y user_version primero
	dbEscritura, err := sql.Open("sqlite", ruta)
	if err != nil {
		t.Fatalf("error creando sqlstore: %v", err)
	}
	if _, err := dbEscritura.Exec("PRAGMA user_version = 1; CREATE TABLE test (id INT);"); err != nil {
		t.Fatalf("error inicializando sqlstore: %v", err)
	}
	_ = dbEscritura.Close()

	dbLectura, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatalf("error abriendo conexión de respaldo: %v", err)
	}
	t.Cleanup(func() { _ = dbLectura.Close() })
	return dbLectura, ruta
}

func TestEscucharStaleSocketRama1Y3Inexistente(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
	})

	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("error inesperado al escuchar en ruta inexistente: %v", err)
	}
	defer srv.Cerrar()

	info, err := os.Stat(socketPath)
	if err != nil {
		t.Fatalf("no se creó el archivo del socket: %v", err)
	}
	if info.Mode().Perm() != 0600 {
		t.Fatalf("permisos del socket inválidos: esperado 0600, obtenido %o", info.Mode().Perm())
	}
}

func TestEscucharStaleSocketRama2OtroSidecarActivo(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	buf := &bufferSeguro{}
	reg := registro.Nuevo(buf, slog.LevelInfo, "test-cell")

	srv1 := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell-1",
		Registro:   reg,
	})
	if err := srv1.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al arrancar primer servidor: %v", err)
	}
	defer srv1.Cerrar()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go srv1.Aceptar(ctx)

	// Segundo servidor intenta arrancar en la misma ruta
	srv2 := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell-2",
		Registro:   reg,
	})

	err := srv2.Escuchar(context.Background())
	if !errors.Is(err, servidor.ErrOtroSidecarActivo) {
		t.Fatalf("se esperaba ErrOtroSidecarActivo, obtenido: %v", err)
	}

	// Comprobar que el archivo del socket sigue existiendo intacto
	if _, err := os.Stat(socketPath); err != nil {
		t.Fatalf("el socket del primer servidor fue eliminado erróneamente: %v", err)
	}
}

func TestEscucharStaleSocketRama3HuerfanoRechazado(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")

	// Crear un socket huérfano real en el kernel de Linux: bind sin listen y cerrar descriptor
	fd, err := syscall.Socket(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		t.Fatalf("error creando socket raw: %v", err)
	}
	addr := &syscall.SockaddrUnix{Name: socketPath}
	if err := syscall.Bind(fd, addr); err != nil {
		_ = syscall.Close(fd)
		t.Fatalf("error en bind de socket huérfano: %v", err)
	}
	_ = syscall.Close(fd)

	// El archivo socket existe en disco pero nadie escucha (conexión rechazada)
	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
	})

	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("error al reutilizar socket huérfano: %v", err)
	}
	defer srv.Cerrar()

	info, err := os.Stat(socketPath)
	if err != nil {
		t.Fatalf("el socket no existe tras re-escucha: %v", err)
	}
	if info.Mode().Perm() != 0600 {
		t.Fatalf("permisos de socket inválidos: %o", info.Mode().Perm())
	}
}

// TestEscucharStaleSocketRama4ErrorDePermisos ejercita realmente la rama 4 del procedimiento
// (sondeo con error distinto de ECONNREFUSED y de "no existe" -> abortar sin borrar nada).
//
// Un directorio 0500 no sirve: net.Dial falla con ENOENT (el socket no existe dentro), lo que
// esConexionRechazadaONoExiste clasifica como rama 3, y el error observado termina viniendo del
// ListenUnix posterior (EACCES), no del aborto de la rama 4. Para forzar EACCES en el sondeo
// mismo se crea el archivo del socket con el directorio contenedor transitable (0700) y luego se
// le quita el bit de ejecución (0600): el kernel ya no puede atravesar el directorio ni para
// conectar ni para hacer stat, así que tanto net.Dial como os.Stat fallan con EACCES, ninguno de
// los dos con "no existe". root ignora los bits de permiso, así que la prueba se omite bajo root.
func TestEscucharStaleSocketRama4ErrorDePermisos(t *testing.T) {
	t.Parallel()
	if os.Geteuid() == 0 {
		t.Skip("se omite: root ignora los bits de permiso del directorio")
	}

	tempDir := t.TempDir()
	caja := filepath.Join(tempDir, "caja")
	if err := os.Mkdir(caja, 0700); err != nil {
		t.Fatalf("error creando directorio contenedor: %v", err)
	}

	socketPath := filepath.Join(caja, "sidecar.sock")

	// Crear un socket huérfano real en el kernel (bind sin listen, luego cerrar descriptor)
	// mientras el directorio aún es transitable.
	fd, err := syscall.Socket(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		t.Fatalf("error creando socket raw: %v", err)
	}
	addr := &syscall.SockaddrUnix{Name: socketPath}
	if err := syscall.Bind(fd, addr); err != nil {
		_ = syscall.Close(fd)
		t.Fatalf("error en bind de socket huérfano: %v", err)
	}
	_ = syscall.Close(fd)

	// Quitar el bit de ejecución del directorio contenedor: ahora ni conectar ni hacer stat
	// pueden atravesarlo, y el error resultante es EACCES, no "no existe".
	if err := os.Chmod(caja, 0600); err != nil {
		t.Fatalf("error quitando bit de ejecución: %v", err)
	}
	t.Cleanup(func() { _ = os.Chmod(caja, 0700) })

	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
	})

	errEscuchar := srv.Escuchar(context.Background())
	if errEscuchar == nil {
		t.Fatal("se esperaba error ante fallo de sondeo por permisos (rama 4)")
	}
	if errors.Is(errEscuchar, servidor.ErrOtroSidecarActivo) {
		t.Fatalf("no debía reportarse ErrOtroSidecarActivo ante error de permisos: %v", errEscuchar)
	}

	// Restaurar el bit de ejecución para poder comprobar que el socket preexistente
	// sigue en disco: la rama 4 debe abortar sin borrar nada.
	if err := os.Chmod(caja, 0700); err != nil {
		t.Fatalf("error restaurando bit de ejecución para verificación: %v", err)
	}
	if _, err := os.Stat(socketPath); err != nil {
		t.Fatalf("el socket preexistente fue eliminado o quedó inaccesible tras rama 4: %v", err)
	}
}

func TestSaludoDesajusteDeVersionCierraConexionYRegistraAmbas(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	buf := &bufferSeguro{}
	reg := registro.Nuevo(buf, slog.LevelInfo, "test-cell")

	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
		Registro:   reg,
	})
	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al escuchar: %v", err)
	}
	defer srv.Cerrar()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go srv.Aceptar(ctx)

	conn, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("fallo conectando cliente: %v", err)
	}
	defer conn.Close()

	// Enviar saludo con versión 3 incompatible
	lineaInvalida := `{"version":3,"tipo":"saludo","emisor":"nucleo","id_celula":"test-cell"}` + "\n"
	if _, err := conn.Write([]byte(lineaInvalida)); err != nil {
		t.Fatalf("error enviando saludo incompatible: %v", err)
	}

	// El servidor debe cerrar la conexión (EOF al leer)
	lector := bufio.NewReader(conn)
	_, errRead := lector.ReadBytes('\n')
	if !errors.Is(errRead, io.EOF) {
		t.Fatalf("se esperaba cierre de conexión (EOF), obtenido: %v", errRead)
	}

	// Verificar que el registro contiene ambas versiones (3 y 4)
	salidaLog := buf.String()
	if !strings.Contains(salidaLog, "recibida 3") || !strings.Contains(salidaLog, "esperada 4") {
		t.Fatalf("el registro no contiene ambas versiones: %s", salidaLog)
	}
}

func TestErrorDeProtocoloLineaMalformadaCierraConexion(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
	})
	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al escuchar: %v", err)
	}
	defer srv.Cerrar()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go srv.Aceptar(ctx)

	conn, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("error conectando: %v", err)
	}
	defer conn.Close()

	if _, err := conn.Write([]byte("no-es-json\n")); err != nil {
		t.Fatalf("error escribiendo: %v", err)
	}

	lector := bufio.NewReader(conn)
	_, err = lector.ReadBytes('\n')
	if !errors.Is(err, io.EOF) {
		t.Fatalf("se esperaba EOF por desconexión ante error de protocolo, obtenido: %v", err)
	}

	// El socket file sigue existiendo intacto
	if _, err := os.Stat(socketPath); err != nil {
		t.Fatalf("el socket file fue eliminado erróneamente: %v", err)
	}
}

func TestTakeoverConexionMasRecienteGana(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
	})
	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al escuchar: %v", err)
	}
	defer srv.Cerrar()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go srv.Aceptar(ctx)

	// Cliente 1 conecta y hace handshake
	c1, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("error conectando cliente 1: %v", err)
	}
	defer c1.Close()

	saludoNucleo, _ := ipc.Codificar(ipc.NuevoSobre(ipc.Saludo{Emisor: ipc.EmisorNucleo, IdCelula: "test-cell"}))
	if _, err := c1.Write(saludoNucleo); err != nil {
		t.Fatalf("error enviando saludo c1: %v", err)
	}
	lector1 := bufio.NewReader(c1)
	resp1, err := lector1.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo respuesta saludo c1: %v", err)
	}
	sobre1, err := ipc.Decodificar(resp1)
	if err != nil || sobre1.Tipo != ipc.TipoSaludo {
		t.Fatalf("saludo de servidor c1 inválido: %v, sobre=%+v", err, sobre1)
	}

	// Cliente 2 conecta y hace handshake (takeover)
	c2, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("error conectando cliente 2: %v", err)
	}
	defer c2.Close()

	if _, err := c2.Write(saludoNucleo); err != nil {
		t.Fatalf("error enviando saludo c2: %v", err)
	}
	lector2 := bufio.NewReader(c2)
	resp2, err := lector2.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo respuesta saludo c2: %v", err)
	}
	sobre2, err := ipc.Decodificar(resp2)
	if err != nil || sobre2.Tipo != ipc.TipoSaludo {
		t.Fatalf("saludo de servidor c2 inválido: %v, sobre=%+v", err, sobre2)
	}

	// Cliente 1 debe observar desconexión (EOF)
	_, errEof1 := lector1.ReadBytes('\n')
	if !errors.Is(errEof1, io.EOF) {
		t.Fatalf("se esperaba EOF en cliente 1 tras relevo, obtenido: %v", errEof1)
	}

	// Cliente 2 sigue vivo y puede recibir notificaciones
	srv.EnviarEstadoSesion(ipc.EstadoSesion{
		Estado:     ipc.EstadoActiva,
		Causa:      "",
		Codigo:     0,
		ExpiraEnMs: 0,
	})

	lineaEstado, err := lector2.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo estado en cliente 2: %v", err)
	}
	sobreEstado, err := ipc.Decodificar(lineaEstado)
	if err != nil || sobreEstado.Tipo != ipc.TipoEstadoSesion {
		t.Fatalf("estado en cliente 2 inválido: %v, sobre=%+v", err, sobreEstado)
	}
}

func TestBucleCompletoSobreSocket(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	buzon := abrirBuzonPrueba(t)
	dbRespaldo, _ := abrirDbRespaldoPrueba(t)

	// Persistir un evento entrante previo en el Outbox (no confirmado)
	evtPrevio := ipc.EventoEntrante{
		IdDeduplicacion: "dedup-1",
		IdConversacion:  "conv-1",
		IdRemitente:     "remitente-1",
		Contenido:       "hola previo",
		MarcaTemporalMs: 1000,
	}
	sobrePrevio := ipc.NuevoSobre(evtPrevio)
	cargaPrevia, err := ipc.Codificar(sobrePrevio)
	if err != nil {
		t.Fatalf("error codificando evento previo: %v", err)
	}
	if err := buzon.Persistir(context.Background(), "dedup-1", cargaPrevia); err != nil {
		t.Fatalf("error persistiendo evento previo: %v", err)
	}

	// Portero y cola de salida
	colaSalida := outbox.NuevaColaDeSalida(buzon.DB(), 1000, 3, nil, nil, nil)
	portero := outbox.NuevoPorteroDeSalida(colaSalida, nil, nil, nil, nil)

	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
		Buzon:      buzon,
		Portero:    portero,
		DBRespaldo: dbRespaldo,
	})
	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al escuchar: %v", err)
	}
	defer srv.Cerrar()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go srv.Aceptar(ctx)

	conn, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("error conectando cliente: %v", err)
	}
	defer conn.Close()

	lector := bufio.NewReader(conn)

	// 1. Handshake de saludo
	saludoNucleo, _ := ipc.Codificar(ipc.NuevoSobre(ipc.Saludo{Emisor: ipc.EmisorNucleo, IdCelula: "test-cell"}))
	if _, err := conn.Write(saludoNucleo); err != nil {
		t.Fatalf("error enviando saludo: %v", err)
	}
	respSaludo, err := lector.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error recibiendo saludo servidor: %v", err)
	}
	sobreSrv, err := ipc.Decodificar(respSaludo)
	if err != nil || sobreSrv.Tipo != ipc.TipoSaludo {
		t.Fatalf("saludo servidor inválido: %v, sobre=%+v", err, sobreSrv)
	}

	// 2. Recibir redistribución del evento no confirmado
	lineaEvt, err := lector.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error recibiendo evento redistribuido: %v", err)
	}
	sobreEvt, err := ipc.Decodificar(lineaEvt)
	if err != nil || sobreEvt.Tipo != ipc.TipoEventoEntrante {
		t.Fatalf("evento redistribuido inválido: %v, sobre=%+v", err, sobreEvt)
	}
	evtRecibido, ok := sobreEvt.Cuerpo.(ipc.EventoEntrante)
	if !ok || evtRecibido.IdDeduplicacion != "dedup-1" {
		t.Fatalf("id_deduplicacion incorrecto: %+v", evtRecibido)
	}

	// 3. Enviar mensaje_saliente
	msgSaliente, _ := ipc.Codificar(ipc.NuevoSobre(ipc.MensajeSaliente{
		IdMensaje:             "msg-1",
		IdConversacion:        "conv-1",
		Contenido:             "hola saliente",
		MarcaTemporalOrigenMs: 2000,
	}))
	if _, err := conn.Write(msgSaliente); err != nil {
		t.Fatalf("error enviando mensaje saliente: %v", err)
	}

	// 4. Enviar confirmación del evento entrante
	confBytes, _ := ipc.Codificar(ipc.NuevoSobre(ipc.Confirmacion{IdDeduplicacion: "dedup-1"}))
	if _, err := conn.Write(confBytes); err != nil {
		t.Fatalf("error enviando confirmacion: %v", err)
	}

	// 5. Enviar orden_respaldo_sqlstore (su respuesta en el mismo socket garantiza que los anteriores fueron leídos)
	dirDestino := t.TempDir()
	ordenRespaldo, _ := ipc.Codificar(ipc.NuevoSobre(ipc.OrdenRespaldoSqlstore{
		Orden:                ipc.OrdenRespaldarSqlstore,
		Destino:              dirDestino,
		IdentificadorDeRonda: "ronda-test-1",
	}))
	if _, err := conn.Write(ordenRespaldo); err != nil {
		t.Fatalf("error enviando orden respaldo: %v", err)
	}

	// Recibir acuse_respaldo_sqlstore
	lineaAcuse, err := lector.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error recibiendo acuse respaldo: %v", err)
	}
	sobreAcuse, err := ipc.Decodificar(lineaAcuse)
	if err != nil || sobreAcuse.Tipo != ipc.TipoAcuseRespaldoSqlstore {
		t.Fatalf("acuse respaldo inválido: %v, sobre=%+v", err, sobreAcuse)
	}
	acuseRespaldo := sobreAcuse.Cuerpo.(ipc.AcuseRespaldoSqlstore)
	if acuseRespaldo.Resultado != ipc.ResultadoCompletado || acuseRespaldo.IdentificadorDeRonda != "ronda-test-1" {
		t.Fatalf("acuse respaldo fallido: %+v", acuseRespaldo)
	}

	// Comprobar que mensaje_saliente aterrizó en cola_salida
	var countSalida int
	buzon.DB().QueryRow("SELECT COUNT(*) FROM cola_salida WHERE id_mensaje='msg-1'").Scan(&countSalida)
	if countSalida != 1 {
		t.Fatalf("el mensaje saliente no se persistió en cola_salida: count=%d", countSalida)
	}

	// Comprobar que la confirmación procesó el evento en outbox
	pendientes, err := buzon.Pendientes(context.Background())
	if err != nil || len(pendientes) != 0 {
		t.Fatalf("el evento confirmado sigue pendiente en outbox: %d entradas", len(pendientes))
	}

	// 6. Enviar estado_sesion y acuse_envio desde el servidor hacia el cliente
	srv.EnviarEstadoSesion(ipc.EstadoSesion{Estado: ipc.EstadoActiva})
	srv.EnviarAcuseEnvio(ipc.AcuseEnvio{IdMensaje: "msg-1", Estado: ipc.EstadoEnvioEnviado, MarcaTemporalMs: 3000})

	lineaSesion, err := lector.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo estado sesion: %v", err)
	}
	sobreSesion, _ := ipc.Decodificar(lineaSesion)
	if sobreSesion.Tipo != ipc.TipoEstadoSesion {
		t.Fatalf("tipo incorrecto para estado sesion: %s", sobreSesion.Tipo)
	}

	lineaAcuseEnv, err := lector.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo acuse envio: %v", err)
	}
	sobreAcuseEnv, _ := ipc.Decodificar(lineaAcuseEnv)
	if sobreAcuseEnv.Tipo != ipc.TipoAcuseEnvio {
		t.Fatalf("tipo incorrecto para acuse envio: %s", sobreAcuseEnv.Tipo)
	}
}

func TestDesconexionDeClienteNoAfectaOperacionDelSidecarNiSesion(t *testing.T) {
	t.Parallel()
	socketPath := filepath.Join(t.TempDir(), "ipc.sock")
	buzon := abrirBuzonPrueba(t)

	srv := servidor.NuevoServidor(servidor.Dependencias{
		RutaSocket: socketPath,
		IdCelula:   "test-cell",
		Buzon:      buzon,
	})
	if err := srv.Escuchar(context.Background()); err != nil {
		t.Fatalf("fallo al escuchar: %v", err)
	}
	defer srv.Cerrar()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go srv.Aceptar(ctx)

	// Conectar cliente 1, hacer saludo, luego desconectar
	c1, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("error conectando cliente 1: %v", err)
	}
	saludo, _ := ipc.Codificar(ipc.NuevoSobre(ipc.Saludo{Emisor: ipc.EmisorNucleo, IdCelula: "test-cell"}))
	_, _ = c1.Write(saludo)
	lector1 := bufio.NewReader(c1)
	_, _ = lector1.ReadBytes('\n')
	_ = c1.Close()

	// Mientras no hay ningún cliente conectado: persistir nuevo evento y notificar trabajo
	evt := ipc.EventoEntrante{
		IdDeduplicacion: "dedup-offline",
		IdConversacion:  "conv-1",
		IdRemitente:     "rem-1",
		Contenido:       "offline",
		MarcaTemporalMs: 5000,
	}
	carga, _ := ipc.Codificar(ipc.NuevoSobre(evt))
	if err := buzon.Persistir(context.Background(), "dedup-offline", carga); err != nil {
		t.Fatalf("error persistiendo evento offline: %v", err)
	}
	srv.NotificarTrabajo()

	// Reconectar con cliente 2
	c2, err := net.Dial("unix", socketPath)
	if err != nil {
		t.Fatalf("error reconectando cliente 2: %v", err)
	}
	defer c2.Close()

	_, _ = c2.Write(saludo)
	lector2 := bufio.NewReader(c2)

	// Leer saludo del servidor
	respSaludo, err := lector2.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo saludo tras reconexión: %v", err)
	}
	sobreSrv, _ := ipc.Decodificar(respSaludo)
	if sobreSrv.Tipo != ipc.TipoSaludo {
		t.Fatalf("saludo de servidor esperado, recibido: %s", sobreSrv.Tipo)
	}

	// Leer redistribución del evento offline
	lineaEvt, err := lector2.ReadBytes('\n')
	if err != nil {
		t.Fatalf("error leyendo evento offline redistribuido: %v", err)
	}
	sobreEvt, _ := ipc.Decodificar(lineaEvt)
	if sobreEvt.Tipo != ipc.TipoEventoEntrante {
		t.Fatalf("evento_entrante esperado, recibido: %s", sobreEvt.Tipo)
	}
	evtRecibido := sobreEvt.Cuerpo.(ipc.EventoEntrante)
	if evtRecibido.IdDeduplicacion != "dedup-offline" {
		t.Fatalf("evento redistribuido incorrecto: %+v", evtRecibido)
	}
}
