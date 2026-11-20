#!/bin/bash
# Kalango Remote · abre o app compilado (target/debug) com as bibliotecas e fontes de kalangoos/.deps.
#   kalangoos/scripts/rodar-app.sh [--demo] [argumentos do app]
#     --demo  perfil descartável com computadores inventados (para mostrar e capturar telas; nada pessoal)
set -u
cd "$(dirname "$0")/../.."
source kalangoos/scripts/env.sh
[ -f kalangoos/.deps/fonts.conf ] && export FONTCONFIG_FILE="$PWD/kalangoos/.deps/fonts.conf"
export KALANGO_CLOUDFLARED="${KALANGO_CLOUDFLARED:-$PWD/kalangoos/.deps/bin/cloudflared}"
if [ "${1:-}" = --demo ]; then
  shift
  D=${KALANGO_REMOTE_DEMO:-$PWD/target/perfil-demo}
  mkdir -p "$D/config/kalango/remote" "$D/cache/kalango/remote/thumbs" "$D/data"
  python3 - "$D" <<'PY'
import json, sys, time, struct, zlib
d = sys.argv[1]
def png(path, top, bottom, w=412, h=234):
    ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b) & 0xffffffff)
    rows = []
    for y in range(h):
        k = y / h
        c = bytes(int(top[i] * (1 - k) + bottom[i] * k) for i in range(3))
        rows.append(b'\0' + c * w)
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(b''.join(rows))) + ch(b'IEND', b''))
now = int(time.time())
peers = []
demo = [("Notebook da mãe", True, "cloudflare", 38, 0, (232, 240, 236), (180, 220, 200)),
        ("PC do escritório", True, "lan", 3, 0, (32, 34, 44), (22, 23, 28)),
        ("Servidor de casa", True, "lan", 0, 3600 * 5, (28, 30, 38), (40, 42, 60)),
        ("Mini PC da sala", True, "cloudflare", 0, 86400 + 3600, (20, 21, 26), (16, 17, 21)),
        ("Loja · caixa 2", False, "cloudflare", 0, 86400 + 600, (24, 26, 34), (30, 34, 46)),
        ("Notebook da Bia", False, "cloudflare", 0, 3 * 86400, (200, 204, 210), (150, 160, 170)),
        ("Estúdio · Mac", False, "lan", 0, 4 * 86400, (30, 30, 40), (60, 60, 90)),
        ("Painel da recepção", False, "cloudflare", 0, 9 * 86400, (240, 244, 242), (200, 230, 215))]
for i, (name, fav, route, lat, ago, top, bottom) in enumerate(demo):
    t = f"{d}/cache/kalango/remote/thumbs/demo{i}.png"
    png(t, top, bottom)
    peers.append({"name": name, "target": f"https://demo-{i}.trycloudflare.com/" if route != "lan" else f"192.0.2.{10 + i}:21118",
                  "code": "", "route": route, "last_used": now - ago - 60, "latency_ms": lat, "favorite": fav,
                  "view_only": name == "Estúdio · Mac", "thumb": t})
json.dump({"list": peers}, open(f"{d}/config/kalango/remote/peers.json", "w"), ensure_ascii=False, indent=1)
PY
  export XDG_CONFIG_HOME="$D/config" XDG_CACHE_HOME="$D/cache" XDG_DATA_HOME="$D/data" KALANGO_REMOTE_APP_ID=os.kalango.RemoteDemo LANG=pt_BR.UTF-8 LANGUAGE=pt_BR
fi
exec "$PWD/target/debug/kalango-remote" "$@"
