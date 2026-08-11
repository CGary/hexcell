package outbox

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"sync/atomic"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

// MotivoDeAplazamiento identifica la razón de disciplina por la cual una salida se posterga.
type MotivoDeAplazamiento int

const (
	// MotivoNinguno indica que la salida no fue aplazada y puede transmitirse.
	MotivoNinguno MotivoDeAplazamiento = iota
	// MotivoLatenciaMinima indica aplazamiento por no haber transcurrido el suelo de latencia mínima.
	// [causa documentada]
	MotivoLatenciaMinima
	// MotivoFueraDeHorario indica aplazamiento por estar fuera de la ventana de atención configurada.
	// [causa documentada]
	MotivoFueraDeHorario
	// MotivoRampaDeVolumen indica aplazamiento por haber alcanzado el cupo diario de la rampa de volumen.
	// [precautorio]
	MotivoRampaDeVolumen
)

var (
	// ContadorAplazadasPorLatencia cuenta los aplazamientos debidos al suelo de latencia mínima.
	// [causa documentada]
	ContadorAplazadasPorLatencia atomic.Int64

	// ContadorAplazadasPorHorario cuenta los aplazamientos debidos a la ventana de atención comercial.
	// [causa documentada]
	ContadorAplazadasPorHorario atomic.Int64

	// ContadorAplazadasPorRampa cuenta los aplazamientos debidos al tope de la rampa de volumen.
	// [precautorio]
	ContadorAplazadasPorRampa atomic.Int64
)

// DisciplinaDeSalida es la compuerta de disciplina de salida previa a la transmisión.
//
// Aplica tres medidas de adr-0015:
//  1. Suelo de latencia mínima [causa documentada]: retención mínima medida desde marca_temporal_origen_ms.
//     Limitación: mide el suelo de retención en el sidecar, no la latencia extremo a extremo del turno.
//  2. Ventana de atención [causa documentada]: restricción a horario comercial y días hábiles configurados.
//  3. Rampa de volumen [precautorio]: contención de daño mediante un cupo diario escalonado por semanas.
type DisciplinaDeSalida struct {
	latenciaMinimaMs int64
	ventana          configuracion.VentanaDeAtencion
	rampa            configuracion.RampaDeVolumen
}

// NuevaDisciplinaDeSalida construye la compuerta de disciplina a partir de la configuración validada.
func NuevaDisciplinaDeSalida(cfg configuracion.Disciplina) *DisciplinaDeSalida {
	latencia := cfg.LatenciaMinimaMs
	if latencia <= 0 {
		latencia = configuracion.LatenciaMinimaMsPorOmision
	}
	return &DisciplinaDeSalida{
		latenciaMinimaMs: latencia,
		ventana:          cfg.Ventana,
		rampa:            cfg.Rampa,
	}
}

// Decidir evalúa de forma pura si un mensaje puede transmitirse en este instante o debe postergarse.
// No interactúa directamente con la base de datos: recibe los conteos persistidos como parámetros.
// Sin guarda de "valor en cero": configuracion.Cargar ya rechaza todo valor degenerado, así que
// una guarda adicional aquí solo abriría una vía de bypass silencioso sin error ni registro.
func (d *DisciplinaDeSalida) Decidir(ahoraMs, origenMs, enviadosHoy, primerDiaMs int64) (bool, MotivoDeAplazamiento) {
	// 1. Evaluación de ventana de atención [causa documentada]
	t := time.UnixMilli(ahoraMs).In(d.ventana.Zona)
	diaIso := int(t.Weekday())
	if diaIso == 0 {
		diaIso = 7
	}

	diaValido := false
	for _, dv := range d.ventana.Dias {
		if dv == diaIso {
			diaValido = true
			break
		}
	}
	if !diaValido {
		return false, MotivoFueraDeHorario
	}

	minutoActual := t.Hour()*60 + t.Minute()
	minutoApertura := d.ventana.HoraApertura*60 + d.ventana.MinutoApertura
	minutoCierre := d.ventana.HoraCierre*60 + d.ventana.MinutoCierre

	if minutoActual < minutoApertura || minutoActual >= minutoCierre {
		return false, MotivoFueraDeHorario
	}

	// 2. Evaluación de rampa de volumen [precautorio]
	var semana int64
	if primerDiaMs > 0 && ahoraMs >= primerDiaMs {
		duracionMs := ahoraMs - primerDiaMs
		semana = duracionMs / (7 * 24 * 60 * 60 * 1000)
		if semana > d.rampa.Semanas {
			semana = d.rampa.Semanas
		}
	}
	cupoHoy := d.rampa.DiariaInicial + semana*d.rampa.IncrementoSemanal
	if enviadosHoy >= cupoHoy {
		return false, MotivoRampaDeVolumen
	}

	// 3. Evaluación de latencia mínima [causa documentada]. Ningún aplazamiento consume un intento
	// (intentarUno regresa antes de Transmitir); uno que sobreviva al TTL se descarta con dureza
	// por descartarExpiradas, la limitación de daño deliberada del discard D-13 ("a determinar").
	if ahoraMs-origenMs < d.latenciaMinimaMs {
		return false, MotivoLatenciaMinima
	}

	return true, MotivoNinguno
}

