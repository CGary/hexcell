#!/usr/bin/env bash
# Guarda estática de HEX-093 (saneamiento de marcas de época sospechosa, hijo a: núcleo).
#
# Juzga el árbol de trabajo (lo comprometido más lo que aún no se comprometió) frente a la base
# común `git merge-base main HEAD`, nunca solo contra HEAD: con el trabajo comprometido ese diff
# está vacío y la guarda nacería muerta. La base común, y no la punta de `main`, evita que el
# trabajo que otra sesión fusione en `main` aparezca aquí como borrado; aparte, el número del ADR
# y del descarte nuevos se contrastan con la PUNTA de `main` para detectar la colisión con una
# hermana paralela (FALLA[...-colision]: rebasar y renumerar). Cada fallo sale con un código
# FALLA[...] que nombra la causa.
#
# Reglas:
#   docs/STATUS.md       solo se anexa texto al FINAL de la línea de la entrada «Superficie de
#                        operador para el saneamiento de marcas de época sospechosa»; el anexo
#                        nombra HEX-093; ninguna otra línea cambia (la entrada no se mueve).
#   docs/plan/fase-a-5-conocimiento-shadow-db.md
#                        solo anexo (líneas nuevas o texto al final de una línea) dentro del
#                        bloque de la tarea 8; nombra HEX-093.
#   docs/adr/README.md   solo líneas nuevas: exactamente una fila del ADR siguiente al más alto
#                        de `main`, cuyo archivo existe y no existía en `main`; el ADR nuevo cita
#                        adr-0027.
#   docs/bitacora-de-descartes.md
#                        o no cambia, o gana exactamente UN descarte con el número siguiente al
#                        más alto de `main`, con su fila de índice, y la cabecera «Última
#                        actualización» pasa a ese número (único reemplazo admitido).
#   Código               las firmas públicas existentes de retencion.rs y los nombres ya
#                        reexportados por lib.rs siguen presentes; reversion.rs solo cambia la
#                        visibilidad de fecha_absoluta_de_hoy a pub(crate); el recuento de
#                        remove_file/remove_dir y de sync_all/sync_data/fsync no cambia.
#   Escaneos             cada sitio tiene su propia función de listado que llama al predicado
#                        compartido: purgar_epocas_retiradas -> rutas_de_epoca_a_escanear_en_purga
#                        (retencion.rs) y numero_de_epoca_siguiente ->
#                        rutas_de_epoca_a_escanear_para_numerar (promocion.rs); ninguno de los dos
#                        cuerpos conserva el filtro viejo en línea (ends_with del sufijo activo).
#   Fuente única         purgar_epocas_retiradas llama numeros_de_epoca_marcados(ruta_datos) y no
#                        arma su propio conjunto con leer_marcas_de_epoca_sospechosa.
#
# Uso:  bash guarda-hex-093.sh                  (desde la raíz del worktree)
#       bash guarda-hex-093.sh --repo <ruta>
#       bash guarda-hex-093.sh --autoprueba      (casos sintéticos: cada regla vista en verde y en rojo)
set -euo pipefail

exec python3 - "$@" <<'PY'
import difflib
import os
import re
import subprocess
import sys

BASE = "main"
ENTRADA_STATUS = "* **Superficie de operador para el saneamiento de marcas de época sospechosa**"
TAREA_8 = "8. **Implementar retención y reversión de épocas**"
TAREA_9 = "9. **"
CABECERA_BITACORA = re.compile(r"Última actualización: \d{4}-\d{2}-\d{2} \(D-(\d+)\)\.")


class Falla(Exception):
    pass


