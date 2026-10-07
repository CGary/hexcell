#!/usr/bin/env bash
# Guarda estatica de HEX-091-b (documentacion). Se ejecuta desde el worktree de la tarea.
#   guarda-hex-091-b.sh               comprueba el arbol de trabajo contra main
#   guarda-hex-091-b.sh --autoprueba  ademas, prueba cada guarda con mutaciones sobre copias
#
# Reglas (AC-16, AC-22, AC-25):
#  1. Solo-anadir: en README.md, docs/runbook-operacion.md, docs/STATUS.md, el plan de A-6 y la
#     bitacora, toda linea retirada respecto a la base debe reaparecer como prefijo de una linea
#     anadida en el mismo bloque (anadido al final de la linea). Borrar o reescribir falla.
#  2. Literales (AC-25): el nombre del evento y el origen que el sidecar emite
#     (sidecar/internal/servidor/manejo.go) aparecen en el runbook por igualdad exacta.
#  3. Estructura: seccion 9 del runbook antes de «Reejecucion de un comando», seccion 10 del
#     README, filas de tabla, nota del plan, anexo de STATUS.
set -u

RAIZ="$(git rev-parse --show-toplevel)" || exit 2
cd "$RAIZ" || exit 2
BASE="$(git merge-base main HEAD)" || { echo "FALLA[sin-base]: no hay merge-base con main"; exit 2; }

ARCHIVOS_SOLO_ANADIR=(
  README.md
  docs/runbook-operacion.md
  docs/STATUS.md
  docs/plan/fase-a-6-empaquetado-cli.md
  docs/bitacora-de-descartes.md
)
RUNBOOK=docs/runbook-operacion.md
MANEJO=sidecar/internal/servidor/manejo.go

# comprobar_solo_anadir <archivo-base> <archivo-actual> <etiqueta>
comprobar_solo_anadir() {
  python3 -I - "$1" "$2" "$3" <<'PY'
import difflib, sys
base, actual, etiqueta = sys.argv[1:4]
a = open(base, encoding="utf-8").read().split("\n")
b = open(actual, encoding="utf-8").read().split("\n")
ok = True
for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
    if tag == "delete":
        for linea in a[i1:i2]:
            print(f"FALLA[solo-anadir]: {etiqueta}: linea borrada: {linea[:80]!r}")
            ok = False
    elif tag == "replace":
        nuevas = b[j1:j2]
        for linea in a[i1:i2]:
            if not any(n.startswith(linea) for n in nuevas):
                print(f"FALLA[solo-anadir]: {etiqueta}: linea reescrita: {linea[:80]!r}")
                ok = False
sys.exit(0 if ok else 1)
PY
}

# extraer_literal <constante>: valor entre comillas de `const <constante> = "..."` en el sidecar.
extraer_literal() {
  sed -n "s/^const $1 = \"\\(.*\\)\"\$/\\1/p" "$MANEJO" | head -n1
}

# comprobar_literales <runbook>
comprobar_literales() {
  local runbook="$1" evento origen ok=0
  evento="$(extraer_literal EventoBajaDeContactoRevivida)"
  origen="$(extraer_literal OrigenDeLaBajaRevivida)"
  if [ -z "$evento" ] || [ -z "$origen" ]; then
    echo "FALLA[literal-del-sidecar]: no se pudo leer el evento o el origen de $MANEJO"
    return 1
  fi
  grep -qF -- "\`$evento\`" "$runbook" || { echo "FALLA[literal-evento]: el runbook no contiene exactamente «$evento»"; ok=1; }
  grep -qF -- "\`$origen\`" "$runbook" || { echo "FALLA[literal-origen]: el runbook no contiene exactamente «$origen»"; ok=1; }
  return $ok
}

comprobar_estructura() {
  local ok=0 n_seccion n_reejec n_tabla
  n_seccion="$(grep -n '^## 9\. `contacto restablecer`' "$RUNBOOK" | head -n1 | cut -d: -f1)"
  n_reejec="$(grep -n '^## Reejecución de un comando' "$RUNBOOK" | head -n1 | cut -d: -f1)"
  if [ -z "$n_seccion" ]; then
    echo "FALLA[runbook-seccion-9]: falta «## 9. \`contacto restablecer\`» en $RUNBOOK"; ok=1
  elif [ -z "$n_reejec" ] || [ "$n_seccion" -ge "$n_reejec" ]; then
    echo "FALLA[runbook-orden]: la seccion 9 debe ir ANTES de «## Reejecución de un comando»"; ok=1
  fi
  n_tabla="$(grep -c 'contacto restablecer' "$RUNBOOK")"
  if [ "$n_tabla" -lt 4 ]; then
    echo "FALLA[runbook-filas]: se esperan >=4 menciones de «contacto restablecer» (seccion, fila de situacion, fila de reejecucion, alcance); hay $n_tabla"; ok=1
  fi
  grep -q 'contacto_desconocido' "$RUNBOOK" || { echo "FALLA[runbook-contacto-desconocido]: el runbook no menciona contacto_desconocido"; ok=1; }
  grep -q 'HEX-091' "$RUNBOOK" || { echo "FALLA[runbook-referencias]: el runbook no cita HEX-091"; ok=1; }
  grep -q '^### 10\.' README.md || { echo "FALLA[readme-seccion-10]: falta «### 10.» en README.md"; ok=1; }
  grep -q '^### 9\. Reejecución (entregado el 2026-09-26 con HEX-087' README.md || { echo "FALLA[readme-seccion-9]: la cabecera de la seccion 9 cambio"; ok=1; }
  git diff "$BASE" -- docs/plan/fase-a-6-empaquetado-cli.md | grep '^+' | grep -v '^+++' | grep -q 'HEX-091' \
    || { echo "FALLA[plan-nota]: la nota de cierre del plan no cita HEX-091"; ok=1; }
  git diff "$BASE" -- docs/plan/fase-a-6-empaquetado-cli.md | grep '^+' | grep -v '^+++' | grep -q -- '--confirmar' \
    || { echo "FALLA[plan-nota-confirmar]: la nota de cierre no registra --confirmar"; ok=1; }
  git diff "$BASE" -- docs/STATUS.md | grep '^+' | grep -v '^+++' | grep -q 'contacto restablecer' \
    || { echo "FALLA[status-anexo]: el anexo de STATUS no nombra «contacto restablecer»"; ok=1; }
  return $ok
}

