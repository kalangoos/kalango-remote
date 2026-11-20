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
# servidores SSH inventados (endereços de documentação), chaves descartáveis do perfil
import subprocess, os, base64, hashlib
kd = f"{d}/chaves"; os.makedirs(kd, exist_ok=True)
def key(name, t="ed25519"):
    p = f"{kd}/{name}"
    if not os.path.exists(p):
        subprocess.run(["ssh-keygen", "-q", "-t", t, "-N", "", "-C", f"{name}@kalango", "-f", p] + (["-b", "2048"] if t == "rsa" else []), check=True)
    pub = open(p + ".pub").read().strip()
    blob = pub.split()[1]
    fp = "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(blob)).digest()).decode().rstrip("=")
    return pub, fp
ids = []
for name, origin, t in [("deploy-ed25519", "generated here", "ed25519"), ("casa-ed25519", "imported from ~/.ssh", "ed25519"), ("legado-rsa", "imported", "rsa")]:
    pub, fp = key(name, t)
    ids.append({"id": name, "kind": "vault-key", "public_key": pub, "fingerprint": fp, "key_type": "RSA" if t == "rsa" else "ED25519", "origin": origin, "confirm_each_use": True, "created": now})
ids.append({"id": "pi-raspberry", "kind": "vault-password", "public_key": "", "fingerprint": "", "key_type": "", "origin": "", "confirm_each_use": True, "created": now})
hosts = {"version": 1,
 "groups": [{"id": "casa", "name": "Casa", "parent": None, "defaults": {"user": "home", "identity": "casa-ed25519"}},
            {"id": "trabalho", "name": "Trabalho", "parent": None, "defaults": {"user": "deploy", "identity": "deploy-ed25519", "jump": "bastiao-trabalho", "keepalive": 30}},
            {"id": "producao", "name": "produção", "parent": "trabalho", "defaults": {}},
            {"id": "staging", "name": "staging", "parent": "trabalho", "defaults": {}},
            {"id": "clientes", "name": "Clientes", "parent": None, "defaults": {"user": "admin"}}],
 "hosts": [
  {"id": "servidor-casa", "name": "servidor-casa", "address": "192.0.2.10", "port": 22, "group": "casa", "tags": ["casa"], "type": "ssh", "last_used": now - 86400},
  {"id": "raspberry-sala", "name": "raspberry-sala", "address": "raspberry.example", "port": 22, "group": "casa", "tags": ["casa", "iot"], "type": "mosh", "user": "pi", "identity": "pi-raspberry", "last_used": now - 21 * 86400},
  {"id": "app-prod-01", "name": "app-prod-01", "address": "203.0.113.24", "port": 22, "group": "producao", "tags": ["trabalho", "produção"], "type": "ssh", "confirm": True, "last_used": now - 60},
  {"id": "app-staging", "name": "app-staging", "address": "10.0.2.15", "port": 22, "group": "staging", "tags": ["trabalho", "staging"], "type": "ssh", "last_used": now - 3 * 3600},
  {"id": "bastiao-trabalho", "name": "bastiao-trabalho", "address": "bastiao.example.com", "port": 2222, "group": "trabalho", "tags": ["trabalho"], "type": "ssh", "user": "ops", "last_used": now - 3 * 3600 - 60},
  {"id": "loja-centro-nas", "name": "loja-centro-nas", "address": "198.51.100.7", "port": 22, "group": "clientes", "tags": ["clientes"], "type": "ssh", "identity": "legado-rsa", "last_used": now - 38 * 86400}],
 "identities": ids,
 "forwards": [
  {"id": "banco-de-staging", "name": "Banco de staging", "kind": "local", "listen": "localhost:5433", "target": "db-staging:5432", "host": "app-staging", "with_host": True, "loopback_only": True},
  {"id": "painel-grafana", "name": "Painel do Grafana", "kind": "local", "listen": "localhost:3000", "target": "localhost:3000", "host": "app-prod-01", "with_host": True, "loopback_only": True},
  {"id": "demo-cliente", "name": "Demo para o cliente", "kind": "remote", "listen": "8080", "target": "localhost:5173", "host": "app-prod-01", "with_host": False, "loopback_only": True},
  {"id": "proxy-trabalho", "name": "Proxy da rede do trabalho", "kind": "dynamic", "listen": "localhost:1080", "target": "", "host": "bastiao-trabalho", "with_host": False, "loopback_only": True},
  {"id": "nas-loja", "name": "NAS da loja", "kind": "local", "listen": "localhost:5001", "target": "192.168.1.20:5001", "host": "loja-centro-nas", "with_host": False, "loopback_only": True}]}
json.dump(hosts, open(f"{d}/config/kalango/remote/hosts.json", "w"), ensure_ascii=False, indent=1)
hk, _ = key("host-app-prod"); hk2, _ = key("host-loja", "rsa")
open(f"{d}/config/kalango/remote/known_hosts", "w").write(f"203.0.113.24 {' '.join(hk.split()[:2])}\n198.51.100.7 {' '.join(hk2.split()[:2])}\n")
PY
  export XDG_CONFIG_HOME="$D/config" XDG_CACHE_HOME="$D/cache" XDG_DATA_HOME="$D/data" KALANGO_REMOTE_APP_ID=os.kalango.RemoteDemo KALANGO_REMOTE_COFRE=arquivo SSH_AUTH_SOCK= LANG=pt_BR.UTF-8 LANGUAGE=pt_BR
fi
exec "$PWD/target/debug/kalango-remote" "$@"
