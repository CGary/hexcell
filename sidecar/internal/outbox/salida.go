package outbox

import (
	"context"
	"database/sql"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"github.com/CGary/hexcell/sidecar/internal/configuracion"
	"github.com/CGary/hexcell/sidecar/internal/ipc"
	"github.com/CGary/hexcell/sidecar/internal/registro"
)

const (
	// TtlPorOmision e IntentosMaximosPorOmision reexportan, sin repetirlos, los mismos literales
	// que configuracion.Cargar usa para poblar Configuracion.TtlSalidaMs e
	// IntentosMaximosSalida: configuracion.TtlSalidaMsPorOmision y
	// configuracion.IntentosMaximosSalidaPorOmision son la única fuente de verdad de ambos
	// valores. Calibrar de nuevo cualquiera de los dos significa editar un solo sitio.
	TtlPorOmision             = configuracion.TtlSalidaMsPorOmision
	IntentosMaximosPorOmision = configuracion.IntentosMaximosSalidaPorOmision

	EventoSalidaExpirada    = "outbox.salida_expirada"
	EventoSalidaEnviada     = "outbox.salida_enviada"
	EventoSalidaReintentada = "outbox.salida_reintentada"
	EventoSalidaAgotada     = "outbox.salida_agotada"

	// EventoSalidaDescartadaPorCortacircuitos se emite al descartar en el drenaje una fila ya
	// encolada antes de dispararse el cortacircuitos (fix cortacircuitos-descarte-en-drenaje,
	// ENMIENDA 2026-08-11 PAT-022).
	EventoSalidaDescartadaPorCortacircuitos = "outbox.salida_descartada_por_cortacircuitos"
)

// ContadorExpiradas cuenta, a través de todas las instancias del proceso, cuántos mensajes
// salientes se descartaron por expiración de TTL sin haberse transmitido jamás.
var ContadorExpiradas atomic.Int64

// ContadorDescartadasPorCortacircuitos cuenta, a través de todas las instancias del proceso,
// filas descartadas con dureza en el drenaje por haberse disparado el cortacircuitos mientras
// estaban encoladas (admitidas antes del disparo).
var ContadorDescartadasPorCortacircuitos atomic.Int64

// ContadorAplazadasPorErrorCortacircuitos cuenta, a través de todas las instancias del proceso,
// intentos de drenaje que aplazaron una fila -sin transmitirla ni descartarla- porque la
// consulta del estado del cortacircuitos falló (fallo cerrado, ver verificarDisciplina).
var ContadorAplazadasPorErrorCortacircuitos atomic.Int64

// SumideroDeAcuse recibe cada acuse de envío generado al transmitir o fallar un mensaje saliente.
type SumideroDeAcuse func(ipc.AcuseEnvio)

// ColaDeSalida es el buzón de salida durable: el único punto de entrada para encolar un
// mensaje saliente es Encolar, de modo que una futura lista STOP pueda anteponerse sin tocar
// el resto de esta cola.
type ColaDeSalida struct {
	db                   *sql.DB
	ttlMs                int64
	intentosMaximos      int64
	registro             *registro.Registro
	transmisor           Transmisor
	control              ControlDeBaja
	cortacircuitos       ControlDeCortacircuitos
	disciplina           *DisciplinaDeSalida
	emisorPresencia      EmisorDePresencia
	sumideroAcuse        SumideroDeAcuse
	muPresencia          sync.Mutex
	presenciasAnunciadas map[string]struct{}
}

// NuevaColaDeSalida construye la cola sobre una conexión ya abierta (la misma del outbox
// existente) con el TTL, el límite de reintentos, el transmisor y el control de baja inyectados.
// transmisor y control pueden ser nil en pruebas que no los ejercitan.
func NuevaColaDeSalida(db *sql.DB, ttlMs, intentosMaximos int64, reg *registro.Registro, transmisor Transmisor, control ControlDeBaja) *ColaDeSalida {
	if ttlMs <= 0 {
		ttlMs = TtlPorOmision
	}
	if intentosMaximos <= 0 {
		intentosMaximos = IntentosMaximosPorOmision
	}
	return &ColaDeSalida{
		db:                   db,
		ttlMs:                ttlMs,
		intentosMaximos:      intentosMaximos,
		registro:             reg,
		transmisor:           transmisor,
		control:              control,
		presenciasAnunciadas: make(map[string]struct{}),
	}
}

