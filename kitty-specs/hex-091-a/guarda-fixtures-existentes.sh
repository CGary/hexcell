#!/usr/bin/env bash
# Guarda de la excepción ratificada por el humano el 2026-09-30 para HEX-091-a.
#
# En los 18 archivos de prueba PREEXISTENTES de la lista solo se admiten cinco formas de edición
# mecánica:
#   (i)   el literal de la versión de cable 6 -> 7, sin tocar nada más de la línea;
#   (ii)  el literal de cabecera del documento IPC en documento_test.go:
#         "1.5, fijada el 2026-09-11." -> "1.6, fijada el AAAA-MM-DD.";
#   (iii) la línea que inicializa el campo nuevo de OperacionesDeSesion en admin_http.rs:
#         restablecer_contacto: hexcell::admin::restablecimiento_no_disponible(),
#   (iv)  en mensajes_test.go, SOLO líneas añadidas dentro del literal de cuerposDeMuestra que
#         forman EXACTAMENTE dos entradas nuevas, ipc.TipoOrdenRestablecerContacto e
#         ipc.TipoAcuseRestablecerContacto, con cuerpos que ejercitan incluir_baja, el
#         discriminante existe y los tres contadores por tabla; ninguna entrada existente cambia;
#   (v)   en nueve archivos de crates/hexcell-canal-whatsmeow/tests, `enviar_saludo(6,` ->
#         `enviar_saludo(7,` en la misma línea, con EXACTAMENTE 45 pares y el reparto por
#         archivo de la base 43237ea (igualdad, no «al menos»).
# Además, todas esas ediciones viven en UN solo commit `test:` que no toca ningún otro archivo.
# Cualquier otra línea añadida o quitada en esos archivos, una línea quitada sin su reemplazo
# mecánico exacto, o un cambio en cualquier OTRO archivo de prueba preexistente, sale en rojo con
# un código FALLA[...] que nombra la causa. Un diff vacío (la base) pasa.
#
# Estados que se juzgan: lo comprometido (`git diff main...HEAD`) y el árbol de trabajo frente a
# la base común (lo comprometido más lo que aún no se comprometió). Nunca solo contra HEAD: con el
# trabajo comprometido ese diff está vacío y la guarda nacería muerta.
#
# Uso:  bash guarda-fixtures-existentes.sh                 (desde la raíz del worktree)
#       bash guarda-fixtures-existentes.sh --repo <ruta>    (misma guarda, sin cambiar de directorio)
#       bash guarda-fixtures-existentes.sh --autoprueba
set -euo pipefail

exec python3 - "$@" <<'PY'
import difflib
import os
import re
import subprocess
import sys
from collections import Counter

BASE = "main"
DOCUMENTO_TEST = "sidecar/internal/ipc/documento_test.go"
ADMIN_HTTP = "crates/hexcell/tests/admin_http.rs"
MENSAJES_TEST = "sidecar/internal/ipc/mensajes_test.go"
W = "crates/hexcell-canal-whatsmeow/tests/"

# Forma (v): pares `enviar_saludo(6,` por archivo en la base 43237ea (git grep -c). Total 45.
CONTEOS_FORMA_V = {
    W + "cierre_de_sesion.rs": 4,
    W + "contrato_del_puerto.rs": 1,
    W + "emparejamiento.rs": 15,
    W + "privacidad.rs": 1,
    W + "protocolo.rs": 5,
    W + "reconexion.rs": 5,
    W + "respaldo_sqlstore.rs": 6,
    W + "salida.rs": 4,
    W + "senales_de_alerta.rs": 4,
}
TOTAL_FORMA_V = 45

ARCHIVOS = sorted({
    MENSAJES_TEST,
    DOCUMENTO_TEST,
    "sidecar/internal/servidor/servidor_test.go",
    W + "comun/mod.rs",
    W + "protocolo.rs",
    W + "cierre_de_sesion.rs",
    W + "salida.rs",
    W + "emparejamiento.rs",
    ADMIN_HTTP,
    "crates/hexcell/tests/emparejamiento_ipc.rs",
    "crates/hexcell/tests/respaldo_cli.rs",
    "crates/hexcell/tests/respaldo_sqlstore_ipc.rs",
    "crates/hexcell/tests/canal_whatsmeow_seleccionado.rs",
} | set(CONTEOS_FORMA_V))  # 13 de las formas i-iv + 5 que solo trae la forma v = 18

