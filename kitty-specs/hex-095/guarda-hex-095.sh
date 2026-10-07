#!/usr/bin/env bash
# Guarda estática de HEX-095 (hallazgos de laboratorio 7, 8 y 11 y nota de honestidad de Conectar).
#
# Juzga el árbol de trabajo (lo comprometido más lo que aún no se comprometió) frente a la base
# común `git merge-base main HEAD`, nunca solo contra HEAD (con el trabajo comprometido ese diff
# está vacío y la guarda nacería muerta) ni contra la punta de `main` (el trabajo que otra sesión
# fusione allí aparecería como borrado). Cada fallo sale con un código FALLA[...] que nombra la causa.
#
# Reglas (AC-2..AC-5; AC-6 es la autoprueba):
#   docs/STATUS.md        las cuatro entradas (Conectar, Hallazgo 7, 8 y 11) se localizan por su
#                         texto inicial, no por número de línea; cada una solo CRECE: el texto de la
#                         base queda como prefijo exacto y el sufijo es
#                         « *(Resuelto 2026-10-DD, `HEX-095`: …)*» (formato de la entrada del
#                         Hallazgo 10); ninguna otra línea cambia ni se mueve.
#   sidecar/internal/canal/canal.go
#                         todas las líneas de la base siguen presentes y en orden; solo se añaden
#                         líneas de comentario `//`, dentro del bloque de comentario de Conectar; el
#                         texto nuevo nombra la prueba de contexto cancelado y NO dice «sigue pendiente».
#   scripts/laboratorio/entorno.ejemplo.sh
#                         la línea del valor por omisión de HEXCELL_LAB_DIR es byte a byte la misma;
#                         solo se añaden líneas `#`, justo tras la línea 4 de la base; el bloque
#                         lleva el AVISO DE VOLATILIDAD.
#   docs/runbook-canal-fase-a.md
#                         la viñeta «El operador ejecuta `hexcell emparejar`» queda intacta; solo se
#                         añade la viñeta de disciplina operacional justo después.
#   docs/plan/fase-a-6-empaquetado-cli.md
#                         solo anexo al FINAL del archivo, con fecha absoluta y nombrando HEX-095.
#
# Uso:  bash guarda-hex-095.sh                  (desde la raíz del worktree)
#       bash guarda-hex-095.sh --repo <ruta>
#       bash guarda-hex-095.sh --autoprueba      (casos sintéticos: cada regla vista en verde y en rojo)
set -euo pipefail

exec python3 - "$@" <<'PY'
import difflib
import os
import re
import subprocess
import sys

BASE = "main"

ENTRADAS_STATUS = (
    "* **Restauración de nota de honestidad sobre contextos cancelados en Conectar**",
    "* **(Hallazgo 7) `hexcell emparejar` desplaza la conexión IPC",
    "* **(Hallazgo 8) `HEXCELL_LAB_DIR=/tmp` es volátil",
    "* **(Hallazgo 11) El modo `respaldar` registra `id_celula=sin-configurar`",
)
SUFIJO_STATUS = re.compile(r"^ \*\(Resuelto 2026-10-\d{2}, `HEX-095`: .+\)\*\.?$")

PRUEBA_CANCELADO = "TestIniciarEmparejamientoQrSobreAlmacenVacioDevuelveErrorAlNoPoderConectar"
FUNC_CONECTAR = "func (s *Sesion) Conectar("

ANCLA_ENTORNO = "# El empaquetado operable final (Docker + hexcell-admin) corresponde a la etapa A-6."
LINEA_LAB_DIR = 'export HEXCELL_LAB_DIR="${HEXCELL_LAB_DIR:-/tmp/hexcell-laboratorio}"'

LINEA_RUNBOOK = "   * El operador ejecuta `hexcell emparejar --metodo codigo_de_vinculacion`"
INICIO_VINETA = "   * **Disciplina operacional de `hexcell emparejar`:**"

