#!/usr/bin/env bash
# Guarda estática de HEX-096 (aplazamientos por horario y por rampa en la línea de métricas del sidecar).
#
# Juzga el árbol de trabajo (lo comprometido más lo no comprometido) frente a la base común
# `git merge-base main HEAD`, nunca solo contra HEAD: con el trabajo comprometido ese diff estaría
# vacío y la guarda nacería muerta. El número del ADR y del descarte nuevos se contrastan además con
# la PUNTA de `main` para detectar la colisión con una sesión paralela (FALLA[...-colision]).
#
# Reglas (cada fallo sale con un código FALLA[...] que nombra la causa):
#   docs/STATUS.md        solo se anexa texto al FINAL de la línea «(Hallazgo 9)»; el anexo nombra
#                         HEX-096; ninguna otra línea cambia.
#   docs/adr/README.md    solo líneas nuevas: exactamente una fila del ADR siguiente al más alto de
#                         `main`; el ADR nuevo existe, no existía en `main` y cita adr-0033 y adr-0035.
#   docs/bitacora-de-descartes.md
#                         gana exactamente UN descarte con el número siguiente al más alto de
#                         `main`, su fila de índice, y la cabecera «Última actualización» pasa a ese
#                         número (único reemplazo admitido); nada previo se borra ni se reescribe.
#   sidecar/internal/metricas/metricas_test.go
#                         solo líneas añadidas (ninguna borrada ni reescrita).
#   sidecar/go.mod, go.sum  byte-idénticos a la base.
#   metricas.go           no importa outbox, ipc ni canal; NuevoProductor conserva su firma; las
#                         claves aplazadas_por_horario y aplazadas_por_rampa van, en ese orden,
#                         después de contactos_omitidos y antes del bucle de ack_ratio.
#   main.go               el cierre inyectado devuelve ContadorAplazadasPorHorario antes que
#                         ContadorAplazadasPorRampa (ninguna prueba del paquete main lo cubre).
#
# Uso:  bash guarda-hex-096.sh                  (desde la raíz del worktree)
#       bash guarda-hex-096.sh --repo <ruta>
#       bash guarda-hex-096.sh --autoprueba      (casos sintéticos: cada regla vista en verde y en rojo)
set -euo pipefail

exec python3 - "$@" <<'PY'
import difflib
import os
import re
import subprocess
import sys

BASE = "main"
ENTRADA_STATUS = "* **(Hallazgo 9) Los aplazamientos por ventana y rampa son invisibles"
CABECERA_BITACORA = re.compile(r"Última actualización: \d{4}-\d{2}-\d{2} \(D-(\d+)\)\.")
IMPORTS_PROHIBIDOS = ("internal/outbox", "internal/ipc", "internal/canal")


class Falla(Exception):
    pass


def opcodes(v, n):
    return [o for o in difflib.SequenceMatcher(None, v, n, autojunk=False).get_opcodes() if o[0] != "equal"]


def revisar_status(viejo, nuevo):
    v, n = viejo.split("\n"), nuevo.split("\n")
    if len(v) != len(n):
        raise Falla("FALLA[status-lineas]: docs/STATUS.md cambió su número de líneas")
    cambios = [i for i in range(len(v)) if v[i] != n[i]]
    if not cambios:
        raise Falla("FALLA[status-sin-anexo]: falta el anexo HEX-096 en la línea del Hallazgo 9")
    for i in cambios:
        if not v[i].startswith(ENTRADA_STATUS):
            raise Falla(f"FALLA[status-otra-linea]: cambió la línea {i + 1}, que no es el Hallazgo 9")
        if not n[i].startswith(v[i]):
            raise Falla("FALLA[status-no-anexa]: el Hallazgo 9 perdió o reescribió texto existente")
        if "HEX-096" not in n[i][len(v[i]):]:
            raise Falla("FALLA[status-sin-hex]: el anexo no nombra HEX-096")


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
    for previo in ("adr-0033", "adr-0035"):
        if previo not in texto_adr_nuevo:
            raise Falla(f"FALLA[adr-sin-{previo[4:]}]: el ADR nuevo no cita {previo}")