// RegistrarEnvioDelDia incrementa atómicamente el contador de envíos para la fecha dada, en la
// zona horaria que le pase el llamador (la de la ventana de atención, no necesariamente UTC).
func RegistrarEnvioDelDia(ctx context.Context, db *sql.DB, dia string) error {
	query := `INSERT INTO volumen_diario (dia, envios) VALUES (?, 1) ON CONFLICT(dia) DO UPDATE SET envios = envios + 1`
	res, err := db.ExecContext(ctx, query, dia)
	if err != nil {
		return fmt.Errorf("outbox: error al registrar envio del dia: %w", err)
	}
	if _, err := res.RowsAffected(); err != nil {
		return fmt.Errorf("outbox: error al obtener filas afectadas al registrar envio del dia: %w", err)
	}
	return nil
}

// EnviosDelDia consulta el total de mensajes transmitidos en la fecha UTC especificada.
func EnviosDelDia(ctx context.Context, db *sql.DB, dia string) (int64, error) {
	query := `SELECT envios FROM volumen_diario WHERE dia = ?`
	var envios int64
	err := db.QueryRowContext(ctx, query, dia).Scan(&envios)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, nil
	}
	if err != nil {
		return 0, fmt.Errorf("outbox: error al consultar envios del dia: %w", err)
	}
	return envios, nil
}

// PrimerDiaConEnvios devuelve la marca temporal en milisegundos (inicio del día UTC) del primer registro con envíos.
func PrimerDiaConEnvios(ctx context.Context, db *sql.DB) (int64, error) {
	query := `SELECT MIN(dia) FROM volumen_diario WHERE envios > 0`
	var minDia sql.NullString
	err := db.QueryRowContext(ctx, query).Scan(&minDia)
	if errors.Is(err, sql.ErrNoRows) || !minDia.Valid || minDia.String == "" {
		return 0, nil
	}
	if err != nil {
		return 0, fmt.Errorf("outbox: error al consultar primer dia con envios: %w", err)
	}
	t, err := time.ParseInLocation("2006-01-02", minDia.String, time.UTC)
	if err != nil {
		return 0, fmt.Errorf("outbox: error al parsear primer dia con envios %q: %w", minDia.String, err)
	}
	return t.UnixMilli(), nil
}

// diaEnZonaDeLaVentana formatea ahoraMs como AAAA-MM-DD en la zona de la ventana de atención (la
// ventana se evalúa en esa zona, así que el cupo diario debe coincidir con ella); UTC es respaldo.
func (c *ColaDeSalida) diaEnZonaDeLaVentana(ahoraMs int64) string {
	zona := time.UTC
	if c.disciplina != nil && c.disciplina.ventana.Zona != nil {
		zona = c.disciplina.ventana.Zona
	}
	return time.UnixMilli(ahoraMs).In(zona).Format("2006-01-02")
}

// verificarDisciplina consulta los contadores persistidos, evalúa la compuerta pura de disciplina
// y actualiza los contadores de métricas y presencia correspondientes. Un error de lectura de
// cualquiera de los dos contadores falla CERRADO -aplaza como si el cupo de rampa estuviera
// agotado- en vez de asumir enviadosHoy=0 y dejar que el tope diario deje de aplicarse en
// silencio: un mensaje aplazado un intervalo de drenaje es recuperable, un envío sobre cupo no.
func (c *ColaDeSalida) verificarDisciplina(ctx context.Context, idConversacion, idMensaje string, origenMs, ahoraMs int64) bool {
	dia := c.diaEnZonaDeLaVentana(ahoraMs)
	enviadosHoy, errEnvios := EnviosDelDia(ctx, c.db, dia)
	primerDiaMs, errPrimerDia := PrimerDiaConEnvios(ctx, c.db)
	if errEnvios != nil || errPrimerDia != nil {
		if c.registro != nil {
			detalle := errEnvios
			if detalle == nil {
				detalle = errPrimerDia
			}
			c.registro.Error("outbox.error_consultar_rampa", registro.Campos{IdEvento: idMensaje, Detalle: detalle.Error()})
		}
		ContadorAplazadasPorRampa.Add(1)
		return false
	}

	transmitir, motivo := c.disciplina.Decidir(ahoraMs, origenMs, enviadosHoy, primerDiaMs)
	if transmitir {
		return true
	}

	switch motivo {
	case MotivoLatenciaMinima:
		ContadorAplazadasPorLatencia.Add(1)
		c.anunciarPresencia(ctx, idConversacion, idMensaje)
	case MotivoFueraDeHorario:
		ContadorAplazadasPorHorario.Add(1)
	case MotivoRampaDeVolumen:
		ContadorAplazadasPorRampa.Add(1)
	}
	return false
}

func (c *ColaDeSalida) anunciarPresencia(ctx context.Context, idConversacion, idMensaje string) {
	if c.emisorPresencia == nil {
		return
	}
	c.muPresencia.Lock()
	if _, ok := c.presenciasAnunciadas[idMensaje]; ok {
		c.muPresencia.Unlock()
		return
	}
	c.presenciasAnunciadas[idMensaje] = struct{}{}
	c.muPresencia.Unlock()
	_ = c.emisorPresencia.EmitirEscribiendo(ctx, idConversacion)
}

// liberarPresencia borra el marcador en memoria de una fila que ya salió de la cola -enviada o
// descartada-, para que presenciasAnunciadas no crezca sin límite durante la vida del proceso.
func (c *ColaDeSalida) liberarPresencia(idMensaje string) {
	c.muPresencia.Lock()
	delete(c.presenciasAnunciadas, idMensaje)
	c.muPresencia.Unlock()
}
