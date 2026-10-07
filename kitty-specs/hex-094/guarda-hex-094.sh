#!/usr/bin/env bash
# Guarda estática de HEX-094 (reenvío del último estado de sesión al conectar el IPC y estado real
# del canal en /health/ready): cambios solo-anexo en la documentación.
#
# Juzga el árbol de trabajo (lo comprometido más lo que aún no se comprometió) frente a la base
# común `git merge-base main HEAD`, nunca solo contra HEAD (con el trabajo comprometido ese diff
# está vacío y la guarda nacería muerta) ni contra la punta de `main`: `main` avanza en paralelo
# (el 2026-10-07 ya se movió de 8ce2186 a aac95ee mientras la rama estaba abierta) y lo que otra
# sesión fusione allí aparecería aquí como borrado. Cada fallo sale con un código FALLA[...] que
# nombra la causa.
#
# Reglas:
#   docs/STATUS.md        toda línea de la base sigue presente y en orden; una línea de la base
#                         solo puede CRECER por sufijo (su texto queda como prefijo exacto); el
#                         resto son líneas nuevas. Las dos entradas Pendiente «Integración del
#                         estado real del canal en la preparación de la célula» y «Sincronización
#                         del estado de conexión del sidecar en la conexión del cliente IPC» deben
#                         haber crecido y su sufijo nombra HEX-094.
#   docs/plan/fase-a-6-empaquetado-cli.md
#                         la misma regla de solo-crecer; el texto añadido nombra HEX-094.
#   docs/protocolo-ipc-nucleo-sidecar.md
#                         toda línea de la base sigue presente y en orden, sin crecer, con UNA
#                         sola excepción: la línea de cabecera de versión, reemplazo literal exacto
#                         1.6 (2026-09-30) -> 1.7 (2026-10-07). La tabla conserva `| 1.6 | `7` |`
#                         y gana `| 1.7 | `7` |`; la sección 5 contiene el párrafo del reenvío.
#
# Uso:  bash guarda-hex-094.sh                  (desde la raíz del worktree)
#       bash guarda-hex-094.sh --repo <ruta>
#       bash guarda-hex-094.sh --autoprueba      (casos sintéticos: cada regla vista en verde y en
#                                                 rojo con el motivo esperado; además muta copias de
#                                                 los archivos reales de la rama si se corre en ella)
set -euo pipefail

exec python3 - "$@" <<'PY'
import difflib
import os
import re
import subprocess
import sys

BASE = "main"

ENTRADAS_STATUS = (
    "* **Integración del estado real del canal en la preparación de la célula**",
    "* **Sincronización del estado de conexión del sidecar en la conexión del cliente IPC**",
)

CABECERA_VIEJA = "* **Versión de este protocolo:** 1.6, fijada el 2026-09-30."
CABECERA_NUEVA = "* **Versión de este protocolo:** 1.7, fijada el 2026-10-07."
FILA_16 = "| 1.6 | `7` |"
FILA_17 = "| 1.7 | `7` |"
INICIO_SEC5 = "## 5. "
INICIO_SEC6 = "## 6. "
LITERAL_REENVIO = "reenvía al núcleo el último `estado_sesion` que emitió su"


class Falla(Exception):
    pass


def opcodes(v, n):
    return [o for o in difflib.SequenceMatcher(None, v, n, autojunk=False).get_opcodes() if o[0] != "equal"]