def revisar_bitacora(viejo, nuevo):
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
            and (CABECERA_BITACORA.search(v[i1])) and (mn := CABECERA_BITACORA.search(n[j1]))
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


def revisar_solo_anade(viejo, nuevo, nombre):
    for tag, i1, i2, j1, j2 in opcodes(viejo.split("\n"), nuevo.split("\n")):
        if tag != "insert":
            raise Falla(f"FALLA[pruebas-no-anexa]: {nombre}: {tag} en la línea {i1 + 1} borra o reescribe texto")


def revisar_identico(viejo, nuevo, nombre):
    if viejo != nuevo:
        raise Falla(f"FALLA[modulo-cambiado]: {nombre} no es byte-idéntico a la base")


def sin_comentarios(texto):
    return "\n".join(re.sub(r"//.*$", "", l) for l in texto.split("\n"))


def revisar_metricas(viejo, nuevo):
    t = sin_comentarios(nuevo)
    m = re.search(r"import \((.*?)\)", t, re.S)
    for ruta in IMPORTS_PROHIBIDOS:
        if m and ruta in m.group(1):
            raise Falla(f"FALLA[metricas-importa]: metricas.go importa {ruta}; el paquete debe seguir siendo hoja")
    firma = "func NuevoProductor(reg *registro.Registro, ahoraMs func() int64) *Productor {"
    if firma not in nuevo:
        raise Falla("FALLA[firma-nuevoproductor]: NuevoProductor cambió su firma")
    if "func (p *Productor) ObservarAplazamientos(fuente func() (int64, int64))" not in nuevo:
        raise Falla("FALLA[metodo-ausente]: falta ObservarAplazamientos con la firma fijada")
    if "aplazamientosFuente func() (int64, int64)" not in nuevo:
        raise Falla("FALLA[campo-ausente]: falta el campo aplazamientosFuente")
    pos = [t.find(x) for x in (
        'fmt.Sprintf("contactos_omitidos=%d"', 'fmt.Sprintf("aplazadas_por_horario=%d"',
        'fmt.Sprintf("aplazadas_por_rampa=%d"', "for _, id := range ids")]
    if min(pos) < 0 or pos != sorted(pos):
        raise Falla("FALLA[metricas-orden]: las claves aplazadas_por_* faltan o no van entre contactos_omitidos y el bucle de ack_ratio, horario antes que rampa")
    for linea in viejo.split("\n"):
        if linea.strip().startswith('fmt.Sprintf("') and linea not in nuevo:
            raise Falla(f"FALLA[metricas-clave-previa]: desapareció o cambió «{linea.strip()[:60]}»")


def revisar_main(nuevo):
    t = sin_comentarios(nuevo)
    m = re.search(r"productorMetricas\.ObservarAplazamientos\(func\(\) \(int64, int64\) \{(.*?)\}\)", t, re.S)
    if not m:
        raise Falla("FALLA[main-sin-cableado]: sidecar/main.go no cablea ObservarAplazamientos")
    h = m.group(1).find("outbox.ContadorAplazadasPorHorario.Load()")
    r = m.group(1).find("outbox.ContadorAplazadasPorRampa.Load()")
    if h < 0 or r < 0 or h > r:
        raise Falla("FALLA[main-cierre-intercambiado]: el cierre debe devolver horario y luego rampa, ambos con .Load()")
    if t.find("metricas.NuevoProductor(reg, nil)") > m.start() or t.find("metricas.NuevoProductor(reg, nil)") < 0:
        raise Falla("FALLA[main-orden]: el cableado va antes de NuevoProductor o este desapareció")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True, text=True).stdout


def base_comun(repo):
    return git(repo, "merge-base", BASE, "HEAD").strip()


