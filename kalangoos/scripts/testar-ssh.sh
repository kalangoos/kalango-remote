#!/bin/bash
# Kalango Remote · a área SSH contra um sshd de verdade, descartável: roda como o seu usuário, na porta que
# estiver livre em 127.0.0.1, com chave de servidor e pasta pessoal do teste (HOME e XDG apontam para lá;
# segredos num cofre de arquivo do teste, nunca no chaveiro real). Confere:
#   1. importar ~/.ssh/config (Host, HostName, User, Port, IdentityFile, ProxyJump) e a chave para o cofre
#   2. a chave do servidor desconhecida → ssh-command recusa (StrictHostKeyChecking=yes)
#   3. confiar (o que o diálogo faz) → conecta com a chave do cofre, pelo salto (ProxyJump) também
#   4. um encaminhamento local que abre junto com o servidor
#   5. a chave do servidor muda → ssh-scan diz "changed" e a conexão é bloqueada
set -u
cd "$(dirname "$0")/../.."
source kalangoos/scripts/env.sh
B=$PWD/target/debug/kalango-remote
T=$(mktemp -d /tmp/kalango-remote-ssh-XXXX)
mkdir -p "$T/home/.ssh" "$T/config" "$T/srv"
chmod 700 "$T/home/.ssh"
export HOME="$T/home" XDG_CONFIG_HOME="$T/config" XDG_CACHE_HOME="$T/cache" XDG_RUNTIME_DIR="$T/run" KALANGO_REMOTE_COFRE=arquivo
mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1])")
EU=$(id -un)
ssh-keygen -q -t ed25519 -N "" -f "$T/srv/host_ed25519"
ssh-keygen -q -t ed25519 -N "" -C "teste@kalango" -f "$T/home/.ssh/teste_ed25519"
cp "$T/home/.ssh/teste_ed25519.pub" "$T/srv/authorized_keys"
cat > "$T/srv/sshd_config" <<EOF
Port $PORT
ListenAddress 127.0.0.1
HostKey $T/srv/host_ed25519
PidFile $T/srv/sshd.pid
AuthorizedKeysFile $T/srv/authorized_keys
UsePAM no
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
AllowTcpForwarding yes
EOF
inicia_sshd() { /usr/sbin/sshd -D -f "$T/srv/sshd_config" -E "$T/srv/sshd.log" & SP=$!; sleep 0.7; }
inicia_sshd
# um "servidor de destino" para o encaminhamento: ecoa uma linha
python3 -c "
import socket,threading
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(('127.0.0.1',0)); s.listen()
open('$T/eco.port','w').write(str(s.getsockname()[1]))
while True:
    c,_=s.accept(); c.sendall(b'eco-do-servidor\n'); c.close()
" & EP=$!
trap 'kill $SP $EP 2>/dev/null; echo "registros em $T"' EXIT
sleep 0.3
ECO=$(cat "$T/eco.port")
FWD=$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1])")
cat > "$HOME/.ssh/config" <<EOF
Host *
    ServerAliveInterval 60
Host bastiao-teste
    HostName 127.0.0.1
    Port $PORT
    User $EU
    IdentityFile ~/.ssh/teste_ed25519
Host app-teste
    HostName localhost
    Port $PORT
    User $EU
    IdentityFile ~/.ssh/teste_ed25519
    ProxyJump bastiao-teste
EOF
falhou() { echo "FALHOU: $*"; exit 1; }
roda() { # roda o comando que o Terminal rodaria, com um comando remoto no fim
  local argv; argv=$("$B" ssh-command "$1") || return 9
  python3 -c "import json,sys,subprocess; a=json.loads(sys.argv[1])+sys.argv[2:]; sys.exit(subprocess.run(a,stdin=subprocess.DEVNULL,capture_output=True,timeout=20).returncode)" "$argv" "${@:2}"
}