def solo_crece(nombre, v, n, crecer=True, permitido=None):
    """Exige que cada línea de v siga en n, en orden: igual, crecida por sufijo (si crecer) o
    reemplazada por el literal exacto que permite `permitido` ({línea vieja: línea nueva}).

    Devuelve (crecidas, nuevas): crecidas = {índice en v: sufijo}, nuevas = lista de líneas nuevas.
    Motivo del rechazo: «borra» si en el tramo cambiado quedan menos líneas nuevas que viejas por
    emparejar o ninguna nueva se le parece (ratio < 0.6: la línea desapareció); «reescribe» si una
    línea parecida ocupa su sitio con otro texto.
    """
    permitido = permitido or {}

    def empareja(viejo, nuevo_):
        return nuevo_ == viejo or (crecer and nuevo_.startswith(viejo)) or permitido.get(viejo) == nuevo_

    crecidas, nuevas = {}, []
    for tag, i1, i2, j1, j2 in opcodes(v, n):
        if tag == "insert":
            nuevas.extend(n[j1:j2])
            continue
        if tag == "delete":
            raise Falla(f"FALLA[{nombre}-borra]: se borró la línea {i1 + 1} de la base")
        j = j1
        for i in range(i1, i2):
            k = next((k for k in range(j, j2) if empareja(v[i], n[k])), None)
            if k is None:
                parecida = any(difflib.SequenceMatcher(None, v[i], n[k]).ratio() >= 0.6 for k in range(j, j2))
                if (j2 - j) < (i2 - i) or not parecida:
                    raise Falla(f"FALLA[{nombre}-borra]: se borró la línea {i + 1} de la base")
                raise Falla(f"FALLA[{nombre}-reescribe]: la línea {i + 1} de la base se reescribió (no queda igual ni como prefijo)")
            nuevas.extend(n[j:k])
            if n[k] != v[i] and permitido.get(v[i]) != n[k]:
                crecidas[i] = n[k][len(v[i]):]
            j = k + 1
        nuevas.extend(n[j:j2])
    return crecidas, nuevas