// ConDisciplina inyecta, tipados y explícitos (contract behavior 15, ambos nil-legales), la
// compuerta de disciplina y el emisor de presencia; devuelve la cola para encadenar.
func (c *ColaDeSalida) ConDisciplina(disciplina *DisciplinaDeSalida, emisorPresencia EmisorDePresencia) *ColaDeSalida {
	c.disciplina = disciplina
	c.emisorPresencia = emisorPresencia
	return c
}

// ConCortacircuitos inyecta, tipado y explícito (mismo patrón de ConDisciplina, nil-legal), el
// control de cortacircuitos que el drenaje re-consulta antes de transmitir una fila ya encolada,
// para que una respuesta admitida antes del disparo no se transmita después del traspaso
// (fix cortacircuitos-descarte-en-drenaje, ENMIENDA 2026-08-11 PAT-022). Devuelve la cola para
// encadenar.
func (c *ColaDeSalida) ConCortacircuitos(cortacircuitos ControlDeCortacircuitos) *ColaDeSalida {
	c.cortacircuitos = cortacircuitos
	return c
}

// ConSumideroDeAcuse inyecta la función de notificación para acuses de envío (nil-segura);
// devuelve la cola para encadenar.
func (c *ColaDeSalida) ConSumideroDeAcuse(sumidero SumideroDeAcuse) *ColaDeSalida {
	c.sumideroAcuse = sumidero
	return c
}

// PresenciasPendientesParaPruebas expone, solo para pruebas, cuántas entradas conserva en
// memoria el marcador de presencias anunciadas.
func (c *ColaDeSalida) PresenciasPendientesParaPruebas() int {
	c.muPresencia.Lock()
	defer c.muPresencia.Unlock()
	return len(c.presenciasAnunciadas)
}

// Encolar inserta un mensaje saliente de forma idempotente: un id_mensaje repetido no
// sobrescribe la fila existente ni produce un segundo envío.
func (c *ColaDeSalida) Encolar(ctx context.Context, idMensaje, idConversacion, contenido string, marcaTemporalOrigenMs int64) error {
	query := `INSERT INTO cola_salida (id_mensaje, id_conversacion, contenido, marca_temporal_origen_ms) VALUES (?, ?, ?, ?) ON CONFLICT(id_mensaje) DO NOTHING`
	_, err := c.db.ExecContext(ctx, query, idMensaje, idConversacion, contenido, marcaTemporalOrigenMs)
	if err != nil {
		return fmt.Errorf("cola de salida: error al encolar: %w", err)
	}
	return nil
}

// Drenar procesa la cola en dos pasadas separadas, ambas ancladas a la misma marca de "ahora":
//
//  1. descartarExpiradas elimina con dureza (hard discard) todo mensaje pendiente cuyo origen
//     ya superó el TTL, sin intentar transmitirlo jamás: el TTL siempre gana la carrera contra
//     un reintento, incluso si el mensaje llevaba menos de intentosMaximos fallos.
//  2. intentarPendientes selecciona lo que sigue pendiente y vigente e intenta transmitirlo a
//     través del Transmisor inyectado, acotando los reintentos a intentosMaximos. Al agotarse
//     el límite la fila se descarta igual que si hubiera expirado: no existe cola de reenvío,
//     tabla de cuarentena ni recuperación al arrancar para ninguno de los dos motivos de
//     descarte.
//
// El TTL se mide siempre contra c.ttlMs, el valor fijado en el constructor: Drenar no acepta un
// TTL como parámetro para que el llamador no pueda eludir el configurado.
func (c *ColaDeSalida) Drenar(ctx context.Context, ahoraMs int64) error {
	limiteMs := ahoraMs - c.ttlMs

	if err := c.descartarExpiradas(ctx, limiteMs); err != nil {
		return err
	}

	return c.intentarPendientes(ctx, limiteMs, ahoraMs)
}