def en_base(repo, ruta):
    return git(repo, "show", f"{base_comun(repo)}:{ruta}")


def en_punta(repo, ruta):
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
    adr_base = [os.path.basename(x) for x in git(repo, "ls-tree", "--name-only", f"{base_comun(repo)}:docs/adr").split()]
    adr_punta = numeros_adr(git(repo, "ls-tree", "--name-only", f"{BASE}:docs/adr").split())
    adr_nuevo = sorted(x for x in os.listdir(os.path.join(repo, "docs/adr")) if x.startswith("adr-"))
    nuevos = [x for x in adr_nuevo if x not in adr_base]
    texto = en_arbol(repo, "docs/adr/" + nuevos[0]) if len(nuevos) == 1 else ""
    for x in numeros_adr(nuevos):
        if x in adr_punta:
            fallas.append(f"FALLA[adr-colision]: adr-{x:04d} ya existe en la punta de {BASE}; rebasar y renumerar")
    bit_punta = set(re.findall(r"^### D-(\d+)", en_punta(repo, "docs/bitacora-de-descartes.md"), re.M))
    bit_base = set(re.findall(r"^### D-(\d+)", en_base(repo, "docs/bitacora-de-descartes.md"), re.M))
    bit_arbol = set(re.findall(r"^### D-(\d+)", en_arbol(repo, "docs/bitacora-de-descartes.md"), re.M))
    for x in sorted((bit_arbol - bit_base) & bit_punta):
        fallas.append(f"FALLA[bitacora-colision]: D-{x} ya existe en la punta de {BASE}; rebasar y renumerar")
    paso(revisar_readme_adr, *par("docs/adr/README.md"), adr_base, adr_nuevo, texto)
    paso(revisar_bitacora, *par("docs/bitacora-de-descartes.md"))
    paso(revisar_solo_anade, *par("sidecar/internal/metricas/metricas_test.go"), "metricas_test.go")
    for r in ("sidecar/go.mod", "sidecar/go.sum"):
        paso(revisar_identico, *par(r), r)
    paso(revisar_metricas, en_base(repo, "sidecar/internal/metricas/metricas.go"), en_arbol(repo, "sidecar/internal/metricas/metricas.go"))
    paso(revisar_main, en_arbol(repo, "sidecar/main.go"))
    for f in fallas:
        print(f)
    if fallas:
        sys.exit(1)
    print("guarda-hex-096: OK")