# Forma (i): el dígito 6 de la versión de cable, en los únicos contextos en que aparece en las
# pruebas de la base 43237ea, seguido de su delimitador. El reemplazo toca SOLO ese dígito.
FORMA_I = re.compile(
    r'(?P<pre>"version":|\\"version\\":|\bversion: |\.version, |\(propia, |propia=|esperada )6(?=[,);"])'
)
FORMA_V_VIEJA, FORMA_V_NUEVA = "enviar_saludo(6,", "enviar_saludo(7,"
# Forma (ii): literal de cabecera del documento.
CABECERA_VIEJA = "1.5, fijada el 2026-09-11."
CABECERA_NUEVA = re.compile(r"1\.6, fijada el \d{4}-\d{2}-\d{2}\.")
CABECERA_CANONICA = "1.6, fijada el FECHA."
# Forma (iii): la única línea añadida admitida en admin_http.rs.
FORMA_III = re.compile(
    r"^[ \t]*restablecer_contacto: hexcell::admin::restablecimiento_no_disponible\(\),$"
)
# Forma (iv): las dos entradas nuevas de cuerposDeMuestra y los campos que deben ejercitar.
ENTRADAS_IV = {
    "ipc.TipoOrdenRestablecerContacto": (
        "ipc.OrdenRestablecerContacto",
        {"Contacto": "cadena", "IncluirBaja": "cadena"},
    ),
    "ipc.TipoAcuseRestablecerContacto": (
        "ipc.AcuseRestablecerContacto",
        {
            "Contacto": "cadena",
            "IncluirBaja": "cadena",
            "Resultado": "cadena",
            "Existe": "cadena",
            "Cortacircuitos": "entero",
            "PresentacionDeConversacion": "entero",
            "BajaDeContacto": "entero",
        },
    ),
}
VALOR_CADENA = re.compile(r'^(ipc\.\w+|"[^"]+"|`[^`]+`)$')
VALOR_ENTERO = re.compile(r"^[1-9][0-9_]*$")
CAMPO = re.compile(r"(?<![\w.])(\w+):\s*([^,\n{}]+?)\s*(?=,|\}|$)", re.M)

ES_PRUEBA = re.compile(r"(_test\.go$)|(^crates/[^/]+/tests/)")
CABECERA_HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")
ASUNTO_TEST = re.compile(r"^test(\([^)]*\))?: ")


def esperada(archivo, quitada):
    """(línea de reemplazo exacta exigida por las formas i, ii o v, es_forma_v); None si ninguna."""
    nueva = FORMA_I.sub(lambda m: m.group("pre") + "7", quitada)
    es_v = archivo in CONTEOS_FORMA_V and FORMA_V_VIEJA in nueva
    if es_v:
        nueva = nueva.replace(FORMA_V_VIEJA, FORMA_V_NUEVA)
    if archivo == DOCUMENTO_TEST and CABECERA_VIEJA in nueva:
        nueva = nueva.replace(CABECERA_VIEJA, CABECERA_CANONICA)
    return (None, False) if nueva == quitada else (nueva, es_v)


def canonica(archivo, agregada):
    if archivo == DOCUMENTO_TEST:
        return CABECERA_NUEVA.sub(CABECERA_CANONICA, agregada)
    return agregada


def rango_de_muestras(contenido):
    """(primera, última) línea, 1-based, del interior del literal de cuerposDeMuestra."""
    if contenido is None:
        return None
    lineas = contenido.split("\n")
    inicio = next((i for i, l in enumerate(lineas) if l.startswith("func cuerposDeMuestra() ")), None)
    if inicio is None or inicio + 1 >= len(lineas):
        return None
    if not lineas[inicio + 1].startswith("\treturn map[") or not lineas[inicio + 1].endswith("{"):
        return None
    for j in range(inicio + 2, len(lineas)):
        if lineas[j] == "\t}":
            return (inicio + 3, j)
    return None


def entradas_de_muestras(contenido):
    rango = rango_de_muestras(contenido)
    if rango is None:
        return None
    entradas, actual = [], None
    for linea in contenido.split("\n")[rango[0] - 1 : rango[1]]:
        if actual is not None:
            actual.append(linea)
            if linea == "\t\t},":
                entradas.append("\n".join(actual))
                actual = None
            continue
        if linea.startswith("\t\t") and linea.endswith("{"):
            actual = [linea]
        else:
            entradas.append(linea)
    if actual is not None:
        entradas.append("\n".join(actual))
    return entradas