comprobar_arbol() {
  local estado=0 archivo tmp
  tmp="$(mktemp -d)"
  for archivo in "${ARCHIVOS_SOLO_ANADIR[@]}"; do
    git show "$BASE:$archivo" > "$tmp/base" 2>/dev/null || continue
    comprobar_solo_anadir "$tmp/base" "$archivo" "$archivo" || estado=1
  done
  rm -rf "$tmp"
  comprobar_literales "$RUNBOOK" || estado=1
  comprobar_estructura || estado=1
  return $estado
}

autoprueba() {
  local tmp rc=0 base_rb
  tmp="$(mktemp -d)"
  base_rb="$tmp/base_rb"
  git show "$BASE:$RUNBOOK" > "$base_rb"

  # Control positivo: el runbook actual debe pasar la regla de solo-anadir.
  comprobar_solo_anadir "$base_rb" "$RUNBOOK" control >/dev/null \
    || { echo "AUTOPRUEBA[control]: el runbook actual ya viola solo-anadir"; rc=1; }

  # m1: borrar un literal previo (la primera linea de tabla de la base).
  local linea_previa
  linea_previa="$(grep -m1 '^| ' "$base_rb")"
  grep -vxF -- "$linea_previa" "$RUNBOOK" > "$tmp/m1"
  if cmp -s "$tmp/m1" "$RUNBOOK"; then echo "AUTOPRUEBA[m1]: la mutacion no cambio la copia"; rc=1
  elif comprobar_solo_anadir "$base_rb" "$tmp/m1" m1 >/dev/null; then echo "AUTOPRUEBA[m1]: borrar un literal previo NO fue detectado"; rc=1
  else echo "AUTOPRUEBA[m1]: borrado detectado (rojo esperado)"; fi

  # m2: reescribir en el sitio una linea previa (cambia su inicio, no su final).
  local objetivo
  objetivo="$(grep -m1 '^## Referencias' "$base_rb")"
  sed "0,/^## Referencias/s//## REESCRITA Referencias/" "$RUNBOOK" > "$tmp/m2"
  if [ -z "$objetivo" ] || cmp -s "$tmp/m2" "$RUNBOOK"; then echo "AUTOPRUEBA[m2]: la mutacion no cambio la copia"; rc=1
  elif comprobar_solo_anadir "$base_rb" "$tmp/m2" m2 >/dev/null; then echo "AUTOPRUEBA[m2]: reescribir en sitio NO fue detectado"; rc=1
  else echo "AUTOPRUEBA[m2]: reescritura detectada (rojo esperado)"; fi

  # m3: alterar el literal del evento en el runbook (debe romper la igualdad exacta).
  local evento
  evento="$(extraer_literal EventoBajaDeContactoRevivida)"
  sed "s/$evento/${evento}_x/g" "$RUNBOOK" > "$tmp/m3"
  if cmp -s "$tmp/m3" "$RUNBOOK"; then echo "AUTOPRUEBA[m3]: la mutacion no cambio la copia (¿el runbook cita el evento?)"; rc=1
  elif comprobar_literales "$tmp/m3" >/dev/null; then echo "AUTOPRUEBA[m3]: alterar el evento NO fue detectado"; rc=1
  else echo "AUTOPRUEBA[m3]: evento alterado detectado (rojo esperado)"; fi

  # m4: alterar el origen.
  local origen
  origen="$(extraer_literal OrigenDeLaBajaRevivida)"
  sed "s/$origen/${origen} --extra/g" "$RUNBOOK" > "$tmp/m4"
  if cmp -s "$tmp/m4" "$RUNBOOK"; then echo "AUTOPRUEBA[m4]: la mutacion no cambio la copia (¿el runbook cita el origen?)"; rc=1
  elif comprobar_literales "$tmp/m4" >/dev/null; then echo "AUTOPRUEBA[m4]: alterar el origen NO fue detectado"; rc=1
  else echo "AUTOPRUEBA[m4]: origen alterado detectado (rojo esperado)"; fi

  rm -rf "$tmp"
  return $rc
}

if [ "${1:-}" = "--autoprueba" ]; then
  autoprueba
  exit $?
fi

comprobar_arbol
rc=$?
[ $rc -eq 0 ] && echo "guarda-hex-091-b: OK"
exit $rc