echo "1. importar"
"$B" ssh-import || falhou "importar"
python3 - "$XDG_CONFIG_HOME/kalango/remote/hosts.json" <<'PY' || exit 1
import json, sys
d = json.load(open(sys.argv[1]))
hs = {h["name"]: h for h in d["hosts"]}
assert set(hs) == {"bastiao-teste", "app-teste"}, hs.keys()
assert hs["app-teste"]["jump"] == hs["bastiao-teste"]["id"], "ProxyJump"
assert hs["app-teste"]["identity"], "IdentityFile → identidade"
assert d["identities"][0]["fingerprint"].startswith("SHA256:")
print("   ok:", len(hs), "servidores; identidade", d["identities"][0]["id"], d["identities"][0]["fingerprint"])
PY
ls "$XDG_CONFIG_HOME/kalango/remote/cofre-de-teste/" | grep -q ssh-key || falhou "a chave privada não foi para o cofre"
BID=$(python3 -c "import json; print([h['id'] for h in json.load(open('$XDG_CONFIG_HOME/kalango/remote/hosts.json'))['hosts'] if h['name']=='bastiao-teste'][0])")
AID=$(python3 -c "import json; print([h['id'] for h in json.load(open('$XDG_CONFIG_HOME/kalango/remote/hosts.json'))['hosts'] if h['name']=='app-teste'][0])")

echo "2. chave desconhecida"
S=$("$B" ssh-scan "$BID") || falhou "ssh-scan"
echo "   $S"
[ "${S%% *}" = unknown ] || falhou "devia ser unknown"
roda "$BID" echo conectado && falhou "conectou sem confiar na chave"
echo "   ok: recusou sem confiar"

echo "3. confiar e conectar (direto e pelo salto)"
# o que "Confiar e conectar" grava: a chave que o servidor mostrou, no known_hosts do app
ssh-keyscan -p "$PORT" 127.0.0.1 2>/dev/null | sed "s/^127.0.0.1/[127.0.0.1]:$PORT/" >> "$XDG_CONFIG_HOME/kalango/remote/known_hosts"
ssh-keyscan -p "$PORT" localhost 2>/dev/null | sed "s/^localhost/[localhost]:$PORT/" >> "$XDG_CONFIG_HOME/kalango/remote/known_hosts"
[ "$("$B" ssh-scan "$BID" | cut -d' ' -f1)" = known ] || falhou "devia ser known"
roda "$BID" echo conectado || falhou "não conectou direto"
roda "$AID" echo conectado || falhou "não conectou pelo salto"
echo "   ok: direto e via ProxyJump, com a chave do cofre"
ls "$XDG_RUNTIME_DIR/kalango-remote/keys/" | head -1 | sed 's/^/   chave materializada em tmpfs (0600): /'

echo "4. encaminhamento local junto com o servidor"
python3 - "$XDG_CONFIG_HOME/kalango/remote/hosts.json" "$AID" "$FWD" "$ECO" <<'PY'
import json, sys
p, aid, fwd, eco = sys.argv[1:]
d = json.load(open(p))
d["forwards"] = [{"id": "eco", "name": "Eco", "kind": "local", "listen": f"127.0.0.1:{fwd}", "target": f"127.0.0.1:{eco}", "host": aid, "with_host": True, "loopback_only": True}]
json.dump(d, open(p, "w"))
PY
( roda "$AID" sleep 4 & )
sleep 2.5
R=$(python3 -c "import socket; s=socket.create_connection(('127.0.0.1',$FWD),3); print(s.recv(100).decode().strip())" 2>&1)
[ "$R" = eco-do-servidor ] || falhou "encaminhamento: $R"
echo "   ok: 127.0.0.1:$FWD → (salto → servidor) → 127.0.0.1:$ECO respondeu \"$R\""
sleep 2

echo "5. a chave do servidor muda"
kill $SP; wait $SP 2>/dev/null
rm -f "$T/srv/host_ed25519" "$T/srv/host_ed25519.pub"
ssh-keygen -q -t ed25519 -N "" -f "$T/srv/host_ed25519"
inicia_sshd
S=$("$B" ssh-scan "$BID")
echo "   $S"
[ "${S%% *}" = changed ] || falhou "devia ser changed"
roda "$BID" echo conectado && falhou "conectou com a chave trocada"
echo "   ok: bloqueado"
# nada fora da pasta do teste
echo "OK"