def revisar_status(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    crecidas, _ = solo_crece("status", v, n)
    for inicio in ENTRADAS_STATUS:
        idx = [i for i, l in enumerate(v) if l.startswith(inicio)]
        if len(idx) != 1:
            raise Falla(f"FALLA[status-sin-entrada]: la base no tiene exactamente una entrada «{inicio[:60]}»")
        sufijo = crecidas.get(idx[0])
        if not sufijo:
            raise Falla(f"FALLA[status-sin-anexo]: la entrada «{inicio[:60]}» no creció")
        if "HEX-094" not in sufijo:
            raise Falla(f"FALLA[status-sin-hex]: el anexo de «{inicio[:60]}» no nombra HEX-094")


def revisar_plan(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    crecidas, nuevas = solo_crece("plan", v, n)
    anadido = "\n".join(list(crecidas.values()) + nuevas)
    if not anadido.strip():
        raise Falla("FALLA[plan-sin-nota]: el plan A-6 no tiene texto nuevo")
    if "HEX-094" not in anadido:
        raise Falla("FALLA[plan-sin-hex]: el texto nuevo del plan A-6 no nombra HEX-094")


def revisar_protocolo(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    if v.count(CABECERA_VIEJA) != 1:
        raise Falla("FALLA[protocolo-base]: la base no tiene exactamente una cabecera de versión 1.6")
    solo_crece("protocolo", v, n, crecer=False, permitido={CABECERA_VIEJA: CABECERA_NUEVA})
    if n.count(CABECERA_NUEVA) != 1:
        raise Falla("FALLA[protocolo-cabecera]: la cabecera no pasó a «1.7, fijada el 2026-10-07»")
    if n.count(FILA_16) != 1:
        raise Falla("FALLA[protocolo-fila-16]: la tabla perdió o duplicó la fila 1.6 -> 7")
    if n.count(FILA_17) != 1:
        raise Falla("FALLA[protocolo-fila-17]: la tabla no tiene exactamente una fila 1.7 -> 7")
    if n.index(FILA_17) != n.index(FILA_16) + 1:
        raise Falla("FALLA[protocolo-fila-17]: la fila 1.7 no va justo después de la fila 1.6")
    ini = [i for i, l in enumerate(n) if l.startswith(INICIO_SEC5)]
    fin = [i for i, l in enumerate(n) if l.startswith(INICIO_SEC6)]
    if len(ini) != 1 or len(fin) != 1 or fin[0] <= ini[0]:
        raise Falla("FALLA[protocolo-sec5]: no se localiza la sección 5 entre «## 5.» y «## 6.»")
    sec5 = " ".join(l.strip() for l in n[ini[0]:fin[0]])
    if LITERAL_REENVIO not in sec5:
        raise Falla("FALLA[protocolo-sin-reenvio]: la sección 5 no contiene el párrafo del reenvío del último estado_sesion")


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
    (revisar_plan, "docs/plan/fase-a-6-empaquetado-cli.md"),
    (revisar_protocolo, "docs/protocolo-ipc-nucleo-sidecar.md"),
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
    print(f"guarda-hex-094: OK (base común {base_comun(repo)[:7]})")


def mutar(texto, viejo, nuevo):
    """Aplica la mutación y exige que la copia haya cambiado (una mutación que no se aplica prueba nada)."""
    if viejo not in texto:
        raise SystemExit(f"autoprueba rota: el literal «{viejo[:40]}» no está en la muestra")
    r = texto.replace(viejo, nuevo, 1)
    if r == texto:
        raise SystemExit(f"autoprueba rota: la mutación «{viejo[:40]}» no cambió la copia")
    return r


def autoprueba(repo):
    casos = []

    def caso(nombre, f, args, codigo):
        try:
            f(*args)
            obtenido = "OK"
        except Falla as e:
            obtenido = re.match(r"FALLA\[([^\]]+)\]", str(e)).group(1)
        casos.append((nombre, codigo, obtenido))

    # STATUS sintético: dos entradas objetivo y líneas ajenas.
    e1, e2 = (x + " texto previo." for x in ENTRADAS_STATUS)
    st = "\n".join(["## Pendiente", "* **Previa** (2026-08-18).", e1, "* **Otra** (2026-08-18).", e2, "* **Final**", ""])
    anexo = " — *(Resuelto 2026-10-07, HEX-094: cerrado)*"
    st_ok = "\n".join(["## Pendiente", "* **Previa** (2026-08-18).", e1 + anexo, "* **Otra** (2026-08-18).", e2 + anexo, "* **Final**", "* **Nueva** (2026-10-07, HEX-094).", ""])
    caso("status crecen las dos y se añade una", revisar_status, (st, st_ok), "OK")
    caso("status sin cambio", revisar_status, (st, st), "status-sin-anexo")
    caso("status MUTACION (i) borrar línea previa", revisar_status, (st, mutar(st_ok, "* **Otra** (2026-08-18).\n", "")), "status-borra")
    caso("status MUTACION reescribir línea previa", revisar_status, (st, mutar(st_ok, "* **Previa** (2026-08-18).", "* **Previa** (2026-08-19).")), "status-reescribe")
    caso("status MUTACION reescribir entrada y anexar", revisar_status, (st, mutar(st_ok, e1, e1.replace("previo", "nuevo"))), "status-reescribe")
    caso("status sin hex", revisar_status, (st, mutar(st_ok, "HEX-094: cerrado", "HEX-095: cerrado")), "status-sin-hex")
    caso("status entradas reordenadas", revisar_status, (st, "\n".join(["## Pendiente", "* **Previa** (2026-08-18).", e2 + anexo, "* **Otra** (2026-08-18).", e1 + anexo, "* **Final**", ""])), "status-borra")

    # Plan sintético.
    pl = "# Plan\n\n## Dependencias\n\n* **Decisiones:** texto.\n"
    nota = "\n**Cierre de HEX-094 (2026-10-07).** Reenvío del último estado.\n"
    caso("plan nota añadida", revisar_plan, (pl, pl + nota), "OK")
    caso("plan sin cambio", revisar_plan, (pl, pl), "plan-sin-nota")
    caso("plan MUTACION (iii) reescribir línea previa", revisar_plan, (pl, mutar(pl + nota, "## Dependencias", "## Dependencia externa")), "plan-reescribe")
    caso("plan MUTACION borrar línea previa", revisar_plan, (pl, mutar(pl + nota, "* **Decisiones:** texto.\n", "")), "plan-borra")
    caso("plan sin hex", revisar_plan, (pl, mutar(pl + nota, "HEX-094", "HEX-093")), "plan-sin-hex")

    # Protocolo sintético.
    pr = "\n".join(["# Protocolo", "", CABECERA_VIEJA, "", "| Doc | Cable |", FILA_16, "", "## 5. Reconexión",
                    "", "El invariante es el mismo.", "", "### Retroceso", "", "## 6. Tipos", ""])
    parrafo = ["### Reenvío", "", "de saludo se completa, el sidecar **reenvía al núcleo el último `estado_sesion` que emitió su",
               "supervisor**, si ya emitió alguno.", ""]
    pr_ok_l = pr.split("\n")
    pr_ok_l[2] = CABECERA_NUEVA
    pr_ok_l.insert(6, FILA_17)
    k = pr_ok_l.index("### Retroceso")
    pr_ok_l[k:k] = parrafo
    pr_ok = "\n".join(pr_ok_l)
    caso("protocolo correcto", revisar_protocolo, (pr, pr_ok), "OK")
    caso("protocolo sin cambio", revisar_protocolo, (pr, pr), "protocolo-cabecera")
    caso("protocolo MUTACION (ii) reescribir literal de la sección 5", revisar_protocolo, (pr, mutar(pr_ok, "El invariante es el mismo.", "El invariante cambió.")), "protocolo-reescribe")
    caso("protocolo MUTACION crecer línea de la sección 5", revisar_protocolo, (pr, mutar(pr_ok, "El invariante es el mismo.", "El invariante es el mismo. Y más.")), "protocolo-reescribe")
    caso("protocolo borrar línea previa", revisar_protocolo, (pr, mutar(pr_ok, "### Retroceso\n", "")), "protocolo-borra")
    caso("protocolo cabecera con otra fecha", revisar_protocolo, (pr, mutar(pr_ok, CABECERA_NUEVA, CABECERA_NUEVA.replace("10-07", "10-08"))), "protocolo-reescribe")
    caso("protocolo sin fila 1.7", revisar_protocolo, (pr, mutar(pr_ok, FILA_17 + "\n", "")), "protocolo-fila-17")
    caso("protocolo fila 1.6 cambiada a 1.7", revisar_protocolo, (pr, mutar(pr_ok, FILA_16 + "\n" + FILA_17, FILA_17)), "protocolo-reescribe")
    caso("protocolo sin párrafo de reenvío", revisar_protocolo, (pr, mutar(pr_ok, "reenvía al núcleo el último", "envía algo")), "protocolo-sin-reenvio")
    fuera_l = [l for l in pr_ok_l]
    del fuera_l[k:k + len(parrafo)]
    fuera_l[fuera_l.index("## 6. Tipos") + 1:fuera_l.index("## 6. Tipos") + 1] = parrafo
    caso("protocolo párrafo fuera de la sección 5", revisar_protocolo, (pr, "\n".join(fuera_l)), "protocolo-sin-reenvio")

    # Mutaciones sobre copias de los archivos REALES (si la rama los tiene).
    try:
        reales = {r: (en_base(repo, r), en_arbol(repo, r)) for _, r in REGLAS}
    except Exception as e:  # fuera de un repo con la rama: solo casos sintéticos
        reales = None
        print(f"aviso: sin archivos reales ({e}); solo casos sintéticos")
    if reales:
        sv, sn = reales["docs/STATUS.md"]
        caso("REAL status sin mutar", revisar_status, (sv, sn), "OK")
        linea = next(l for l in sv.split("\n") if l.startswith("* **Unificación del nombre de dispositivo vinculado"))
        caso("REAL status MUTACION (i) borrar línea previa", revisar_status, (sv, mutar(sn, linea + "\n", "")), "status-borra")
        pv, pn = reales["docs/plan/fase-a-6-empaquetado-cli.md"]
        caso("REAL plan sin mutar", revisar_plan, (pv, pn), "OK")
        caso("REAL plan MUTACION (iii) reescribir línea previa", revisar_plan,
             (pv, mutar(pn, "entregado con parser manual, sonda HTTP hermana", "entregado con parser automático, sonda HTTP hermana")), "plan-reescribe")
        rv, rn = reales["docs/protocolo-ipc-nucleo-sidecar.md"]
        caso("REAL protocolo sin mutar", revisar_protocolo, (rv, rn), "OK")
        caso("REAL protocolo MUTACION (ii) reescribir literal de la sección 5", revisar_protocolo,
             (rv, mutar(rn, "**el estado que importa está en disco, no en", "**el estado que importa está en memoria, no en")), "protocolo-reescribe")

    malos = [c for c in casos if c[1] != c[2]]
    for nombre, esperado, obtenido in casos:
        print(f"{'ok ' if esperado == obtenido else 'MAL'} {nombre}: esperado={esperado} obtenido={obtenido}")
    if malos:
        sys.exit(1)
    print(f"autoprueba: {len(casos)} casos, todos con el veredicto esperado")


args = sys.argv[1:]
repo = os.getcwd()
if "--repo" in args:
    repo = args[args.index("--repo") + 1]
if "--autoprueba" in args:
    autoprueba(repo)
else:
    modo_real(repo)
PY
