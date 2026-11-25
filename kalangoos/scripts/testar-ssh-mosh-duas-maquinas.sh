#!/bin/bash
# Kalango Remote · o SSH e o Mosh do app entre DUAS máquinas (kalangoos/scripts/duas-vms.sh): o app roda na A, o
# servidor é o sshd de verdade da B (porta 22, endereço da rede entre as duas). Cada lado com pasta pessoal
# descartável; nada toca o ~/.ssh de verdade. Precisa de KALANGO_SSH_A e KALANGO_SSH_B (comandos ssh que abrem uma
# linha de comando em cada VM), KALANGO_IP_B, e de sshd (openssh-server) e mosh na B e mosh na A.
#   1. importar ~/.ssh/config da A (o servidor da B) para o app
#   2. a chave do servidor é desconhecida → o app recusa (StrictHostKeyChecking=yes) e o ssh-scan diz `unknown`
#   3. confiar (o que o diálogo faz) → conecta com a chave do cofre
#   4. Mosh: o app monta o comando `mosh --ssh=…`; a sessão abre pelo mosh-server da B e SOBREVIVE a uma rede
#      interrompida (a placa da B cai por 8 s no meio) — um ssh comum morreria
#   5. a chave do servidor da B muda → ssh-scan diz `changed` e a conexão é bloqueada (nada de mosh também)
# ATENÇÃO: o teste derruba a placa de rede da B e troca a chave de host do sshd dela. Só roda se a B for uma máquina
# virtual (systemd-detect-virt) e restaura a chave e a placa ao sair.
set -u
: "${KALANGO_SSH_A:?}" "${KALANGO_SSH_B:?}" "${KALANGO_IP_B:?}"
APP=${KALANGO_APP:-/usr/lib/kalango/remote/kalango-remote}
T=$(mktemp -d /tmp/kalango-remote-ssh2-XXXXXX)   # (o mesmo nome nas duas VMs; criado com 0700 em cada uma)
falhou() { echo "FALHOU: $*"; exit 1; }
# recusa rodar fora de uma VM: o teste mexe na rede e na chave de host
for lado in "$KALANGO_SSH_A" "$KALANGO_SSH_B"; do
  v=$($lado "systemd-detect-virt" 2> /dev/null); [ -n "$v" ] && [ "$v" != none ] || { echo "RECUSADO: ${lado%% *} não é uma máquina virtual (systemd-detect-virt: ${v:-nada}); este teste derruba a rede e troca a chave do sshd"; exit 2; }
done
A() { $KALANGO_SSH_A "export LD_LIBRARY_PATH=/usr/lib/kalango/remote HOME=$T/home XDG_CONFIG_HOME=$T/config XDG_CACHE_HOME=$T/cache XDG_RUNTIME_DIR=$T/run KALANGO_REMOTE_COFRE=arquivo; $*"; }
Bm() { $KALANGO_SSH_B "$*"; }
Bm "mkdir -m 700 -p $T"; A "mkdir -p $T/home/.ssh $T/config $T/run; chmod 700 $T/home/.ssh $T/run; ssh-keygen -q -t ed25519 -N '' -C teste@kalango -f $T/home/.ssh/teste_ed25519"
PUB=$(A "cat $T/home/.ssh/teste_ed25519.pub")
Bm "mkdir -p ~/.ssh; chmod 700 ~/.ssh; echo '$PUB' >> ~/.ssh/authorized_keys; systemctl is-active ssh > /dev/null || sudo systemctl start ssh"
FP0=$(Bm "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub | cut -d' ' -f2")   # (a impressão original da chave de host da B)
# ao sair: a chave de host original volta do .bak, a placa volta, a chave de teste sai do authorized_keys
limpar() {
  Bm "sudo ip link set enp0s5 up 2> /dev/null; if [ -f /etc/ssh/ssh_host_ed25519_key.bak ]; then sudo cp -p /etc/ssh/ssh_host_ed25519_key.bak /etc/ssh/ssh_host_ed25519_key; sudo cp -p /etc/ssh/ssh_host_ed25519_key.pub.bak /etc/ssh/ssh_host_ed25519_key.pub; sudo rm -f /etc/ssh/ssh_host_ed25519_key.bak /etc/ssh/ssh_host_ed25519_key.pub.bak; sudo systemctl restart ssh; fi; sed -i '/teste@kalango/d' ~/.ssh/authorized_keys; rm -rf $T"
  A "rm -rf $T"
  local fp1; fp1=$(Bm "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub | cut -d' ' -f2")
  if [ "$FP0" = "$fp1" ]; then echo "(chave de host da B restaurada: $fp1, igual à original; rede da B no ar; pastas do teste apagadas)"
  else echo "ERRO: a chave de host da B NÃO voltou ao que era ($FP0 -> $fp1)"; fi
}
trap limpar EXIT
A "cat > $T/home/.ssh/config <<EOF
Host vm-b
    HostName $KALANGO_IP_B
    User kalango
    IdentityFile ~/.ssh/teste_ed25519
