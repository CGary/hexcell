//! Identificadores opacos del dominio.
//!
//! El transporte expone identificadores propios —Meta usa `wa_id`, whatsmeow usa JID— y es el
//! **adaptador**, nunca el núcleo, quien los traduce a los identificadores de este módulo
//! (`docs/PRD.md`, FR-12, elemento 5; `docs/adr/adr-0010-puerto-de-canal.md`, punto 5).
//!
//! Por eso los tipos de aquí no tienen ni derivación ni inversión: el núcleo recibe el valor ya
//! traducido y lo trata como **opaco**. No lo deriva de ningún dato de transporte, no lo
//! interpreta y no lo invierte. Un constructor que aceptase un número de teléfono, o un método
//! que devolviese el identificador de transporte original, duplicaría en el núcleo una
//! responsabilidad que ya tiene el adaptador; y dos piezas que traducen lo mismo acaban
//! divergiendo sin que nadie lo note hasta que hay datos escritos por las dos.
//!
//! La prueba léxica de que ninguna firma nombra un identificador de transporte es **necesaria
//! pero no suficiente**: el mismo error de diseño puede repetirse bajo otro nombre. La parte
//! semántica la cubre `tests/guardian_identidad_conversacion.rs`.
//!
//! Los tres tipos son deliberadamente iguales en forma y distintos en tipo: son identificadores
//! de cosas distintas y confundirlos en una firma debe ser un error de compilación, no un error
//! de ejecución que aparezca en producción con datos de un cliente de pago.

/// Identificador interno de conversación, opaco para el núcleo.
///
/// Es el hilo al que pertenece un mensaje. Su valor lo produce el mapeo que vive dentro del
/// adaptador y que persiste en el almacén propio del adaptador, separado de las credenciales de
/// sesión del transporte para sobrevivir a un re-emparejamiento (`adr-0010`, puntos 5 y 6).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdConversacion(String);

impl IdConversacion {
    /// Construye el identificador a partir de un valor **ya traducido** por el adaptador.
    ///
    /// El núcleo no fabrica estos valores: los recibe. El constructor existe para que el
    /// adaptador —y las pruebas— puedan entregarlos, no para derivarlos de dato alguno.
    pub fn nuevo(valor: impl Into<String>) -> Self {
        Self(valor.into())
    }

    /// Vista prestada del valor opaco, para compararlo o persistirlo.
    ///
    /// Devuelve el identificador **interno**, que es el único que el núcleo conoce; no
    /// reconstruye ningún dato del transporte, porque el núcleo nunca lo tuvo.
    pub fn como_str(&self) -> &str {
        &self.0
    }
}

/// Identificador interno del remitente, opaco para el núcleo.
///
/// Se declara aparte de [`IdConversacion`] porque son cosas distintas —una conversación de grupo
/// tiene varios remitentes— y porque la alternativa cómoda, arrastrar el número de teléfono del
/// contacto hasta el dominio, es exactamente la filtración que `adr-0010` prohíbe.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdRemitente(String);

impl IdRemitente {
    /// Construye el identificador a partir de un valor **ya traducido** por el adaptador.
    pub fn nuevo(valor: impl Into<String>) -> Self {
        Self(valor.into())
    }

    /// Vista prestada del valor opaco.
    pub fn como_str(&self) -> &str {
        &self.0
    }
}

/// Identificador de deduplicación de un evento entrante, opaco para el núcleo.
///
/// El núcleo solo lo compara consigo mismo para descartar reentregas; no lo interpreta. En la
/// Cloud API el candidato natural es el campo `id` del objeto `messages`, y en whatsmeow el
/// identificador de mensaje del protocolo, pero cuál sea es asunto del adaptador.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdDeduplicacion(String);

impl IdDeduplicacion {
    /// Construye el identificador a partir de un valor **ya normalizado** por el adaptador.
    pub fn nuevo(valor: impl Into<String>) -> Self {
        Self(valor.into())
    }

    /// Vista prestada del valor opaco.
    pub fn como_str(&self) -> &str {
        &self.0
    }
}
