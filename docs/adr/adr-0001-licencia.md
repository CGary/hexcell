# ADR-0001 — Licencia del proyecto

* **Estado:** Vigente desde el 2026-07-29.
* **Supersede a:** nada. Primera decisión de licenciamiento del repositorio.
* **Etapa:** A-1.
* **Requisitos tocados:** ninguno directo del PRD; decisión de gobernanza del repositorio, no de
  producto.

---

## Contexto

El repositorio se inicializa sin licencia. Sin un archivo `LICENSE`, el código fuente que aterrice en
las próximas etapas queda bajo "todos los derechos reservados" por defecto, lo que impide cualquier
colaboración externa y no deja constancia escrita de qué puede hacer un tercero con el código. Hace
falta fijar la licencia **antes** de que exista el primer commit de código (Cargo workspace, crates,
sidecar Go), porque cambiar de licencia después de aceptar contribuciones de terceros exige su
consentimiento retroactivo o una reescritura, y ninguna de las dos es barata.

El objetivo comercial declarado del proyecto es vender el orquestador como servicio gestionado a
microempresas sobre hardware propio del titular del proyecto (ver `docs/PRD.md` y
`docs/STATUS.md`). No hay plan de distribuir binarios ni de que un tercero despliegue HexCell por su
cuenta como producto competidor sin devolver mejoras a la comunidad.

## Decisión

Se adopta **AGPL-3.0** (GNU Affero General Public License, versión 3) como licencia del repositorio,
con el titular del copyright conservando la posibilidad de **licenciamiento dual**: el titular puede
ofrecer el mismo código bajo una licencia comercial distinta a quien lo solicite, precisamente porque
es quien ostenta el copyright y no cede esa facultad al publicar bajo AGPL-3.0.

### Alternativas contrastadas

**A. Apache-2.0 (permisiva).** Permite a cualquiera —incluido un competidor directo— tomar el
código, modificarlo y ofrecerlo como servicio sin devolver una sola línea, porque Apache-2.0 no
impone condición alguna sobre el uso en red (no tiene cláusula equivalente a la de AGPL sobre
"Remote Network Interaction"). Para un producto que se vende como servicio gestionado (SaaS/on-prem
por célula), esto regala exactamente la ventaja competitiva que el proyecto necesita conservar
mientras es una operación de una sola persona. Se descarta: es la licencia correcta para una
biblioteca que quiere adopción masiva sin condiciones, no para un producto que se monetiza como
servicio.

**B. BUSL-1.1 (Business Source License).** Impide la competencia directa mediante una cláusula de uso
comercial restringido con una fecha de conversión futura a una licencia open source (normalmente
Apache-2.0 o MPL). Ofrece protección comercial más fuerte que AGPL-3.0 en el corto plazo, porque
prohíbe explícitamente ofrecer el software como servicio competidor durante el período restringido,
sin depender de que un tercero decida "abrir" sus modificaciones. Se descarta por dos motivos: (1) no
es una licencia OSI-aprobada mientras dura el período restringido, lo que complica cualquier
colaboración con terceros que exijan open source real desde el día uno; y (2) su mecánica depende de
mantener y actualizar una fecha de conversión y un cambio de licencia programado, gobernanza adicional
que no aporta valor a un proyecto de un solo titular que ya puede dual-licenciar bajo AGPL-3.0 sin
esa complejidad.

**C. AGPL-3.0 (elegida).** Cierra el hueco que Apache-2.0 deja abierto: su cláusula de red (sección 13)
obliga a quien ofrece el software modificado como servicio a través de una red a publicar el código
fuente de esas modificaciones. Un competidor que tome HexCell, lo modifique y lo revenda como su
propio servicio queda obligado a devolver esas modificaciones a la comunidad, lo que reduce
drásticamente el incentivo de hacerlo sin colaborar. A la vez, al ser el único titular del copyright,
el proyecto conserva la facultad de **dual licensing**: puede negociar una licencia comercial
distinta con un cliente o socio que no quiera operar bajo los términos de AGPL-3.0, sin pedir permiso
a nadie más, porque no hay contribuciones de terceros que licenciar por separado todavía.

## Consecuencias

### Positivas

* Un competidor que ofrezca HexCell modificado como servicio de red queda obligado por la sección 13
  a publicar sus cambios; el copyleft de red es precisamente la protección que el modelo SaaS/on-prem
  por célula necesita y que Apache-2.0 no da.
* El titular conserva el dual licensing como palanca comercial: puede vender una licencia no-AGPL a
  quien lo requiera, sin negociar con terceros, porque todavía no hay coautores del código.
* La licencia es OSI-aprobada y ampliamente reconocida, lo que simplifica cualquier colaboración
  futura sin la gobernanza adicional de una fecha de conversión (a diferencia de BUSL-1.1).

### Negativas

* AGPL-3.0 es percibida por algunas empresas como una licencia "hostil" y puede desalentar
  colaboraciones corporativas que sí aceptarían Apache-2.0 o MIT.
* Si en el futuro se aceptan contribuciones externas de terceros, el dual licensing exige recabar su
  consentimiento (o un CLA) para poder re-licenciar esas partes; hoy el repositorio no tiene
  contribuciones externas, así que esta limitación no aplica todavía pero condiciona cualquier
  política de contribución futura.
* No impide por sí sola que un competidor use el producto internamente sin ofrecerlo como servicio de
  red: la cláusula de AGPL-3.0 se activa por interacción de red, no por uso interno.

## Referencias

* Texto oficial: `LICENSE` (AGPL-3.0, verbatim en inglés, descargado de
  <https://www.gnu.org/licenses/agpl-3.0.txt>).
* `docs/STATUS.md`: la licencia pasa a **Definido** (2026-07-29).
* `docs/adr/README.md`: fila de este ADR.