FECHA_ABSOLUTA = re.compile(r"\b2026-10-\d{2}\b|\b\d{1,2} de octubre de 2026\b")


class Falla(Exception):
    pass


def opcodes(v, n):
    return [o for o in difflib.SequenceMatcher(None, v, n, autojunk=False).get_opcodes() if o[0] != "equal"]


def solo_inserciones(nombre, v, n):
    """Devuelve las inserciones (i1, j1, j2) y falla si hay cualquier borrado o reemplazo."""
    ins = []
    for tag, i1, i2, j1, j2 in opcodes(v, n):
        if tag != "insert":
            raise Falla(f"FALLA[{nombre}-no-anexa]: {tag} en la línea {i1 + 1} borra o reescribe texto")
        ins.append((i1, j1, j2))
    return ins


def revisar_status(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    if len(v) != len(n):
        raise Falla("FALLA[status-lineas]: docs/STATUS.md cambió su número de líneas")
    esperadas = {}
    for inicio in ENTRADAS_STATUS:
        idx = [i for i, l in enumerate(v) if l.startswith(inicio)]
        if len(idx) != 1:
            raise Falla(f"FALLA[status-sin-entrada]: la base no tiene exactamente una entrada «{inicio[:50]}»")
        esperadas[idx[0]] = inicio
    for i in range(len(v)):
        if i in esperadas:
            if not n[i].startswith(v[i]):
                raise Falla(f"FALLA[status-no-anexa]: la entrada «{esperadas[i][:50]}» perdió o reescribió texto")
            sufijo = n[i][len(v[i]):]
            if not sufijo:
                raise Falla(f"FALLA[status-sin-anexo]: la entrada «{esperadas[i][:50]}» no tiene anexo")
            if "HEX-095" not in sufijo:
                raise Falla(f"FALLA[status-sin-hex]: el anexo de «{esperadas[i][:50]}» no nombra HEX-095")
            if not SUFIJO_STATUS.match(sufijo):
                raise Falla(f"FALLA[status-formato]: el anexo de «{esperadas[i][:50]}» no tiene la forma  *(Resuelto 2026-10-DD, `HEX-095`: …)*")
        elif n[i] != v[i]:
            raise Falla(f"FALLA[status-otra-linea]: cambió la línea {i + 1}, que no es una de las cuatro entradas")


def revisar_canal(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    ins = solo_inserciones("canal", v, n)
    if not ins:
        raise Falla("FALLA[canal-sin-nota]: falta la nota nueva en el comentario de Conectar")
    f = next((i for i, l in enumerate(n) if l.startswith(FUNC_CONECTAR)), None)
    if f is None:
        raise Falla("FALLA[canal-sin-funcion]: desapareció func (s *Sesion) Conectar")
    ini = f
    while ini > 0 and n[ini - 1].lstrip().startswith("//"):
        ini -= 1
    anadido = []
    for _, j1, j2 in ins:
        for j in range(j1, j2):
            if not n[j].lstrip().startswith("//"):
                raise Falla(f"FALLA[canal-no-comentario]: la línea nueva {j + 1} no es un comentario")
            if not (ini <= j < f):
                raise Falla(f"FALLA[canal-fuera-de-conectar]: la línea nueva {j + 1} no está en el comentario de Conectar")
            anadido.append(n[j])
    texto = "\n".join(anadido)
    if PRUEBA_CANCELADO not in texto:
        raise Falla("FALLA[canal-sin-prueba]: la nota no nombra la prueba de contexto cancelado")
    if re.search(r"sigue pendiente", texto, re.I):
        raise Falla("FALLA[canal-sobreclama]: la nota dice «sigue pendiente», que STATUS.md (HEX-028) desmiente")


def revisar_entorno(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    if v.count(LINEA_LAB_DIR) != 1:
        raise Falla("FALLA[entorno-base]: la base no tiene exactamente una línea del valor por omisión de HEXCELL_LAB_DIR")
    if n.count(LINEA_LAB_DIR) != 1:
        raise Falla("FALLA[entorno-valor-omision]: la línea del valor por omisión de HEXCELL_LAB_DIR cambió o se duplicó")
    if ANCLA_ENTORNO not in v:
        raise Falla("FALLA[entorno-sin-ancla]: la base no tiene la línea 4 esperada")
    ancla = v.index(ANCLA_ENTORNO) + 1
    ins = solo_inserciones("entorno", v, n)
    if not ins:
        raise Falla("FALLA[entorno-sin-aviso]: falta el bloque de aviso de volatilidad")
    anadido = []
    for i1, j1, j2 in ins:
        if i1 != ancla:
            raise Falla("FALLA[entorno-posicion]: el bloque nuevo no está justo después de la línea 4")
        for j in range(j1, j2):
            if not n[j].startswith("#"):
                raise Falla(f"FALLA[entorno-no-comentario]: la línea nueva {j + 1} no es un comentario")
            anadido.append(n[j])
    texto = "\n".join(anadido)
    if "AVISO DE VOLATILIDAD" not in texto or "/tmp/hexcell-laboratorio" not in texto:
        raise Falla("FALLA[entorno-sin-aviso]: el bloque no lleva AVISO DE VOLATILIDAD ni nombra /tmp/hexcell-laboratorio")


def revisar_runbook(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    idx = [i for i, l in enumerate(v) if l.startswith(LINEA_RUNBOOK)]
    if len(idx) != 1:
        raise Falla("FALLA[runbook-base]: la base no tiene exactamente una viñeta «El operador ejecuta `hexcell emparejar`»")
    ancla = idx[0] + 1
    ins = solo_inserciones("runbook", v, n)
    if not ins:
        raise Falla("FALLA[runbook-sin-vineta]: falta la viñeta de disciplina operacional")
    anadido = []
    for i1, j1, j2 in ins:
        if i1 != ancla:
            raise Falla("FALLA[runbook-posicion]: la viñeta nueva no está justo después de la viñeta del operador")
        anadido.extend(n[j1:j2])
    if not anadido[0].startswith(INICIO_VINETA):
        raise Falla("FALLA[runbook-forma]: la viñeta nueva no empieza por «* **Disciplina operacional de `hexcell emparejar`:**» con la misma sangría")
    texto = "\n".join(anadido)
    for literal in ("DETENIDO", "hexcell-admin cell rebind"):
        if literal not in texto:
            raise Falla(f"FALLA[runbook-sin-literal]: la viñeta no contiene «{literal}»")


def revisar_plan(viejo, nuevo):
    base = viejo.rstrip("\n")
    if not nuevo.startswith(base):
        raise Falla("FALLA[plan-no-anexa]: el plan A-6 perdió o reescribió texto existente")
    anexo = nuevo[len(base):]
    if not anexo.strip():
        raise Falla("FALLA[plan-sin-nota]: falta la nota de cierre al final del plan A-6")
    if not anexo.startswith("\n"):
        raise Falla("FALLA[plan-no-anexa]: la nota se pegó al final de la última línea en vez de ir en líneas nuevas")
    if "HEX-095" not in anexo:
        raise Falla("FALLA[plan-sin-hex]: la nota de cierre no nombra HEX-095")
    if not FECHA_ABSOLUTA.search(anexo):
        raise Falla("FALLA[plan-sin-fecha]: la nota de cierre no lleva fecha absoluta de octubre de 2026")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True, text=True).stdout


def base_comun(repo):
    return git(repo, "merge-base", BASE, "HEAD").strip()


def en_base(repo, ruta):
    return git(repo, "show", f"{base_comun(repo)}:{ruta}")


def en_arbol(repo, ruta):
    with open(os.path.join(repo, ruta), encoding="utf-8") as f:
        return f.read()


REGLAS = (
    (revisar_status, "docs/STATUS.md"),
    (revisar_canal, "sidecar/internal/canal/canal.go"),
    (revisar_entorno, "scripts/laboratorio/entorno.ejemplo.sh"),
    (revisar_runbook, "docs/runbook-canal-fase-a.md"),
    (revisar_plan, "docs/plan/fase-a-6-empaquetado-cli.md"),
)


def modo_real(repo):
    fallas = []
    for regla, ruta in REGLAS:
        try:
            regla(en_base(repo, ruta), en_arbol(repo, ruta))
        except Falla as e:
            fallas.append(str(e))
    for f in fallas:
        print(f)
    if fallas:
        sys.exit(1)
    print("guarda-hex-095: OK")


def mutar(texto, viejo, nuevo):
    """Aplica la mutación y exige que la copia haya cambiado (una mutación que no se aplica prueba nada)."""
    if viejo not in texto:
        raise SystemExit(f"autoprueba rota: el literal «{viejo[:40]}» no está en la muestra")
    r = texto.replace(viejo, nuevo, 1)
    if r == texto:
        raise SystemExit(f"autoprueba rota: la mutación «{viejo[:40]}» no cambió la copia")
    return r


def autoprueba():
    casos = []

    def caso(nombre, f, args, codigo):
        try:
            f(*args)
            obtenido = "OK"
        except Falla as e:
            obtenido = re.match(r"FALLA\[([^\]]+)\]", str(e)).group(1)
        casos.append((nombre, codigo, obtenido))

    # STATUS: cuatro entradas más una ajena
    e1, e2, e3, e4 = (x + " texto previo." for x in ENTRADAS_STATUS)
    st = "\n".join(["## Pendiente", e1, e2, e3, "* **Otra** (2026-08-20).", e4, "* **Final**", ""])
    anexo = " *(Resuelto 2026-10-08, `HEX-095`: cerrado en esta tarea)*"
    st_ok = "\n".join(["## Pendiente", e1 + anexo, e2 + anexo, e3 + anexo, "* **Otra** (2026-08-20).", e4 + anexo + ".", "* **Final**", ""])
    caso("status anexo en las cuatro", revisar_status, (st, st_ok), "OK")
    caso("status sin cambio", revisar_status, (st, st), "status-sin-anexo")
    caso("status MUTACION borrar literal previo", revisar_status, (st, mutar(st_ok, "texto previo.", "")), "status-no-anexa")
    caso("status MUTACION reescribir en sitio", revisar_status, (st, mutar(st_ok, e2, e2.replace("previo", "nuevo"))), "status-no-anexa")
    caso("status reescribe y anexa", revisar_status, (st, mutar(st_ok, "(Hallazgo 8) `HEXCELL_LAB_DIR=/tmp` es volátil texto previo.", "(Hallazgo 8) `HEXCELL_LAB_DIR=/tmp` es volátil texto REESCRITO.")), "status-no-anexa")
    caso("status falta una entrada", revisar_status, (st, st_ok.replace(e3 + anexo, e3)), "status-sin-anexo")
    caso("status sin hex", revisar_status, (st, mutar(st_ok, "`HEX-095`", "`HEX-094`")), "status-sin-hex")
    caso("status formato", revisar_status, (st, mutar(st_ok, " *(Resuelto 2026-10-08,", " (Resuelto 2026-10-08, HEX-095")), "status-formato")
    caso("status otra línea", revisar_status, (st, mutar(st_ok, "* **Otra** (2026-08-20).", "* **Otra** (2026-08-20). HEX-095")), "status-otra-linea")
    caso("status línea movida", revisar_status, (st, st_ok.replace("* **Final**\n", "* **Final**\n* **Extra**\n")), "status-lineas")
    caso("status entrada movida", revisar_status, (st, "\n".join(["## Pendiente", e2 + anexo, e1 + anexo, e3 + anexo, "* **Otra** (2026-08-20).", e4 + anexo, "* **Final**", ""])), "status-no-anexa")

    # canal.go
    go = ("package canal\n\n// Cerrar x.\n\n// Conectar abre el websocket saliente hacia WhatsApp.\n//\n// Los tests de este paquete ejercitan únicamente el cableado.\n"
          "func (s *Sesion) Conectar(ctx context.Context) error {\n\treturn s.cliente.ConnectContext(ctx)\n}\n")
    nota = ("//\n// Los tests solo ejercitan la ruta de fallo con un contexto cancelado\n// (" + PRUEBA_CANCELADO + ", emparejamiento_test.go);\n"
            "// esas evidencias de laboratorio no son una prueba unitaria de esta función.\n")
    go_ok = mutar(go, "cableado.\nfunc", "cableado.\n" + nota + "func")
    caso("canal nota añadida", revisar_canal, (go, go_ok), "OK")
    caso("canal sin cambio", revisar_canal, (go, go), "canal-sin-nota")
    caso("canal MUTACION borrar línea previa", revisar_canal, (go, mutar(go_ok, "// Los tests de este paquete ejercitan únicamente el cableado.\n", "")), "canal-no-anexa")
    caso("canal MUTACION reescribir línea previa", revisar_canal, (go, mutar(go_ok, "únicamente el cableado.", "solo el cableado.")), "canal-no-anexa")
    caso("canal sobreclama", revisar_canal, (go, mutar(go_ok, "no son una prueba unitaria de esta función.", "sigue pendiente.")), "canal-sobreclama")
    caso("canal sin prueba nombrada", revisar_canal, (go, mutar(go_ok, PRUEBA_CANCELADO, "TestOtra")), "canal-sin-prueba")
    caso("canal código añadido", revisar_canal, (go, mutar(go_ok, "\treturn s.cliente", "\tlog.Println(1)\n\treturn s.cliente")), "canal-no-comentario")
    caso("canal comentario fuera", revisar_canal, (go, mutar(go_ok, "// Cerrar x.", "// Cerrar x.\n// añadido ajeno")), "canal-fuera-de-conectar")

    # entorno.ejemplo.sh
    sh = ("#!/usr/bin/env sh\n# Entorno compartido.\n# AVISO: Este arnés opera procesos directos.\n" + ANCLA_ENTORNO + "\n\n"
          "# Directorio raíz\n" + LINEA_LAB_DIR + "\n\nexport X=1\n")
    aviso = ("# AVISO DE VOLATILIDAD: el valor por omisión `/tmp/hexcell-laboratorio` es EFÍMERO.\n"
             "# Para persistir: export HEXCELL_LAB_DIR=\"$HOME/hexcell-laboratorio\"\n")
    sh_ok = mutar(sh, ANCLA_ENTORNO + "\n", ANCLA_ENTORNO + "\n" + aviso)
    caso("entorno aviso añadido", revisar_entorno, (sh, sh_ok), "OK")
    caso("entorno sin cambio", revisar_entorno, (sh, sh), "entorno-sin-aviso")
    caso("entorno MUTACION borrar línea previa", revisar_entorno, (sh, mutar(sh_ok, "# Directorio raíz\n", "")), "entorno-no-anexa")
    caso("entorno MUTACION cambiar el valor por omisión", revisar_entorno, (sh, mutar(sh_ok, "/tmp/hexcell-laboratorio}\"", "$HOME/hexcell-laboratorio}\"")), "entorno-valor-omision")
    caso("entorno aviso mal puesto", revisar_entorno, (sh, mutar(sh, "export X=1\n", "export X=1\n" + aviso)), "entorno-posicion")
    caso("entorno línea no comentario", revisar_entorno, (sh, mutar(sh_ok, "# Para persistir:", "echo x # Para persistir:")), "entorno-no-comentario")
    caso("entorno aviso sin marca", revisar_entorno, (sh, mutar(sh_ok, "AVISO DE VOLATILIDAD", "NOTA")), "entorno-sin-aviso")

    # runbook
    rb = ("3. **Superficie:**\n" + LINEA_RUNBOOK + " (o simplemente `hexcell emparejar`) en la terminal.\n"
          "   * *Superficie remota (Pendiente, Etapa A-6):* texto.\n\n4. **Otro**\n")
    vineta = (INICIO_VINETA + " abre su propio cliente IPC. Se ejecuta con el núcleo DETENIDO y el sidecar EN EJECUCIÓN. "
              "En una célula en servicio el camino es `hexcell-admin cell rebind`.\n")
    rb_ok = mutar(rb, "terminal.\n", "terminal.\n" + vineta)
    caso("runbook viñeta añadida", revisar_runbook, (rb, rb_ok), "OK")
    caso("runbook sin cambio", revisar_runbook, (rb, rb), "runbook-sin-vineta")
    caso("runbook MUTACION reescribir línea 34", revisar_runbook, (rb, mutar(rb_ok, "(o simplemente `hexcell emparejar`)", "")), "runbook-no-anexa")
    caso("runbook MUTACION borrar línea previa", revisar_runbook, (rb, mutar(rb_ok, "   * *Superficie remota (Pendiente, Etapa A-6):* texto.\n", "")), "runbook-no-anexa")
    caso("runbook viñeta mal puesta", revisar_runbook, (rb, mutar(rb, "4. **Otro**\n", "4. **Otro**\n" + vineta)), "runbook-posicion")
    caso("runbook forma", revisar_runbook, (rb, mutar(rb_ok, "**Disciplina operacional de `hexcell emparejar`:**", "Disciplina")), "runbook-forma")
    caso("runbook sin rebind", revisar_runbook, (rb, mutar(rb_ok, "hexcell-admin cell rebind", "otra cosa")), "runbook-sin-literal")

    # plan
    pl = "# Plan\n\n## Dependencias\n\n* **Decisiones:** texto.\n"
    nota_pl = "\n## Nota de cierre de HEX-095 (2026-10-08)\n\nSe cerraron los hallazgos 7, 8 y 11.\n"
    caso("plan nota añadida", revisar_plan, (pl, pl + nota_pl), "OK")
    caso("plan sin cambio", revisar_plan, (pl, pl), "plan-sin-nota")
    caso("plan MUTACION borrar línea previa", revisar_plan, (pl, mutar(pl + nota_pl, "* **Decisiones:** texto.\n", "")), "plan-no-anexa")
    caso("plan MUTACION reescribir en sitio", revisar_plan, (pl, mutar(pl + nota_pl, "## Dependencias", "## Dependencias y más")), "plan-no-anexa")
    caso("plan nota pegada a la última línea", revisar_plan, (pl, mutar(pl + "Cierre HEX-095 2026-10-08.\n", "texto.\nCierre", "texto. Cierre")), "plan-no-anexa")
    caso("plan sin hex", revisar_plan, (pl, mutar(pl + nota_pl, "HEX-095", "HEX-094")), "plan-sin-hex")
    caso("plan sin fecha", revisar_plan, (pl, mutar(pl + nota_pl, " (2026-10-08)", "")), "plan-sin-fecha")
    caso("plan fecha relativa", revisar_plan, (pl, mutar(pl + nota_pl, " (2026-10-08)", " (hoy)")), "plan-sin-fecha")

    malos = [c for c in casos if c[1] != c[2]]
    for nombre, esperado, obtenido in casos:
        print(f"{'ok ' if esperado == obtenido else 'MAL'} {nombre}: esperado={esperado} obtenido={obtenido}")
    if malos:
        sys.exit(1)
    print(f"autoprueba: {len(casos)} casos, todos con el veredicto esperado")


args = sys.argv[1:]
if args[:1] == ["--autoprueba"]:
    autoprueba()
else:
    repo = args[1] if args[:1] == ["--repo"] else os.getcwd()
    modo_real(repo)
PY