def verificar_muestras(base, nuevo):
    """Forma (iv): las entradas de la base intactas y en orden, más exactamente las dos nuevas."""
    fallas = []
    viejas, nuevas = entradas_de_muestras(base), entradas_de_muestras(nuevo)
    if viejas is None or nuevas is None:
        return [("MUESTRAS-ILOCALIZABLES", "no se encontró el literal de cuerposDeMuestra")]
    emparejadas_viejas, emparejadas_nuevas = set(), set()
    for bloque in difflib.SequenceMatcher(None, viejas, nuevas, autojunk=False).get_matching_blocks():
        emparejadas_viejas.update(range(bloque.a, bloque.a + bloque.size))
        emparejadas_nuevas.update(range(bloque.b, bloque.b + bloque.size))
    clave_de = lambda e: (re.match(r"^\t\t(ipc\.\w+):", e) or [None, None])[1]
    alteradas = Counter()
    for i, entrada in enumerate(viejas):
        if i not in emparejadas_viejas:
            fallas.append(("MUESTRA-EXISTENTE-ALTERADA", f"entrada de la base cambiada, movida o borrada: {entrada.splitlines()[0].strip()}"))
            alteradas[clave_de(entrada)] += 1
    agregadas = []
    for i, entrada in enumerate(nuevas):
        if i in emparejadas_nuevas:
            continue
        clave = clave_de(entrada)
        if alteradas[clave] > 0:
            alteradas[clave] -= 1  # versión modificada de una entrada existente, ya reportada
            continue
        agregadas.append(entrada)
    claves = []
    for entrada in agregadas:
        m = re.match(r"^\t\t(ipc\.\w+): (ipc\.\w+)\{", entrada)
        claves.append(m.group(1) if m else None)
        if not m or m.group(1) not in ENTRADAS_IV:
            continue
        tipo_esperado, campos = ENTRADAS_IV[m.group(1)]
        if m.group(2) != tipo_esperado:
            fallas.append(("MUESTRA-SIN-CAMPOS", f"{m.group(1)} no construye {tipo_esperado}"))
            continue
        valores = {c: v for c, v in CAMPO.findall(entrada[m.end():])}
        for campo, clase in campos.items():
            valor = valores.get(campo)
            patron = VALOR_CADENA if clase == "cadena" else VALOR_ENTERO
            if valor is None or not patron.match(valor):
                fallas.append(("MUESTRA-SIN-CAMPOS", f"{m.group(1)}.{campo} ausente o vacío ({valor!r})"))
    if sorted(c for c in claves if c) != sorted(ENTRADAS_IV) or len(claves) != len(ENTRADAS_IV):
        fallas.append(("MUESTRAS-NUEVAS-INCORRECTAS", f"se esperaban exactamente {sorted(ENTRADAS_IV)}, hay {claves}"))
    return fallas


def verificar_diff(texto, obtener_nuevo):
    """(fallas, pares de forma v por archivo) de un diff unificado -U0."""
    fallas = []
    quitadas, agregadas = [], []
    archivo, en_cabecera, numero = None, False, 0
    for linea in texto.splitlines():
        if linea.startswith("diff --git "):
            archivo = linea.split(" b/", 1)[1] if " b/" in linea else None
            en_cabecera = True
            continue
        m = CABECERA_HUNK.match(linea)
        if m:
            en_cabecera, numero = False, int(m.group(1))
            continue
        if en_cabecera or linea.startswith("\\ No newline"):
            continue
        if linea.startswith("-"):
            quitadas.append((archivo, linea[1:]))
        elif linea.startswith("+"):
            agregadas.append((archivo, linea[1:], numero))
            numero += 1
    pendientes, pendientes_v, pares_v = Counter(), Counter(), Counter()
    for archivo, quitada in quitadas:
        nueva, es_v = esperada(archivo, quitada)
        if nueva is None:
            fallas.append(("QUITADA-FUERA-DE-FORMA", f"{archivo}: -{quitada.strip()}"))
        else:
            pendientes[(archivo, nueva)] += 1
            pendientes_v[(archivo, nueva)] += es_v
    rangos = {}
    for archivo, agregada, numero in agregadas:
        clave = (archivo, canonica(archivo, agregada))
        if pendientes[clave] > 0:
            pendientes[clave] -= 1
            if pendientes_v[clave] > 0:  # un par cuenta una sola vez aunque la línea lleve i y v
                pendientes_v[clave] -= 1
                pares_v[archivo] += 1
            continue
        if archivo == ADMIN_HTTP and FORMA_III.match(agregada):
            continue
        if archivo == MENSAJES_TEST:
            if archivo not in rangos:
                rangos[archivo] = rango_de_muestras(obtener_nuevo(archivo))
            rango = rangos[archivo]
            if rango is not None and rango[0] <= numero <= rango[1]:
                continue  # su contenido lo juzga verificar_muestras
        fallas.append(("AGREGADA-FUERA-DE-FORMA", f"{archivo}:{numero}: +{agregada.strip()}"))
    for (archivo, nueva), restantes in pendientes.items():
        if restantes > 0:
            fallas.append(("REEMPLAZO-AUSENTE", f"{archivo}: falta +{nueva.strip()} (x{restantes})"))
    return fallas, pares_v