// descartarExpiradas elimina físicamente toda fila no enviada cuyo origen ya superó el TTL y
// emite un evento de registro dedicado por cada mensaje descartado -no un conteo agregado-,
// para que el rastro de auditoría anti-baneo pueda identificar cuál mensaje concreto se perdió.
func (c *ColaDeSalida) descartarExpiradas(ctx context.Context, limiteMs int64) error {
	query := `DELETE FROM cola_salida WHERE enviado_en_ms IS NULL AND marca_temporal_origen_ms < ? RETURNING id_mensaje`
	filas, err := c.db.QueryContext(ctx, query, limiteMs)
	if err != nil {
		return fmt.Errorf("cola de salida: error al drenar expiradas: %w", err)
	}
	defer filas.Close()

	var expiradas int64
	for filas.Next() {
		var idMensaje string
		if err := filas.Scan(&idMensaje); err != nil {
			return fmt.Errorf("cola de salida: error al leer mensaje expirado: %w", err)
		}
		expiradas++
		c.liberarPresencia(idMensaje)
		if c.registro != nil {
			c.registro.Aviso(EventoSalidaExpirada, registro.Campos{
				IdEvento: idMensaje,
			})
		}
	}
	if err := filas.Err(); err != nil {
		return fmt.Errorf("cola de salida: error al iterar expiradas: %w", err)
	}

	if expiradas > 0 {
		ContadorExpiradas.Add(expiradas)
	}
	return nil
}

// pendienteSalida es la proyección mínima de una fila pendiente que intentarPendientes
// necesita para llamar al Transmisor y decidir si reintenta o abandona.
type pendienteSalida struct {
	idMensaje             string
	idConversacion        string
	contenido             string
	marcaTemporalOrigenMs int64
	intentos              int64
}

// intentarPendientes selecciona lo que sigue pendiente y vigente -no expirado contra el mismo
// límite que ya aplicó descartarExpiradas- e intenta transmitir cada fila. Sin un transmisor
// configurado no hay nada que intentar: la selección se omite en vez de fallar, para que las
// pruebas de solo expiración puedan seguir construyendo la cola con transmisor nil.
func (c *ColaDeSalida) intentarPendientes(ctx context.Context, limiteMs, ahoraMs int64) error {
	if c.transmisor == nil {
		return nil
	}

	query := `SELECT id_mensaje, id_conversacion, contenido, marca_temporal_origen_ms, intentos FROM cola_salida WHERE enviado_en_ms IS NULL AND marca_temporal_origen_ms >= ?`
	filas, err := c.db.QueryContext(ctx, query, limiteMs)
	if err != nil {
		return fmt.Errorf("cola de salida: error al seleccionar pendientes: %w", err)
	}

	var pendientes []pendienteSalida
	for filas.Next() {
		var p pendienteSalida
		if err := filas.Scan(&p.idMensaje, &p.idConversacion, &p.contenido, &p.marcaTemporalOrigenMs, &p.intentos); err != nil {
			filas.Close()
			return fmt.Errorf("cola de salida: error al leer mensaje pendiente: %w", err)
		}
		pendientes = append(pendientes, p)
	}
	if err := filas.Err(); err != nil {
		filas.Close()
		return fmt.Errorf("cola de salida: error al iterar pendientes: %w", err)
	}
	filas.Close()

	for _, p := range pendientes {
		c.intentarUno(ctx, p, ahoraMs)
	}
	return nil
}

