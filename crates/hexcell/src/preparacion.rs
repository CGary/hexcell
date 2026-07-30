//! Combinador puro de preparación: qué tiene que estar sano para que la célula esté lista.
//!
//! `GET /health/ready` responde a una pregunta concreta: **¿puede esta célula atender un mensaje
//! ahora mismo?** La respuesta es la conjunción de tres términos: la vitalidad de `sessions.db`, la
//! de `knowledge_live.db` y el estado de la sesión del canal. Si falta cualquiera de los tres, la
//! célula está viva pero no puede trabajar, y decirlo es justo lo que evita que un supervisor le
//! siga mandando tráfico.
//!
//! # Por qué el estado de sesión llega desde fuera y no desde el puerto de canal
//!
//! Comprobado contra el árbol el 2026-07-30: `ChannelAdapter` declara `send` y `estado_ventana`, y
//! nada más; `CicloDeVidaSesion` declara `iniciar_emparejamiento` y `cerrar_sesion` y no lo
//! implementa ningún tipo del workspace. **No existe** ningún método que responda «¿está la sesión
//! del canal en pie?», y esta tarea no reabre `hexcell-core` para inventarlo. Por eso el término
//! entra como tercer argumento, provisto en la raíz de composición.
//!
//! Y **no** se sustituye por `estado_ventana`, que es la tentación fácil y sería un error de
//! diseño: esa llamada responde si **una conversación concreta** está dentro de su ventana de
//! servicio de 24 horas, así que cablearla a la preparación haría que una célula se declarase
//! enferma porque un cliente lleva un día sin escribir.
//!
//! La etapa A-3, que trae el sidecar, sustituye [`SesionDelCanal::siempre_activa`] por la señal
//! real de conexión del canal propio. Hasta entonces el término existe, se evalúa y es
//! falsificable por un test: un término que ningún test puede tumbar es decoración, no una
//! comprobación.

use hexcell_storage::Vitalidad;

/// Estado de la sesión del canal, tal y como lo conoce la raíz de composición.
///
/// Es un tipo propio de este crate y no del puerto a propósito (ver la nota del módulo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SesionDelCanal {
    activa: bool,
}

impl SesionDelCanal {
    /// Sesión declarada permanentemente activa.
    ///
    /// Es lo que la raíz de composición usa hoy: el adaptador simulado no tiene sesión que
    /// perder, y el canal propio todavía no está integrado. La etapa A-3 sustituye este
    /// constructor por la señal real que publique el sidecar, sin tocar el combinador.
    pub fn siempre_activa() -> Self {
        Self { activa: true }
    }

    /// Sesión caída.
    ///
    /// Ningún adaptador de esta etapa la produce, y existe precisamente para que el tercer
    /// término de la conjunción sea comprobable: sin este constructor, la parte de la preparación
    /// que depende de la sesión no podría falsificarse desde ningún test.
    pub fn caida() -> Self {
        Self { activa: false }
    }

    /// ¿Está la sesión del canal en pie?
    pub fn esta_activa(&self) -> bool {
        self.activa
    }
}

/// Resultado del combinador de preparación.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preparacion {
    /// Los tres términos están sanos.
    Lista,
    /// Al menos un término no lo está. Nombra **cuál**: una respuesta de no preparado que no dice
    /// qué falló obliga a diagnosticar a ciegas desde fuera del contenedor.
    NoLista {
        /// Componente concreto que impide la preparación.
        componente: &'static str,
        /// Motivo legible, en español.
        motivo: String,
    },
}

/// Nombre del término de sesión en las respuestas de no preparado.
pub const COMPONENTE_SESION_DEL_CANAL: &str = "sesion-del-canal";

/// Combina los tres términos. Función **pura**: no toca red, no toca disco y no lee ningún reloj.
///
/// El orden de evaluación es el de la gravedad para el trabajo de la célula: sin `sessions.db` no
/// se puede ni deduplicar ni recordar, sin `knowledge_live.db` no se puede responder con criterio,
/// y sin sesión de canal no hay por dónde contestar. Se informa del primero que falla porque el
/// consumidor de esta respuesta actúa sobre uno, no sobre una lista.
pub fn evaluar_preparacion(
    vitalidad_de_sesiones: Vitalidad,
    vitalidad_de_conocimiento: Vitalidad,
    sesion: &SesionDelCanal,
) -> Preparacion {
    if let Vitalidad::Caida { componente, motivo } = vitalidad_de_sesiones {
        return Preparacion::NoLista { componente, motivo };
    }

    if let Vitalidad::Caida { componente, motivo } = vitalidad_de_conocimiento {
        return Preparacion::NoLista { componente, motivo };
    }

    if !sesion.esta_activa() {
        return Preparacion::NoLista {
            componente: COMPONENTE_SESION_DEL_CANAL,
            motivo: "la sesión del canal no está activa".to_string(),
        };
    }

    Preparacion::Lista
}