def verificar_conteo_v(pares_v):
    """Igualdad exacta del número de pares de la forma v, por archivo y en total."""
    fallas = []
    for archivo, esperado in CONTEOS_FORMA_V.items():
        if pares_v.get(archivo, 0) != esperado:
            fallas.append(("FORMA-V-CONTEO", f"{archivo}: {pares_v.get(archivo, 0)} pares, se esperaban {esperado}"))
    total = sum(pares_v.values())
    if total != TOTAL_FORMA_V:
        fallas.append(("FORMA-V-CONTEO", f"total {total} pares, se esperaban exactamente {TOTAL_FORMA_V}"))
    return fallas


def verificar_nombres(estado_por_archivo, preexistentes):
    """Todo archivo de prueba preexistente fuera de la lista debe quedar intacto."""
    fallas = []
    for ruta, estado in estado_por_archivo:
        if ruta in ARCHIVOS and estado == "M":
            continue
        if ruta in preexistentes or ruta in ARCHIVOS:
            fallas.append(("ARCHIVO-DE-PRUEBA-AJENO", f"{estado} {ruta}"))
    return fallas


def verificar_commits(commits):
    """commits: [(sha, asunto, [archivos])] de main..HEAD, del más viejo al más nuevo.
    Todas las ediciones mecánicas viven en UN solo commit `test:` que no toca nada más."""
    mecanicos = [c for c in commits if set(c[2]) & set(ARCHIVOS)]
    if not mecanicos:
        return []
    if len(mecanicos) > 1:
        lista = ", ".join(f"{sha[:7]} {asunto}" for sha, asunto, _ in mecanicos)
        return [("MECANICO-EN-VARIOS-COMMITS", f"{len(mecanicos)} commits tocan archivos de la lista: {lista}")]
    sha, asunto, archivos = mecanicos[0]
    fallas = []
    if not ASUNTO_TEST.match(asunto):
        fallas.append(("MECANICO-SIN-COMMIT-TEST", f"{sha[:7]} '{asunto}' no es un commit test:"))
    ajenos = sorted(set(archivos) - set(ARCHIVOS))
    if ajenos:
        fallas.append(("MECANICO-MEZCLADO", f"{sha[:7]} también toca {ajenos}"))
    return fallas


def verificar_estado(diff, obtener_nuevo, contenido_base_mensajes, contenido_nuevo_mensajes):
    """Juicio completo de un estado (lo comprometido, o el árbol de trabajo) frente a la base."""
    if not diff.strip():
        return []  # la base, o nada que juzgar en este estado
    fallas, pares_v = verificar_diff(diff, obtener_nuevo)
    fallas += verificar_conteo_v(pares_v)
    if contenido_base_mensajes != contenido_nuevo_mensajes:
        fallas += verificar_muestras(contenido_base_mensajes, contenido_nuevo_mensajes)
    return fallas


def git(*argumentos):
    return subprocess.run(["git", *argumentos], check=True, capture_output=True, text=True).stdout


def mostrar(ref, archivo):
    resultado = subprocess.run(["git", "show", f"{ref}:{archivo}"], capture_output=True, text=True)
    return resultado.stdout if resultado.returncode == 0 else None