def autoprueba():
    casos = []

    def caso(nombre, f, args, codigo):
        try:
            f(*args)
            obtenido = "OK"
        except Falla as e:
            obtenido = re.match(r"FALLA\[([^\]]+)\]", str(e)).group(1)
        casos.append((nombre, codigo, obtenido))

    st = "## Pendiente\n" + ENTRADA_STATUS + " (2026-08-20). Texto.\n* **(Hallazgo 10) otra**\n"
    caso("status anexo", revisar_status, (st, st.replace("Texto.", "Texto. *(Resuelto 2026-10-08, `HEX-096`: dos claves.)*")), "OK")
    caso("status sin cambio", revisar_status, (st, st), "status-sin-anexo")
    caso("status reescribe", revisar_status, (st, st.replace("Texto.", "Otro. HEX-096")), "status-no-anexa")
    caso("status borra literal", revisar_status, (st, st.replace(" (2026-08-20).", " HEX-096")), "status-no-anexa")
    caso("status otra línea", revisar_status, (st, st.replace("otra**", "otra** HEX-096")), "status-otra-linea")
    caso("status línea nueva", revisar_status, (st, st.replace("* **(Hallazgo 10)", "HEX-096\n* **(Hallazgo 10)")), "status-lineas")
    caso("status sin hex", revisar_status, (st, st.replace("Texto.", "Texto. más")), "status-sin-hex")

    base = ["adr-0033-m.md", "adr-0035-l.md", "adr-0041-a.md", "README.md"]
    rd = "| a |\n| `adr-0041-a.md` | x |\n\nLos ADR restantes\n"
    fila = "| `adr-0042-m.md` | y |\n"
    rd_ok = rd.replace("\n\nLos", "\n" + fila + "\nLos")
    ok_args = (rd, rd_ok, base, base + ["adr-0042-m.md"], "extiende adr-0033 y adr-0035")
    caso("readme ok", revisar_readme_adr, ok_args, "OK")
    caso("readme número", revisar_readme_adr, (rd, rd_ok.replace("0042", "0043"), base, base + ["adr-0043-m.md"], ok_args[4]), "adr-numero")
    caso("readme borra", revisar_readme_adr, (rd, rd_ok.replace("| a |\n", ""), *ok_args[2:]), "readme-no-anexa")
    caso("readme reescribe", revisar_readme_adr, (rd, rd_ok.replace("| x |", "| z |"), *ok_args[2:]), "readme-no-anexa")
    caso("readme sin fila", revisar_readme_adr, (rd, rd, *ok_args[2:]), "readme-fila")
    caso("readme sin adr", revisar_readme_adr, (rd, rd_ok, base, base, ok_args[4]), "adr-archivos")
    caso("adr sin 0033", revisar_readme_adr, (*ok_args[:4], "solo adr-0035"), "adr-sin-0033")
    caso("adr sin 0035", revisar_readme_adr, (*ok_args[:4], "solo adr-0033"), "adr-sin-0035")

    bi = "> Última actualización: 2026-09-30 (D-60).\n| [D-60](#d-60) | x |\n\n### D-60: x\n\nrazón 60\n"
    bi_ok = (bi.replace("(D-60).", "(D-61).").replace("| x |\n", "| x |\n| [D-61](#d-61) | y |\n") + "\n### D-61: y\n\ncuerpo\n")
    caso("bitácora D-61", revisar_bitacora, (bi, bi_ok), "OK")
    caso("bitácora sin cambio", revisar_bitacora, (bi, bi), "bitacora-numero")
    caso("bitácora D-62", revisar_bitacora, (bi, bi_ok.replace("D-61", "D-62").replace("d-61", "d-62")), "bitacora-numero")
    caso("bitácora borra", revisar_bitacora, (bi, bi_ok.replace("razón 60\n", "")), "bitacora-no-anexa")
    caso("bitácora reescribe", revisar_bitacora, (bi, bi_ok.replace("razón 60", "razón cambiada")), "bitacora-no-anexa")
    caso("bitácora cabecera", revisar_bitacora, (bi, bi_ok.replace("(D-61).", "(D-60).")), "bitacora-cabecera")
    caso("bitácora índice", revisar_bitacora, (bi, bi_ok.replace("| [D-61](#d-61) | y |\n", "")), "bitacora-indice")

    tt = "func TestA(t *testing.T) {\n\tx\n}\n"
    caso("pruebas añade", revisar_solo_anade, (tt, tt + "func TestB(t *testing.T) {\n}\n", "t"), "OK")
    caso("pruebas borra", revisar_solo_anade, (tt, tt.replace("\tx\n", ""), "t"), "pruebas-no-anexa")
    caso("pruebas reescribe", revisar_solo_anade, (tt, tt.replace("\tx\n", "\ty\n"), "t"), "pruebas-no-anexa")
    caso("módulo igual", revisar_identico, ("a", "a", "go.mod"), "OK")
    caso("módulo cambia", revisar_identico, ("a", "a ", "go.mod"), "modulo-cambiado")

    mv = ('import (\n\t"fmt"\n\t"github.com/CGary/hexcell/sidecar/internal/registro"\n)\n'
          "func NuevoProductor(reg *registro.Registro, ahoraMs func() int64) *Productor {\n}\n"
          '\tpartes := []string{\n\t\tfmt.Sprintf("contactos_omitidos=%d", p.c),\n\t}\n\tfor _, id := range ids {\n\t}\n')
    mn = mv.replace('\t\tfmt.Sprintf("contactos_omitidos=%d", p.c),\n',
                    '\t\tfmt.Sprintf("contactos_omitidos=%d", p.c),\n\t\tfmt.Sprintf("aplazadas_por_horario=%d", h),\n\t\tfmt.Sprintf("aplazadas_por_rampa=%d", r),\n')
    mn += ("type Productor struct {\n\taplazamientosFuente func() (int64, int64)\n}\n"
           "func (p *Productor) ObservarAplazamientos(fuente func() (int64, int64)) {\n}\n")
    caso("métricas ok", revisar_metricas, (mv, mn), "OK")
    caso("métricas importa outbox", revisar_metricas, (mv, mn.replace('\t"fmt"\n', '\t"fmt"\n\t"github.com/CGary/hexcell/sidecar/internal/outbox"\n')), "metricas-importa")
    caso("métricas importa canal", revisar_metricas, (mv, mn.replace('\t"fmt"\n', '\t"fmt"\n\t"github.com/CGary/hexcell/sidecar/internal/canal"\n')), "metricas-importa")
    caso("métricas firma", revisar_metricas, (mv, mn.replace("ahoraMs func() int64", "ahoraMs func() int64, x int")), "firma-nuevoproductor")
    caso("métricas sin método", revisar_metricas, (mv, mn.replace("ObservarAplazamientos", "Otro")), "metodo-ausente")
    caso("métricas sin campo", revisar_metricas, (mv, mn.replace("aplazamientosFuente func", "otro func")), "campo-ausente")
    caso("métricas swap claves", revisar_metricas, (mv, mn.replace("horario", "TMP").replace("rampa", "horario").replace("TMP", "rampa")), "metricas-orden")
    caso("métricas sin una clave", revisar_metricas, (mv, mn.replace('\t\tfmt.Sprintf("aplazadas_por_rampa=%d", r),\n', "")), "metricas-orden")
    caso("métricas claves tras ack", revisar_metricas, (mv, mn.replace('\t\tfmt.Sprintf("aplazadas_por_horario=%d", h),\n\t\tfmt.Sprintf("aplazadas_por_rampa=%d", r),\n', "").replace("\tfor _, id := range ids {\n\t}\n", '\tfor _, id := range ids {\n\t}\n\tpartes = append(partes, fmt.Sprintf("aplazadas_por_horario=%d", h), fmt.Sprintf("aplazadas_por_rampa=%d", r))\n')), "metricas-orden")
    caso("métricas pierde clave previa", revisar_metricas, (mv + '\t\tfmt.Sprintf("silencio_entrante_ms=%d", s),\n', mn), "metricas-clave-previa")

    mm = ("productorMetricas := metricas.NuevoProductor(reg, nil)\n"
          "productorMetricas.ObservarAplazamientos(func() (int64, int64) {\n"
          "\treturn outbox.ContadorAplazadasPorHorario.Load(), outbox.ContadorAplazadasPorRampa.Load()\n})\n")
    caso("main ok", revisar_main, (mm,), "OK")
    caso("main sin cableado", revisar_main, (mm.replace("ObservarAplazamientos", "Otro"),), "main-sin-cableado")
    caso("main swap cierre", revisar_main, (mm.replace("Horario", "TMP").replace("Rampa", "Horario").replace("TMP", "Rampa"),), "main-cierre-intercambiado")
    caso("main sin Load", revisar_main, (mm.replace("ContadorAplazadasPorRampa.Load()", "ContadorAplazadasPorRampa"),), "main-cierre-intercambiado")
    caso("main antes de constructor", revisar_main, ("productorMetricas.ObservarAplazamientos(func() (int64, int64) {\n\treturn outbox.ContadorAplazadasPorHorario.Load(), outbox.ContadorAplazadasPorRampa.Load()\n})\nproductorMetricas := metricas.NuevoProductor(reg, nil)\n",), "main-orden")

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