// intentarUno intenta transmitir una única fila pendiente. Un éxito delega en MarcarEnviado,
// que ya es idempotente frente a una llamada repetida; un fallo cuenta como un intento más y
// puede agotar la fila. Si el contacto fue dado de baja, se descarta con dureza inmediatamente.
func (c *ColaDeSalida) intentarUno(ctx context.Context, p pendienteSalida, ahoraMs int64) {
	if c.control != nil {
		permitido, err := c.control.EnvioPermitido(ctx, p.idConversacion, p.idMensaje)
		if err != nil {
			if c.registro != nil {
				c.registro.Error("outbox.error_consultar_baja", registro.Campos{IdEvento: p.idMensaje, Detalle: err.Error()})
			}
			return
		}
		if !permitido {
			query := `DELETE FROM cola_salida WHERE id_mensaje = ? AND enviado_en_ms IS NULL`
			res, err := c.db.ExecContext(ctx, query, p.idMensaje)
			if err != nil {
				if c.registro != nil {
					c.registro.Error("outbox.error_descartar_baja", registro.Campos{IdEvento: p.idMensaje, Detalle: err.Error()})
				}
				return
			}
			if filas, _ := res.RowsAffected(); filas > 0 {
				ContadorDescartadasPorBaja.Add(filas)
				c.liberarPresencia(p.idMensaje)
				if c.registro != nil {
					c.registro.Aviso(EventoSalidaDescartadaPorBaja, registro.Campos{IdEvento: p.idMensaje})
				}
			}
			return
		}
	}

	// Recheque del cortacircuitos en el drenaje: espeja el recheque de baja de arriba porque la
	// admisión (Admitir) ya no basta -una respuesta admitida antes del disparo puede seguir
	// encolada cuando el cortacircuitos se dispara- así que sin este recheque el bot no queda en
	// silencio tras el traspaso. Un error de lectura falla CERRADO aplazando (nunca descartando)
	// la fila, igual que verificarDisciplina; el traspaso mismo pasa porque SalidaPermitida
	// siempre permite el id_mensaje_traspaso reclamado.
	// (fix cortacircuitos-descarte-en-drenaje, ENMIENDA 2026-08-11 PAT-022)
	if c.cortacircuitos != nil {
		permitido, err := c.cortacircuitos.SalidaPermitida(ctx, p.idConversacion, p.idMensaje)
		if err != nil {
			if c.registro != nil {
				c.registro.Error("outbox.error_consultar_cortacircuitos_en_drenaje", registro.Campos{IdEvento: p.idMensaje, Detalle: err.Error()})
			}
			ContadorAplazadasPorErrorCortacircuitos.Add(1)
			return
		}
		if !permitido {
			query := `DELETE FROM cola_salida WHERE id_mensaje = ? AND enviado_en_ms IS NULL`
			res, err := c.db.ExecContext(ctx, query, p.idMensaje)
			if err != nil {
				if c.registro != nil {
					c.registro.Error("outbox.error_descartar_cortacircuitos", registro.Campos{IdEvento: p.idMensaje, Detalle: err.Error()})
				}
				return
			}
			if filas, _ := res.RowsAffected(); filas > 0 {
				ContadorDescartadasPorCortacircuitos.Add(filas)
				c.liberarPresencia(p.idMensaje)
				if c.registro != nil {
					c.registro.Aviso(EventoSalidaDescartadaPorCortacircuitos, registro.Campos{IdEvento: p.idMensaje})
				}
			}
			return
		}
	}

	// c.disciplina nil es legal explícitamente (contract behavior 15): ver Decidir sobre D-13.
	if c.disciplina != nil {
		if !c.verificarDisciplina(ctx, p.idConversacion, p.idMensaje, p.marcaTemporalOrigenMs, ahoraMs) {
			return
		}
	}

	idCorrelacion, err := c.transmisor.Transmitir(ctx, p.idConversacion, p.contenido)
	if err != nil {
		c.registrarFallo(ctx, p)
		return
	}

	if err := c.MarcarEnviado(ctx, p.idMensaje, idCorrelacion, ahoraMs); err != nil && c.registro != nil {
		c.registro.Error("outbox.error_marcar_enviado", registro.Campos{
			IdEvento: p.idMensaje,
			Detalle:  err.Error(),
		})
	}
}