EOF"
roda() { # roda o comando que o Terminal rodaria (ssh-command), com um comando remoto no fim
  A "argv=\$($APP ssh-command $1) || exit 9; python3 -c \"import json,sys,subprocess; a=json.loads(sys.argv[1])+sys.argv[2:]; sys.exit(subprocess.run(a,stdin=subprocess.DEVNULL,capture_output=True,timeout=40).returncode)\" \"\$argv\" ${*:2}"
}

echo "1. importar"
A "$APP ssh-import" || falhou "importar"
VID=$(A "python3 -c \"import json; print([h['id'] for h in json.load(open('$T/config/kalango/remote/hosts.json'))['hosts'] if h['name']=='vm-b'][0])\"") || falhou "servidor não importado"
echo "   ok: vm-b importado ($VID)"
# a variante Mosh do mesmo servidor
A "python3 - $T/config/kalango/remote/hosts.json <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
h = dict([h for h in d['hosts'] if h['name'] == 'vm-b'][0]); h.update(id='vm-b-mosh', name='vm-b-mosh', type='mosh')
d['hosts'].append(h); json.dump(d, open(p, 'w'))
PY"

echo "2. chave desconhecida"
S=$(A "$APP ssh-scan $VID") || falhou "ssh-scan"
echo "   $S"
[ "${S%% *}" = unknown ] || falhou "devia ser unknown"
roda "$VID" echo conectado && falhou "conectou sem confiar na chave"
echo "   ok: recusou sem confiar"

echo "3. confiar e conectar"
A "ssh-keyscan $KALANGO_IP_B 2>/dev/null >> $T/config/kalango/remote/known_hosts"
[ "$(A "$APP ssh-scan $VID" | cut -d' ' -f1)" = known ] || falhou "devia ser known"
roda "$VID" echo conectado || falhou "não conectou"
echo "   ok: conectou pelo sshd da B com a chave do cofre"

echo "4. Mosh, com a rede interrompida"
A "$APP ssh-command vm-b-mosh" | head -c 200; echo
A "which mosh mosh-client" | sed 's/^/   /'
Bm "which mosh-server" | sed 's/^/   B: /'
# (o marcador FIM-MOSH-42 é calculado pela B: o cabeçalho do log do `script` repete o comando, mas só a saída tem o "42")
# o comando do app, dentro de um pty (o mosh exige terminal); `script` grava o que a tela da A recebeu COM o tempo de
# cada pedaço; a B conta de 1 a 22 e perde a rede por 8 s no meio. A prova: o intervalo entre o último número que a A
# viu antes da queda e o primeiro depois dela é de pelo menos 7,5 s (o servidor continuou contando; o cliente não
# recebeu nada), e o FIM chega depois que a rede volta.
# (os scripts vão para a A por stdin, sem passar por camadas de aspas)
A "cat > $T/mosh-run.sh" <<'EOS'
#!/bin/bash
argv=$("$APP" ssh-command vm-b-mosh)
cmd=$(python3 -c 'import json,shlex,sys; print(" ".join(shlex.quote(x) for x in json.loads(sys.argv[1])))' "$argv")
export TERM=${TERM:-xterm}   # (sem terminal o mosh morre: "TERM environment variable not set")
exec script -q -m advanced -O "$T/mosh.log" -T "$T/mosh.tim" -c "stty rows 30 cols 100; $cmd -- sh -c 'for i in \$(seq 1 22); do echo CONTA-\$i; sleep 1; done; echo FIM-MOSH-\$((40+2))'"
EOS
A "cat > $T/mosh-gaps.py" <<'EOS'
import re, sys
data = open(sys.argv[1], 'rb').read()
chunks, t = [], 0.0
for l in open(sys.argv[2]):
    p = l.split()
    if len(p) == 3 and p[0] == 'O':
        t += float(p[1]); chunks.append((t, int(p[2])))
