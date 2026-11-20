# Kalango Remote · resume um nó exportado do FigCraft (get_node) numa lista legível: posição absoluta,
# tamanho, preenchimento, cantos, borda e texto (fonte, tamanho, peso). Só lê.
#   python3 kalangoos/scripts/figcraft-resumo.py ARQUIVO.json [profundidade-máx]
import json, sys

t = open(sys.argv[1]).read()
try:
    d = json.loads(t)
except Exception:
    d = json.loads(json.loads(t)[0]["text"])
maxd = int(sys.argv[2]) if len(sys.argv) > 2 else 99
ox = d.get("absoluteBounds", {}).get("x", 0)
oy = d.get("absoluteBounds", {}).get("y", 0)

def cor(fills):
    out = []
    for f in fills or []:
        if f.get("visible") is False:
            continue
        if f.get("type", "").lower() == "solid":
            c = f.get("color", "")
            op = f.get("opacity", 1)
            out.append(c + (f"@{op:.2f}" if op not in (1, None) else ""))
        else:
            out.append(f.get("type", "?"))
    return ",".join(out)

def walk(n, depth):
    if depth > maxd or n.get("visible") is False:
        return
    b = n.get("absoluteBounds", {})
    x, y = round(b.get("x", 0) - ox), round(b.get("y", 0) - oy)
    w, h = round(n.get("width", 0)), round(n.get("height", 0))
    s = f"{'  ' * depth}{n.get('type','')[:4]} {n.get('name','')[:40]!r} @{x},{y} {w}x{h}"
    if n.get("fills"):
        s += f" fill={cor(n['fills'])}"
    if n.get("strokes"):
        s += f" stroke={cor(n['strokes'])}/{n.get('strokeWeight','')}"
    if n.get("cornerRadius"):
        s += f" r={n['cornerRadius']}"
    if n.get("type") == "TEXT":
        fam, size, wt = n.get("fontFamily"), n.get("fontSize"), n.get("fontStyle")
        ls = n.get("letterSpacing") if n.get("letterSpacing") not in ("0%", 0, None) else None
        s += f" [{fam} {size} {wt}{' ls='+str(ls) if ls else ''}] {n.get('characters','')[:70]!r}"
    lay = n.get("layout")
    if isinstance(lay, dict) and lay.get("mode") not in (None, "none"):
        s += f" {lay['mode'][:1].upper()}gap={lay.get('gap')} pad={[int(p) for p in lay.get('padding', [])]} {lay.get('mainAlign')}/{lay.get('crossAlign')}"
    print(s)
    for c in n.get("children", []) or []:
        walk(c, depth + 1)

walk(d, 0)