// registrarFallo cuenta un intento fallido de transmisión: si el nuevo total de intentos
// alcanza intentosMaximos, abandona la fila con descarte duro (nunca una cola de reenvío);
// en caso contrario, persiste el nuevo contador para que un futuro Drenar vuelva a intentarlo.
// Ambas rutas son error-driven: no hay espera, jitter ni calendario de reintento, solo un
// tope de fallos.
func (c *ColaDeSalida) registrarFallo(ctx context.Context, p pendienteSalida) {
	nuevoIntentos := p.intentos + 1

	if nuevoIntentos >= c.intentosMaximos {
		query := `DELETE FROM cola_salida WHERE id_mensaje = ? AND enviado_en_ms IS NULL`
		res, err := c.db.ExecContext(ctx, query, p.idMensaje)
		if err != nil {
			if c.registro != nil {
				c.registro.Error("outbox.error_agotar", registro.Campos{IdEvento: p.idMensaje, Detalle: err.Error()})
			}
			return
		}
		if filas, _ := res.RowsAffected(); filas > 0 {
			c.liberarPresencia(p.idMensaje)
			if c.registro != nil {
				c.registro.Aviso(EventoSalidaAgotada, registro.Campos{IdEvento: p.idMensaje})
			}
			if c.sumideroAcuse != nil {
				c.sumideroAcuse(ipc.AcuseEnvio{
					IdMensaje:       p.idMensaje,
					Estado:          ipc.EstadoEnvioFallido,
					IdCorrelacion:   "",
					Motivo:          "intentos de transmisión agotados",
					MarcaTemporalMs: time.Now().UnixMilli(),
				})
			}
		}
		return
	}

	query := `UPDATE cola_salida SET intentos = ? WHERE id_mensaje = ? AND enviado_en_ms IS NULL`
	res, err := c.db.ExecContext(ctx, query, nuevoIntentos, p.idMensaje)
	if err != nil {
		if c.registro != nil {
			c.registro.Error("outbox.error_reintentar", registro.Campos{IdEvento: p.idMensaje, Detalle: err.Error()})
		}
		return
	}
	if filas, _ := res.RowsAffected(); filas > 0 && c.registro != nil {
		c.registro.Aviso(EventoSalidaReintentada, registro.Campos{IdEvento: p.idMensaje})
	}
}

// MarcarEnviado marca una fila como enviada de forma idempotente: guarda en enviado_en_ms IS
// NULL, así que una segunda llamada con el mismo id_mensaje (un reintento repetido o un acuse
// reenviado) afecta cero filas y nunca produce un segundo envío observable.
func (c *ColaDeSalida) MarcarEnviado(ctx context.Context, idMensaje, idCorrelacion string, ahoraMs int64) error {
	query := `UPDATE cola_salida SET enviado_en_ms = ?, id_correlacion = ? WHERE id_mensaje = ? AND enviado_en_ms IS NULL`
	res, err := c.db.ExecContext(ctx, query, ahoraMs, idCorrelacion, idMensaje)
	if err != nil {
		return fmt.Errorf("cola de salida: error al marcar enviado: %w", err)
	}
	filas, err := res.RowsAffected()
	if err != nil {
		return fmt.Errorf("cola de salida: error al obtener filas afectadas al marcar enviado: %w", err)
	}
	if filas > 0 {
		c.liberarPresencia(idMensaje)
		if err := RegistrarEnvioDelDia(ctx, c.db, c.diaEnZonaDeLaVentana(ahoraMs)); err != nil && c.registro != nil {
			c.registro.Error("outbox.error_registrar_envio_del_dia", registro.Campos{IdEvento: idMensaje, Detalle: err.Error()})
		}
		if c.registro != nil {
			c.registro.Info(EventoSalidaEnviada, registro.Campos{IdEvento: idMensaje})
		}
		if c.sumideroAcuse != nil {
			c.sumideroAcuse(ipc.AcuseEnvio{
				IdMensaje:       idMensaje,
				Estado:          ipc.EstadoEnvioEnviado,
				IdCorrelacion:   idCorrelacion,
				Motivo:          "",
				MarcaTemporalMs: ahoraMs,
			})
		}
	}
	return nil
}