# (o log começa com uma linha de cabeçalho do `script` que o arquivo de tempos não conta)
seen, pos, buf = {}, data.index(b'\n') + 1, ''
for t, n in chunks:
    buf += data[pos:pos + n].decode(errors='replace'); pos += n
    # (sem os códigos de controle do terminal; um número ainda incompleto, como o "1" de "12", já foi visto antes)
    for m in re.finditer(r'CONTA-(\d+)', re.sub(r'\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][0-9A-Za-z]|\r', '', buf)):
        seen.setdefault(int(m.group(1)), t)
nums = sorted(seen)
gaps = [(seen[b] - seen[a], a, b) for a, b in zip(nums, nums[1:])]
if not gaps:
    print('   a A não viu nenhum número'); sys.exit(1)
g = max(gaps)
print('   instantes em que a A viu cada número (s): ' + ' '.join('%d@%.1f' % (n, seen[n]) for n in nums))
print('   números vistos pela A: %d a %d; maior intervalo entre dois: %.1f s (entre %d e %d)' % (nums[0], nums[-1], *g))
sys.exit(0 if g[0] >= 7.5 else 1)
EOS
A "APP=$APP T=$T setsid bash $T/mosh-run.sh < /dev/null > /dev/null 2>&1 &"
sleep 7
Bm "sudo ip link set enp0s5 down"; echo "   (rede da B caída)"; sleep 8; Bm "sudo ip link set enp0s5 up; sleep 1; sudo nmcli con up lan > /dev/null 2>&1"; echo "   (rede da B de volta)"
for _ in $(seq 40); do A "grep -aq FIM-MOSH-42 $T/mosh.log 2> /dev/null" && break; sleep 1; done
A "grep -aq FIM-MOSH-42 $T/mosh.log" || { A "tail -c 700 $T/mosh.log | tr -d \"\\033\" | tail -12"; falhou "o Mosh não terminou a contagem depois da rede voltar"; }
A "python3 $T/mosh-gaps.py $T/mosh.log $T/mosh.tim" || { A "grep -c . $T/mosh.tim; awk '\$1==\"O\"' $T/mosh.tim | head -40 | tr '\n' ';'"; falhou "não houve um intervalo de pelo menos 7,5 s sem a A receber nada (a queda não aconteceu ou o Mosh não a mostrou)"; }
echo "   ok: a sessão Mosh ficou sem receber nada por >= 7,5 s, voltou sozinha e terminou a contagem"

echo "5. a chave do servidor muda"
Bm "sudo cp -p /etc/ssh/ssh_host_ed25519_key /etc/ssh/ssh_host_ed25519_key.bak; sudo cp -p /etc/ssh/ssh_host_ed25519_key.pub /etc/ssh/ssh_host_ed25519_key.pub.bak; sudo rm -f /etc/ssh/ssh_host_ed25519_key /etc/ssh/ssh_host_ed25519_key.pub; sudo ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key; sudo systemctl restart ssh"
sleep 2
S=$(A "$APP ssh-scan $VID")
echo "   $S"
[ "${S%% *}" = changed ] || falhou "devia ser changed"
roda "$VID" echo conectado && falhou "conectou com a chave trocada"
roda vm-b-mosh echo conectado && falhou "o Mosh conectou com a chave trocada"
echo "   ok: bloqueado (ssh e mosh)"
echo "OK"