def revisar_status(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    if len(v) != len(n):
        raise Falla("FALLA[status-lineas]: docs/STATUS.md cambió su número de líneas")
    cambios = [i for i in range(len(v)) if v[i] != n[i]]
    if not cambios:
        raise Falla("FALLA[status-sin-anexo]: falta el anexo HEX-093 en la entrada de la línea 469")
    for i in cambios:
        if not v[i].startswith(ENTRADA_STATUS):
            raise Falla(f"FALLA[status-otra-linea]: cambió la línea {i + 1}, que no es la entrada")
        if not n[i].startswith(v[i]):
            raise Falla("FALLA[status-no-anexa]: la entrada perdió o reescribió texto existente")
        if "HEX-093" not in n[i][len(v[i]):]:
            raise Falla("FALLA[status-sin-hex]: el anexo no nombra HEX-093")


def opcodes(v, n):
    return [o for o in difflib.SequenceMatcher(None, v, n, autojunk=False).get_opcodes() if o[0] != "equal"]


def revisar_plan(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    ini = next((i for i, l in enumerate(v) if l.startswith(TAREA_8)), None)
    fin = next((i for i, l in enumerate(v) if l.startswith(TAREA_9)), None)
    if ini is None or fin is None:
        raise Falla("FALLA[plan-sin-tarea-8]: no se encontró el bloque de la tarea 8 en main")
    ops = opcodes(v, n)
    if not ops:
        raise Falla("FALLA[plan-sin-anexo]: falta la frase HEX-093 en la tarea 8")
    anexado = []
    for tag, i1, i2, j1, j2 in ops:
        if not (ini < i1 <= fin and i2 <= fin):
            raise Falla(f"FALLA[plan-fuera-de-tarea-8]: cambio en la línea {i1 + 1}, fuera de la tarea 8")
        if tag == "insert":
            anexado.extend(n[j1:j2])
        elif tag == "replace" and i2 - i1 == 1 and n[j1].startswith(v[i1]):
            anexado.append(n[j1][len(v[i1]):])
            anexado.extend(n[j1 + 1:j2])
        else:
            raise Falla(f"FALLA[plan-no-anexa]: {tag} en la línea {i1 + 1} borra o reescribe texto")
    if "HEX-093" not in "\n".join(anexado):
        raise Falla("FALLA[plan-sin-hex]: el anexo de la tarea 8 no nombra HEX-093")


def numeros_adr(nombres):
    return sorted(int(m.group(1)) for x in nombres if (m := re.match(r"adr-(\d{4})-.*\.md$", x)))


def revisar_readme_adr(viejo, nuevo, adr_base, adr_nuevo, texto_adr_nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    anadidas = []
    for tag, i1, i2, j1, j2 in opcodes(v, n):
        if tag != "insert":
            raise Falla(f"FALLA[readme-no-anexa]: {tag} en la línea {i1 + 1} de docs/adr/README.md")
        anadidas.extend(n[j1:j2])
    anadidas = [l for l in anadidas if l.strip()]
    siguiente = max(numeros_adr(adr_base)) + 1
    esperados = [x for x in adr_nuevo if x not in adr_base]
    if len(esperados) != 1:
        raise Falla(f"FALLA[adr-archivos]: se esperaba exactamente un ADR nuevo, hay {len(esperados)}")
    if numeros_adr(esperados) != [siguiente]:
        raise Falla(f"FALLA[adr-numero]: el ADR nuevo {esperados[0]} no es adr-{siguiente:04d}")
    if len(anadidas) != 1 or not anadidas[0].startswith(f"| `{esperados[0]}` |"):
        raise Falla("FALLA[readme-fila]: se esperaba una única fila nueva para " + esperados[0])
    if "adr-0027" not in texto_adr_nuevo:
        raise Falla("FALLA[adr-sin-0027]: el ADR nuevo no cita adr-0027")


def revisar_bitacora(viejo, nuevo):
    if viejo == nuevo:
        return
    v, n = viejo.split("\n"), nuevo.split("\n")
    base = [int(x) for x in re.findall(r"^### D-(\d+)", viejo, re.M)]
    nuevos = sorted(set(int(x) for x in re.findall(r"^### D-(\d+)", nuevo, re.M)) - set(base))
    siguiente = max(base) + 1
    if nuevos != [siguiente]:
        raise Falla(f"FALLA[bitacora-numero]: se esperaba exactamente D-{siguiente}, hay {nuevos}")
    for tag, i1, i2, j1, j2 in opcodes(v, n):
        if tag == "insert":
            continue
        ok = (
            tag == "replace" and i2 - i1 == 1 and j2 - j1 == 1
            and (mv := CABECERA_BITACORA.search(v[i1])) and (mn := CABECERA_BITACORA.search(n[j1]))
            and CABECERA_BITACORA.sub("", v[i1]) == CABECERA_BITACORA.sub("", n[j1])
            and int(mn.group(1)) == siguiente
        )
        if not ok:
            raise Falla(f"FALLA[bitacora-no-anexa]: {tag} en la línea {i1 + 1} borra o reescribe texto")
    cab = CABECERA_BITACORA.search(nuevo)
    if not cab or int(cab.group(1)) != siguiente:
        raise Falla(f"FALLA[bitacora-cabecera]: la cabecera no pasa a D-{siguiente}")
    if not re.search(rf"^\| \[D-{siguiente}\]\(#d-{siguiente}\) \|", nuevo, re.M):
        raise Falla(f"FALLA[bitacora-indice]: falta la fila de índice de D-{siguiente}")


def sin_comentarios(texto):
    return re.sub(r"\s+", " ", "\n".join(l for l in texto.split("\n") if not l.strip().startswith("//")))


def revisar_firmas(viejo, nuevo):
    v, n = sin_comentarios(viejo), sin_comentarios(nuevo)
    items = re.findall(r"pub (?:fn|const) [^{;]*[{;]", v) + re.findall(r"pub (?:struct|enum) \w+ \{[^}]*\}", v)
    for item in items:
        if item not in n:
            raise Falla(f"FALLA[firma-publica]: cambió o desapareció «{item[:90]}»")


def nombres_reexportados(texto):
    nombres = set()
    for bloque in re.findall(r"pub use [^;]*;", texto, re.S):
        nombres.update(re.findall(r"\b[A-Za-z_][A-Za-z0-9_]*\b", bloque))
    return nombres


def revisar_reexportes(viejo, nuevo):
    faltan = sorted(nombres_reexportados(viejo) - nombres_reexportados(nuevo))
    if faltan:
        raise Falla(f"FALLA[reexporte]: lib.rs dejó de reexportar {faltan}")


def revisar_reversion(viejo, nuevo):
    permitido = viejo.replace("\nfn fecha_absoluta_de_hoy()", "\npub(crate) fn fecha_absoluta_de_hoy()", 1)
    if nuevo not in (viejo, permitido):
        raise Falla("FALLA[reversion]: reversion.rs cambió algo más que la visibilidad de fecha_absoluta_de_hoy")


BORRADO = re.compile(r"remove_file|remove_dir")
FSYNC = re.compile(r"sync_all|sync_data|fsync")


def revisar_recuentos(pares):
    for ruta, (viejo, nuevo) in pares.items():
        for nombre, patron in (("borrado", BORRADO), ("fsync", FSYNC)):
            a, b = len(patron.findall(viejo)), len(patron.findall(nuevo))
            if a != b:
                raise Falla(f"FALLA[{nombre}]: {ruta} pasó de {a} a {b} apariciones de {patron.pattern}")


def sin_comentarios_lineas(texto):
    return "\n".join(re.sub(r"//.*$", "", l) for l in texto.split("\n"))


def cuerpo_de(texto, nombre):
    """Cuerpo (entre llaves) de `fn nombre`, sin comentarios; None si la función no existe."""
    t = sin_comentarios_lineas(texto)
    m = re.search(rf"\bfn {nombre}\b", t)
    if not m:
        return None
    i = t.find("{", m.end())
    if i < 0:
        return None
    nivel = 0
    for j in range(i, len(t)):
        if t[j] == "{":
            nivel += 1
        elif t[j] == "}":
            nivel -= 1
            if nivel == 0:
                return t[i:j + 1]
    return None


FILTRO_VIEJO = re.compile(r"ends_with\(\s*(?:crate::retencion::)?SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA\s*\)")
SITIOS = (
    ("retencion.rs", "purgar_epocas_retiradas", "rutas_de_epoca_a_escanear_en_purga"),
    ("promocion.rs", "numero_de_epoca_siguiente", "rutas_de_epoca_a_escanear_para_numerar"),
)


def revisar_escaneos(retencion, promocion):
    textos = {"retencion.rs": retencion, "promocion.rs": promocion}
    for ruta, escaneo, listado in SITIOS:
        texto = textos[ruta]
        c_listado, c_escaneo = cuerpo_de(texto, listado), cuerpo_de(texto, escaneo)
        if c_listado is None:
            raise Falla(f"FALLA[escaneo-sin-listado]: {ruta} no define fn {listado}")
        if "es_nombre_ajeno_al_escaneo_de_epocas(" not in c_listado:
            raise Falla(f"FALLA[escaneo-sin-predicado]: {listado} ({ruta}) no llama es_nombre_ajeno_al_escaneo_de_epocas")
        if FILTRO_VIEJO.search(c_listado):
            raise Falla(f"FALLA[escaneo-filtro-viejo]: {listado} ({ruta}) conserva el filtro viejo en línea")
        if c_escaneo is None or f"{listado}(" not in c_escaneo:
            raise Falla(f"FALLA[escaneo-no-usa-listado]: {escaneo} ({ruta}) no llama {listado}")
        if FILTRO_VIEJO.search(c_escaneo):
            raise Falla(f"FALLA[escaneo-filtro-viejo]: {escaneo} ({ruta}) conserva el filtro viejo en línea")


def revisar_fuente_unica(retencion):
    c = cuerpo_de(retencion, "purgar_epocas_retiradas")
    if c is None or not re.search(r"numeros_de_epoca_marcados\(\s*ruta_datos\s*\)", c):
        raise Falla("FALLA[purga-sin-fuente-unica]: purgar_epocas_retiradas no llama numeros_de_epoca_marcados(ruta_datos)")
    if "leer_marcas_de_epoca_sospechosa(" in c:
        raise Falla("FALLA[purga-conjunto-propio]: purgar_epocas_retiradas arma su propio conjunto con leer_marcas_de_epoca_sospechosa")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True, text=True).stdout


def base_comun(repo):
    return git(repo, "merge-base", BASE, "HEAD").strip()


def en_base(repo, ruta):
    return git(repo, "show", f"{base_comun(repo)}:{ruta}")


def en_rama(repo, ruta):
    return git(repo, "show", f"{BASE}:{ruta}")


def en_arbol(repo, ruta):
    with open(os.path.join(repo, ruta), encoding="utf-8") as f:
        return f.read()


def modo_real(repo):
    fallas = []

    def paso(f, *a):
        try:
            f(*a)
        except Falla as e:
            fallas.append(str(e))

    par = lambda r: (en_base(repo, r), en_arbol(repo, r))
    paso(revisar_status, *par("docs/STATUS.md"))
    paso(revisar_plan, *par("docs/plan/fase-a-5-conocimiento-shadow-db.md"))
    adr_base = [os.path.basename(x) for x in git(repo, "ls-tree", "--name-only", f"{base_comun(repo)}:docs/adr").split()]
    adr_punta = numeros_adr(git(repo, "ls-tree", "--name-only", f"{BASE}:docs/adr").split())
    adr_nuevo = sorted(os.listdir(os.path.join(repo, "docs/adr")))
    nuevos = [x for x in adr_nuevo if x.startswith("adr-") and x not in adr_base]
    texto = en_arbol(repo, "docs/adr/" + nuevos[0]) if len(nuevos) == 1 else ""
    for x in numeros_adr(nuevos):
        if x in adr_punta:
            fallas.append(f"FALLA[adr-colision]: adr-{x:04d} ya existe en la punta de {BASE}; rebasar y renumerar")
    bit_punta = set(re.findall(r"^### D-(\d+)", en_rama(repo, "docs/bitacora-de-descartes.md"), re.M))
    bit_base = set(re.findall(r"^### D-(\d+)", en_base(repo, "docs/bitacora-de-descartes.md"), re.M))
    bit_arbol = set(re.findall(r"^### D-(\d+)", en_arbol(repo, "docs/bitacora-de-descartes.md"), re.M))
    for x in sorted((bit_arbol - bit_base) & bit_punta):
        fallas.append(f"FALLA[bitacora-colision]: D-{x} ya existe en la punta de {BASE}; rebasar y renumerar")
    paso(revisar_readme_adr, *par("docs/adr/README.md"), adr_base, [x for x in adr_nuevo if x.startswith("adr-")], texto)
    paso(revisar_bitacora, *par("docs/bitacora-de-descartes.md"))
    paso(revisar_firmas, *par("crates/hexcell-storage/src/retencion.rs"))
    paso(revisar_reexportes, *par("crates/hexcell-storage/src/lib.rs"))
    paso(revisar_reversion, *par("crates/hexcell-storage/src/reversion.rs"))
    rutas = [
        "crates/hexcell-storage/src/retencion.rs",
        "crates/hexcell-storage/src/promocion.rs",
        "crates/hexcell-storage/src/reversion.rs",
        "crates/hexcell-storage/src/lib.rs",
        "crates/hexcell/src/admin.rs",
    ]
    paso(revisar_recuentos, {r: par(r) for r in rutas})
    paso(revisar_escaneos, en_arbol(repo, rutas[0]), en_arbol(repo, rutas[1]))
    paso(revisar_fuente_unica, en_arbol(repo, rutas[0]))
    for f in fallas:
        print(f)
    if fallas:
        sys.exit(1)
    print("guarda-hex-093: OK")


def autoprueba():
    casos = []

    def caso(nombre, f, args, codigo):
        try:
            f(*args)
            obtenido = "OK"
        except Falla as e:
            obtenido = re.match(r"FALLA\[([^\]]+)\]", str(e)).group(1)
        casos.append((nombre, codigo, obtenido))

    st = "## Pendiente\n* **Valor**\n" + ENTRADA_STATUS + " (2026-08-31). Texto.\n* **Otra**\n"
    caso("status anexo", revisar_status, (st, st.replace("Texto.", "Texto. *(Actualización 2026-10-01, HEX-093: hijo a.)*")), "OK")
    caso("status sin cambio", revisar_status, (st, st), "status-sin-anexo")
    caso("status reescribe", revisar_status, (st, st.replace("Texto.", "Otro. HEX-093")), "status-no-anexa")
    caso("status otra línea", revisar_status, (st, st.replace("* **Otra**", "* **Otra** HEX-093")), "status-otra-linea")
    caso("status línea nueva", revisar_status, (st, st.replace("* **Otra**", "HEX-093\n* **Otra**")), "status-lineas")
    caso("status sin hex", revisar_status, (st, st.replace("Texto.", "Texto. más")), "status-sin-hex")

    pl = "7. **x**\n   a\n" + TAREA_8 + " (1,5 días).\n   b promoción.\n9. **y**\n   c\n"
    caso("plan línea nueva", revisar_plan, (pl, pl.replace("promoción.\n", "promoción.\n   HEX-093 frase.\n")), "OK")
    caso("plan sufijo", revisar_plan, (pl, pl.replace("promoción.", "promoción. HEX-093 frase.")), "OK")
    caso("plan borra", revisar_plan, (pl, pl.replace("   b promoción.\n", "   HEX-093\n")), "plan-no-anexa")
    caso("plan fuera", revisar_plan, (pl, pl.replace("   c\n", "   c HEX-093\n")), "plan-fuera-de-tarea-8")
    caso("plan sin cambio", revisar_plan, (pl, pl), "plan-sin-anexo")
    caso("plan sin hex", revisar_plan, (pl, pl.replace("promoción.", "promoción. frase.")), "plan-sin-hex")

    base = ["adr-0027-r.md", "adr-0040-p.md", "README.md"]
    rd = "| a |\n| `adr-0040-p.md` | x |\n\nLos ADR restantes\n"
    fila = "| `adr-0041-m.md` | y |\n"
    rd_ok = rd.replace("\n\nLos", "\n" + fila + "\nLos")
    caso("readme ok", revisar_readme_adr, (rd, rd_ok, base, base + ["adr-0041-m.md"], "extiende adr-0027"), "OK")
    caso("readme número", revisar_readme_adr, (rd, rd_ok.replace("0041", "0042"), base, base + ["adr-0042-m.md"], "adr-0027"), "adr-numero")
    caso("readme borra", revisar_readme_adr, (rd, rd_ok.replace("| a |\n", ""), base, base + ["adr-0041-m.md"], "adr-0027"), "readme-no-anexa")
    caso("readme sin fila", revisar_readme_adr, (rd, rd, base, base + ["adr-0041-m.md"], "adr-0027"), "readme-fila")
    caso("readme sin adr", revisar_readme_adr, (rd, rd_ok, base, base, "adr-0027"), "adr-archivos")
    caso("adr sin 0027", revisar_readme_adr, (rd, rd_ok, base, base + ["adr-0041-m.md"], "nada"), "adr-sin-0027")

    bi = "> Última actualización: 2026-09-30 (D-59).\n| [D-59](#d-59) | x |\n\n### D-59: x\n\nrazón 59\n"
    bi_ok = (bi.replace("(D-59).", "(D-60).").replace("| x |\n", "| x |\n| [D-60](#d-60) | y |\n")
             + "\n### D-60: y\n\ncuerpo\n")
    caso("bitácora sin cambio", revisar_bitacora, (bi, bi), "OK")
    caso("bitácora D-60", revisar_bitacora, (bi, bi_ok), "OK")
    caso("bitácora D-61", revisar_bitacora, (bi, bi_ok.replace("D-60", "D-61").replace("d-60", "d-61")), "bitacora-numero")
    caso("bitácora borra", revisar_bitacora, (bi, bi_ok.replace("razón 59\n", "")), "bitacora-no-anexa")
    caso("bitácora reescribe", revisar_bitacora, (bi, bi_ok.replace("razón 59", "razón cambiada")), "bitacora-no-anexa")
    caso("bitácora cabecera", revisar_bitacora, (bi, bi_ok.replace("(D-60).", "(D-59).")), "bitacora-cabecera")
    caso("bitácora índice", revisar_bitacora, (bi, bi_ok.replace("| [D-60](#d-60) | y |\n", "")), "bitacora-indice")

    rs = ("/// doc\npub const S: &str = \".s\";\npub struct M {\n    /// n\n    pub n: i64,\n}\n"
          "pub fn f(\n    a: &Path,\n) -> Result<(), E> {\n    x\n}\n")
    caso("firmas ok", revisar_firmas, (rs, rs + "pub fn g() {}\n"), "OK")
    caso("firmas cambia fn", revisar_firmas, (rs, rs.replace("a: &Path", "a: &Path, b: i64")), "firma-publica")
    caso("firmas cambia struct", revisar_firmas, (rs, rs.replace("pub n: i64,", "pub n: i64,\n    pub m: i64,")), "firma-publica")
    lib = "pub use retencion::{\n    A, b,\n};\n"
    caso("reexporte ok", revisar_reexportes, (lib, lib.replace("A, b", "A, C, b")), "OK")
    caso("reexporte quita", revisar_reexportes, (lib, lib.replace("A, b", "A")), "reexporte")
    rv = "x\nfn fecha_absoluta_de_hoy() -> String {\n}\n"
    caso("reversión ok", revisar_reversion, (rv, rv.replace("\nfn fecha", "\npub(crate) fn fecha")), "OK")
    caso("reversión otra", revisar_reversion, (rv, rv + "y\n"), "reversion")
    caso("recuento ok", revisar_recuentos, ({"a": ("remove_file(x)", "remove_file(x)\nrename(y)")},), "OK")
    caso("recuento borrado", revisar_recuentos, ({"a": ("remove_file(x)", "remove_file(x)\nremove_file(y)")},), "borrado")
    caso("recuento fsync", revisar_recuentos, ({"a": ("", "f.sync_all()")},), "fsync")
    ret = ("pub fn purgar_epocas_retiradas(g: &G, ruta_datos: &Path) -> R {\n"
           "    let numeros_marcados = numeros_de_epoca_marcados(ruta_datos)?;\n"
           "    for ruta in rutas_de_epoca_a_escanear_en_purga(ruta_datos)? { let f = format!(\"{x}\"); }\n}\n"
           "pub(crate) fn rutas_de_epoca_a_escanear_en_purga(ruta_datos: &Path) -> R {\n"
           "    if es_nombre_ajeno_al_escaneo_de_epocas(nombre) { continue; }\n}\n")
    pro = ("pub fn numero_de_epoca_siguiente(ruta_datos: &Path) -> R {\n"
           "    for ruta in rutas_de_epoca_a_escanear_para_numerar(ruta_datos)? {}\n}\n"
           "pub(crate) fn rutas_de_epoca_a_escanear_para_numerar(ruta_datos: &Path) -> R {\n"
           "    if crate::retencion::es_nombre_ajeno_al_escaneo_de_epocas(nombre) { continue; }\n}\n")
    viejo_r = "nombre.starts_with('.') || nombre.ends_with(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA)"
    viejo_p = "nombre.starts_with('.') || nombre.ends_with(crate::retencion::SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA)"
    caso("escaneos ok", revisar_escaneos, (ret, pro), "OK")
    caso("m4 purga filtro viejo", revisar_escaneos, (ret.replace("es_nombre_ajeno_al_escaneo_de_epocas(nombre)", viejo_r), pro), "escaneo-sin-predicado")
    caso("m5 numeración filtro viejo", revisar_escaneos, (ret, pro.replace("crate::retencion::es_nombre_ajeno_al_escaneo_de_epocas(nombre)", viejo_p)), "escaneo-sin-predicado")
    caso("purga refiltra en línea", revisar_escaneos, (ret.replace("let f = format", "if nombre.ends_with(SUFIJO_DE_MARCA_DE_EPOCA_SOSPECHOSA) {} let f = format"), pro), "escaneo-filtro-viejo")
    caso("purga no usa listado", revisar_escaneos, (ret.replace("in rutas_de_epoca_a_escanear_en_purga(ruta_datos)?", "in otras(ruta_datos)?"), pro), "escaneo-no-usa-listado")
    caso("numeración sin listado", revisar_escaneos, (ret, pro.replace("fn rutas_de_epoca_a_escanear_para_numerar", "fn otra")), "escaneo-sin-listado")
    caso("predicado comentado", revisar_escaneos, (ret.replace("    if es_nombre", "    // if es_nombre"), pro), "escaneo-sin-predicado")
    caso("fuente única ok", revisar_fuente_unica, (ret,), "OK")
    caso("m6 conjunto propio", revisar_fuente_unica, (ret.replace("numeros_de_epoca_marcados(ruta_datos)?", "leer_marcas_de_epoca_sospechosa(ruta_datos)?"),), "purga-sin-fuente-unica")
    caso("m6 conjunto propio además", revisar_fuente_unica, (ret.replace("    let numeros", "    let m = leer_marcas_de_epoca_sospechosa(ruta_datos)?;\n    let numeros"),), "purga-conjunto-propio")

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