def leer(archivo):
    try:
        with open(archivo, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return None


def nombres(*rango):
    pares = []
    for linea in git("diff", "--name-status", "--no-renames", *rango).splitlines():
        partes = linea.split("\t")
        if len(partes) >= 2 and ES_PRUEBA.search(partes[-1]):
            pares.append((partes[-1], partes[0][:1]))
    return pares


def principal():
    preexistentes = {r for r in git("ls-tree", "-r", "--name-only", BASE).splitlines() if ES_PRUEBA.search(r)}
    faltan = [r for r in ARCHIVOS if r not in preexistentes]
    if faltan:
        print(f"FALLA[LISTA-DESACTUALIZADA] no existen en {BASE}: {faltan}")
        return 1
    base_comun = git("merge-base", BASE, "HEAD").strip()
    antes = mostrar(base_comun, MENSAJES_TEST)
    fallas = []
    # Estado 1: lo comprometido en la rama.
    diff = git("diff", "-U0", "--no-color", "--no-ext-diff", f"{BASE}...HEAD", "--", *ARCHIVOS)
    fallas += verificar_estado(diff, lambda a: mostrar("HEAD", a), antes, mostrar("HEAD", MENSAJES_TEST))
    fallas += verificar_nombres(nombres(f"{BASE}...HEAD"), preexistentes)
    # Estado 2: el árbol de trabajo frente a la base común (comprometido + sin comprometer).
    diff = git("diff", "-U0", "--no-color", "--no-ext-diff", base_comun, "--", *ARCHIVOS)
    fallas += verificar_estado(diff, leer, antes, leer(MENSAJES_TEST))
    fallas += verificar_nombres(nombres(base_comun), preexistentes)
    # Forma de la historia: un solo commit test: con todas las ediciones mecánicas.
    commits = []
    for sha in git("rev-list", "--reverse", f"{BASE}..HEAD").split():
        asunto = git("log", "-1", "--format=%s", sha).strip()
        archivos = git("diff-tree", "--no-commit-id", "--name-only", "-r", "--root", sha).split("\n")
        commits.append((sha, asunto, [a for a in archivos if a]))
    fallas += verificar_commits(commits)
    vistos = set()
    for codigo, detalle in fallas:
        if (codigo, detalle) not in vistos:
            vistos.add((codigo, detalle))
            print(f"FALLA[{codigo}] {detalle}")
    if fallas:
        return 1
    print("OK: los archivos de prueba preexistentes solo llevan las cinco formas mecánicas ratificadas, en un solo commit test:")
    return 0


# ---------------------------------------------------------------------------------------------
# Autoprueba: cada caso comprueba primero que su mutación CAMBIA la entrada respecto del caso
# válido, y después que la guarda devuelve exactamente el conjunto de códigos esperado.
# ---------------------------------------------------------------------------------------------

def diff_sintetico(archivo, quitadas, agregadas):
    cuerpo = [f"diff --git a/{archivo} b/{archivo}", "index 0000000..1111111 100644",
              f"--- a/{archivo}", f"+++ b/{archivo}", "@@ -1 +1 @@"]
    cuerpo += ["-" + q for q in quitadas] + ["+" + a for a in agregadas]
    return "\n".join(cuerpo) + "\n"


def diff_de_contenidos(archivo, antes, despues):
    cuerpo = [f"diff --git a/{archivo} b/{archivo}", "index 0000000..1111111 100644"]
    cuerpo += list(difflib.unified_diff(antes.split("\n"), despues.split("\n"),
                                        f"a/{archivo}", f"b/{archivo}", n=0, lineterm=""))
    return "\n".join(cuerpo) + "\n"


BASE_MENSAJES = "\n".join([
    "package ipc_test",
    "",
    "func cuerposDeMuestra() map[ipc.TipoMensaje]ipc.Cuerpo {",
    "\treturn map[ipc.TipoMensaje]ipc.Cuerpo{",
    "\t\tipc.TipoSaludo: ipc.Saludo{",
    "\t\t\tEmisor:   ipc.EmisorSidecar,",
    '\t\t\tIdCelula: "piloto-01",',
    "\t\t},",
    "\t\tipc.TipoAcusePausaDeEnvio: ipc.AcusePausaDeEnvio{",
    "\t\t\tAccion:    ipc.AccionPausarEnvio,",
    "\t\t\tResultado: ipc.ResultadoPausaAplicado,",
    '\t\t\tMotivo:    "",',
    "\t\t},",
    "\t}",
    "}",
    "",
    "func TestUnaLineaCruda(t *testing.T) {",
    '\tlinea := []byte(`{"version":6,"tipo":"tipo_inexistente","texto":"hola"}` + "\\n")',
    "}",
    "",
])
ORDEN_IV = [
    "\t\tipc.TipoOrdenRestablecerContacto: ipc.OrdenRestablecerContacto{",
    '\t\t\tContacto:    "ct-0123456789abcdef0123456789abcdef",',
    "\t\t\tIncluirBaja: ipc.ValorSi,",
    "\t\t},",
]
ACUSE_IV = [
    "\t\tipc.TipoAcuseRestablecerContacto: ipc.AcuseRestablecerContacto{",
    '\t\t\tContacto:                   "ct-0123456789abcdef0123456789abcdef",',
    "\t\t\tIncluirBaja:                ipc.ValorSi,",
    "\t\t\tResultado:                  ipc.ResultadoRestablecimientoAplicado,",
    "\t\t\tExiste:                     ipc.ValorSi,",
    "\t\t\tCortacircuitos:             1,",
    "\t\t\tPresentacionDeConversacion: 2,",
    "\t\t\tBajaDeContacto:             3,",
    '\t\t\tMotivo:                     "",',
    "\t\t},",
]


def con_entradas(base, *bloques, tras="\t\t},\n\t}"):
    insercion = "".join("\n".join(b) + "\n" for b in bloques)
    return base.replace(tras, "\t\t},\n" + insercion + "\t}", 1)


def evaluar_mensajes(antes, despues):
    fallas, _ = verificar_diff(diff_de_contenidos(MENSAJES_TEST, antes, despues), lambda _a: despues)
    fallas += verificar_muestras(antes, despues)
    return {codigo for codigo, _ in fallas}


def diff_forma_v(conteos, extra_por_archivo=None, forma_i_en=None):
    """Diff sintético con `conteos[archivo]` pares de la forma v, cada uno en una línea distinta."""
    extra_por_archivo = extra_por_archivo or {}
    partes = []
    for archivo, n in sorted(conteos.items()):
        quitadas = [f'    sidecar.enviar_saludo(6, "celula-{k}").await;' for k in range(n)]
        agregadas = [q.replace("enviar_saludo(6,", "enviar_saludo(7,") for q in quitadas]
        if forma_i_en == archivo:
            quitadas.append("    assert_eq!(saludo_nucleo.version, 6);")
            agregadas.append("    assert_eq!(saludo_nucleo.version, 7);")
        q_extra, a_extra = extra_por_archivo.get(archivo, ([], []))
        partes.append(diff_sintetico(archivo, quitadas + q_extra, agregadas + a_extra))
    return "".join(partes)


def evaluar_v(diff):
    fallas, pares_v = verificar_diff(diff, lambda _a: None)
    return {c for c, _ in fallas + verificar_conteo_v(pares_v)}


def autoprueba():
    cierre = W + "cierre_de_sesion.rs"
    reconexion = W + "reconexion.rs"
    protocolo = W + "protocolo.rs"
    ipc_rs = "crates/hexcell/tests/respaldo_sqlstore_ipc.rs"
    json_escapado_6 = r'        "{{\"version\":6,\"tipo\":\"acuse_respaldo_sqlstore\",\"resultado\":\"completado\"}}",'
    json_escapado_7 = json_escapado_6.replace(r'\"version\":6', r'\"version\":7')
    cabecera_6 = '\tif !strings.Contains(documento, "**Versión de este protocolo:** 1.5, fijada el 2026-09-11.") {'
    cabecera_7 = cabecera_6.replace("1.5, fijada el 2026-09-11.", "1.6, fijada el 2026-09-30.")
    linea_iii = "        restablecer_contacto: hexcell::admin::restablecimiento_no_disponible(),"
    casos = [
        ("A01 diff vacío", "", set()),
        ("A02 forma i en aserción", diff_sintetico(cierre, ["    assert_eq!(orden.version, 6);"], ["    assert_eq!(orden.version, 7);"]), set()),
        ("A03 forma i en JSON escapado", diff_sintetico(ipc_rs, [json_escapado_6], [json_escapado_7]), set()),
        ("A04 forma ii en documento_test", diff_sintetico(DOCUMENTO_TEST, [cabecera_6], [cabecera_7]), set()),
        ("A05 forma iii en admin_http", diff_sintetico(ADMIN_HTTP, [], [linea_iii]), set()),
        ("A06 valor esperado cambiado en la misma línea de la versión",
         diff_sintetico(ipc_rs, [json_escapado_6], [json_escapado_7.replace("completado", "fallido")]),
         {"AGREGADA-FUERA-DE-FORMA", "REEMPLAZO-AUSENTE"}),
        ("A07 aserción de versión borrada", diff_sintetico(cierre, ["    assert_eq!(orden.version, 6);"], []), {"REEMPLAZO-AUSENTE"}),
        ("A08 valor esperado cambiado sin versión",
         diff_sintetico(cierre, ["    assert_eq!(acuse.resultado, \"completado\");"], ["    assert_eq!(acuse.resultado, \"fallido\");"]),
         {"QUITADA-FUERA-DE-FORMA", "AGREGADA-FUERA-DE-FORMA"}),
        ("A09 forma iii fuera de admin_http", diff_sintetico(cierre, [], [linea_iii]), {"AGREGADA-FUERA-DE-FORMA"}),
        ("A10 forma ii fuera de documento_test",
         diff_sintetico("crates/hexcell/tests/respaldo_cli.rs", [cabecera_6], [cabecera_7]),
         {"QUITADA-FUERA-DE-FORMA", "AGREGADA-FUERA-DE-FORMA"}),
        ("A11 aserción nueva en archivo preexistente", diff_sintetico(cierre, [], ["    assert!(true);"]), {"AGREGADA-FUERA-DE-FORMA"}),
        ("A12 versión 6 -> 8", diff_sintetico(cierre, ["    assert_eq!(orden.version, 6);"], ["    assert_eq!(orden.version, 8);"]),
         {"AGREGADA-FUERA-DE-FORMA", "REEMPLAZO-AUSENTE"}),
        ("A13 nombre de prueba renombrado",
         diff_sintetico(cierre, ["async fn cierre_de_sesion_ok() {"], ["async fn cierre_de_sesion_bien() {"]),
         {"QUITADA-FUERA-DE-FORMA", "AGREGADA-FUERA-DE-FORMA"}),
    ]
    b = BASE_MENSAJES
    valido = con_entradas(b, ORDEN_IV, ACUSE_IV)
    casos_iv = [
        ("V01 forma iv válida: dos entradas nuevas completas", b, valido, set()),
        ("V02 forma iv junto a forma i en el mismo archivo", b, valido.replace('{"version":6,', '{"version":7,'), set()),
        ("V03 entrada existente tocada", b, valido.replace('IdCelula: "piloto-01"', 'IdCelula: "piloto-02"'),
         {"QUITADA-FUERA-DE-FORMA", "MUESTRA-EXISTENTE-ALTERADA"}),
        ("V04 una de las dos entradas nuevas quitada (falta el acuse)", b, con_entradas(b, ORDEN_IV), {"MUESTRAS-NUEVAS-INCORRECTAS"}),
        ("V05 una de las dos entradas nuevas quitada (falta la orden)", b, con_entradas(b, ACUSE_IV), {"MUESTRAS-NUEVAS-INCORRECTAS"}),
        ("V06 cuerpos vacíos", b,
         con_entradas(b, ["\t\tipc.TipoOrdenRestablecerContacto: ipc.OrdenRestablecerContacto{},"],
                      ["\t\tipc.TipoAcuseRestablecerContacto: ipc.AcuseRestablecerContacto{},"]),
         {"MUESTRA-SIN-CAMPOS"}),
        ("V07 contadores en cero y existe vacío", b,
         con_entradas(b, ORDEN_IV, [l.replace(": 1,", ": 0,").replace("Existe:                     ipc.ValorSi", 'Existe:                     ""') for l in ACUSE_IV]),
         {"MUESTRA-SIN-CAMPOS"}),
        ("V08 tercera entrada nueva", b, con_entradas(b, ORDEN_IV, ACUSE_IV, ["\t\tipc.TipoSaludo: ipc.Saludo{},"]),
         {"MUESTRAS-NUEVAS-INCORRECTAS"}),
        ("V09 línea añadida fuera de cuerposDeMuestra", b,
         valido.replace("func TestUnaLineaCruda", "var extra = 1\n\nfunc TestUnaLineaCruda"), {"AGREGADA-FUERA-DE-FORMA"}),
        ("V10 entrada existente borrada", b,
         valido.replace("\t\tipc.TipoSaludo: ipc.Saludo{\n\t\t\tEmisor:   ipc.EmisorSidecar,\n\t\t\tIdCelula: \"piloto-01\",\n\t\t},\n", ""),
         {"QUITADA-FUERA-DE-FORMA", "MUESTRA-EXISTENTE-ALTERADA"}),
    ]
    completo = dict(CONTEOS_FORMA_V)
    uno_menos = dict(CONTEOS_FORMA_V, **{reconexion: CONTEOS_FORMA_V[reconexion] - 1})
    uno_mas = dict(CONTEOS_FORMA_V, **{reconexion: CONTEOS_FORMA_V[reconexion] + 1})
    valido_v = diff_forma_v(completo, forma_i_en=protocolo)
    casos_v = [
        ("F01 forma v completa (45 pares) con forma i en protocolo.rs, sin contar doble", valido_v, set()),
        ("F02 un sitio dejado en 6 (44 pares)", diff_forma_v(uno_menos, forma_i_en=protocolo), {"FORMA-V-CONTEO"}),
        ("F03 otro cambio en un archivo de la forma v",
         diff_forma_v(completo, {reconexion: (["    assert!(conectado);"], ["    assert!(!conectado);"])}, forma_i_en=protocolo),
         {"QUITADA-FUERA-DE-FORMA", "AGREGADA-FUERA-DE-FORMA"}),
        ("F04 enviar_saludo(6 -> 8",
         diff_forma_v(uno_menos, {reconexion: (['    sidecar.enviar_saludo(6, "celula-x").await;'], ['    sidecar.enviar_saludo(8, "celula-x").await;'])}, forma_i_en=protocolo),
         {"AGREGADA-FUERA-DE-FORMA", "REEMPLAZO-AUSENTE", "FORMA-V-CONTEO"}),
        ("F05 forma v en un archivo fuera de los nueve",
         valido_v + diff_sintetico(W + "comun/mod.rs", ['    sidecar.enviar_saludo(6, "x").await;'], ['    sidecar.enviar_saludo(7, "x").await;']),
         {"QUITADA-FUERA-DE-FORMA", "AGREGADA-FUERA-DE-FORMA"}),
        ("F06 un par de más (46)", diff_forma_v(uno_mas, forma_i_en=protocolo), {"FORMA-V-CONTEO"}),
    ]
    mecanico = ("a" * 40, "test: subir fixtures al cable 7 (HEX-091-a)", [cierre, reconexion, MENSAJES_TEST])
    feat = ("b" * 40, "feat: restablecimiento de contacto", ["crates/hexcell/src/admin.rs"])
    casos_commits = [
        ("C01 ningún commit toca la lista (la base)", [feat], set()),
        ("C02 un único commit test: solo con archivos de la lista", [feat, mecanico], set()),
        ("C03 ediciones mecánicas repartidas en dos commits", [mecanico, ("c" * 40, "test: resto", [reconexion])],
         {"MECANICO-EN-VARIOS-COMMITS"}),
        ("C04 ediciones mecánicas en un commit feat:", [("d" * 40, "feat: todo junto", [cierre])], {"MECANICO-SIN-COMMIT-TEST"}),
        ("C05 commit test: mezclado con código de producción",
         [("e" * 40, "test: fixtures", [cierre, "crates/hexcell/src/admin.rs"])], {"MECANICO-MEZCLADO"}),
    ]
    preexistentes = set(ARCHIVOS) | {"crates/hexcell/tests/motor.rs"}
    casos_de_nombres = [
        ("N01 archivo de la lista modificado", [(cierre, "M")], set()),
        ("N02 prueba nueva añadida", [("crates/hexcell/tests/restablecimiento_de_contacto_http.rs", "A")], set()),
        ("N03 otra prueba preexistente modificada", [("crates/hexcell/tests/motor.rs", "M")], {"ARCHIVO-DE-PRUEBA-AJENO"}),
        ("N04 archivo de la lista borrado", [(cierre, "D")], {"ARCHIVO-DE-PRUEBA-AJENO"}),
    ]
    errores, total = 0, 0

    def informar(nombre, obtenidos, esperados, entrada=None, referencia=None):
        nonlocal errores, total
        total += 1
        if esperados and entrada is not None and entrada == referencia:
            errores += 1
            print(f"MAL {nombre}: la mutación no cambió la entrada respecto del caso válido")
            return
        estado = "ok" if obtenidos == esperados else "MAL"
        errores += estado == "MAL"
        print(f"{estado} {nombre}: esperados={sorted(esperados)} obtenidos={sorted(obtenidos)}")

    for nombre, diff, esperados in casos:
        informar(nombre, {c for c, _ in verificar_diff(diff, lambda _a: None)[0]}, esperados, diff, "")
    for nombre, antes, despues, esperados in casos_iv:
        informar(nombre, evaluar_mensajes(antes, despues), esperados, despues, valido)
    for nombre, diff, esperados in casos_v:
        informar(nombre, evaluar_v(diff), esperados, diff, valido_v)
    for nombre, commits, esperados in casos_commits:
        informar(nombre, {c for c, _ in verificar_commits(commits)}, esperados, commits, [feat, mecanico])
    for nombre, pares, esperados in casos_de_nombres:
        informar(nombre, {c for c, _ in verificar_nombres(pares, preexistentes)}, esperados, pares, [(cierre, "M")])
    if len(ARCHIVOS) != 18 or sum(CONTEOS_FORMA_V.values()) != TOTAL_FORMA_V:
        errores += 1
        print(f"MAL lista: {len(ARCHIVOS)} archivos (se esperaban 18), suma forma v {sum(CONTEOS_FORMA_V.values())}")
    if errores:
        print(f"FALLA[AUTOPRUEBA] {errores} de {total} caso(s) no dieron el código esperado")
        return 1
    print(f"OK: autoprueba completa, {total} casos, cada uno dio exactamente sus códigos esperados")
    return 0


def argumentos():
    resto, repo, auto = sys.argv[1:], None, False
    while resto:
        if resto[0] == "--autoprueba":
            auto, resto = True, resto[1:]
        elif resto[0] == "--repo" and len(resto) > 1:
            repo, resto = resto[1], resto[2:]
        else:
            print(f"FALLA[USO] argumento desconocido: {resto[0]}")
            sys.exit(2)
    return repo, auto


REPO, AUTO = argumentos()
if REPO:
    os.chdir(REPO)  # solo lectura: la guarda nunca escribe ni compromete
sys.exit(autoprueba() if AUTO else principal())
PY
