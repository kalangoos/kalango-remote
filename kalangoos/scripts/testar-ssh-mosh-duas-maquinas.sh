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
set -u
: "${KALANGO_SSH_A:?}" "${KALANGO_SSH_B:?}" "${KALANGO_IP_B:?}"
APP=${KALANGO_APP:-/usr/lib/kalango/remote/kalango-remote}
T=/tmp/kalango-remote-ssh2-$$
falhou() { echo "FALHOU: $*"; exit 1; }
A() { $KALANGO_SSH_A "export LD_LIBRARY_PATH=/usr/lib/kalango/remote HOME=$T/home XDG_CONFIG_HOME=$T/config XDG_CACHE_HOME=$T/cache XDG_RUNTIME_DIR=$T/run KALANGO_REMOTE_COFRE=arquivo; $*"; }
Bm() { $KALANGO_SSH_B "$*"; }
A "mkdir -p $T/home/.ssh $T/config $T/run; chmod 700 $T/home/.ssh $T/run; ssh-keygen -q -t ed25519 -N '' -C teste@kalango -f $T/home/.ssh/teste_ed25519"
PUB=$(A "cat $T/home/.ssh/teste_ed25519.pub")
Bm "mkdir -p ~/.ssh; chmod 700 ~/.ssh; echo '$PUB' >> ~/.ssh/authorized_keys; systemctl is-active ssh > /dev/null || sudo systemctl start ssh"
trap 'Bm "sed -i \"/teste@kalango/d\" ~/.ssh/authorized_keys; sudo rm -f /etc/ssh/ssh_host_ed25519_key.bak" ; A "rm -rf $T"; echo "(pastas do teste apagadas)"' EXIT
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
# o comando do app, dentro de um pty (o mosh exige terminal); a B perde a rede por 8 s no meio da contagem
A "argv=\$($APP ssh-command vm-b-mosh); cmd=\$(python3 -c \"import json,shlex,sys; print(' '.join(shlex.quote(x) for x in json.loads(sys.argv[1])))\" \"\$argv\"); \
   (setsid script -qec \"\$cmd -- sh -c 'for i in \\\$(seq 1 22); do echo CONTA-\\\$i; sleep 1; done; echo FIM-MOSH'\" $T/mosh.log < /dev/null > /dev/null 2>&1 &)"
sleep 7
Bm "sudo ip link set enp0s5 down"; echo "   (rede da B caída)"; sleep 8; Bm "sudo ip link set enp0s5 up; sleep 1; sudo nmcli con up lan > /dev/null 2>&1"; echo "   (rede da B de volta)"
for _ in $(seq 40); do A "grep -aq FIM-MOSH $T/mosh.log" && break; sleep 1; done
A "grep -aoE 'Last (contact|reply)[^.]*|Connection reestablished|FIM-MOSH' $T/mosh.log | sort -u | tr '\n' ';'" | sed 's/^/   o mosh mostrou: /'; echo
A "grep -aq FIM-MOSH $T/mosh.log" || falhou "o Mosh não terminou a contagem depois da rede voltar"
echo "   ok: a sessão Mosh sobreviveu à rede interrompida e terminou"

echo "5. a chave do servidor muda"
Bm "sudo cp /etc/ssh/ssh_host_ed25519_key /etc/ssh/ssh_host_ed25519_key.bak; sudo rm -f /etc/ssh/ssh_host_ed25519_key /etc/ssh/ssh_host_ed25519_key.pub; sudo ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key; sudo systemctl restart ssh"
sleep 2
S=$(A "$APP ssh-scan $VID")
echo "   $S"
[ "${S%% *}" = changed ] || falhou "devia ser changed"
roda "$VID" echo conectado && falhou "conectou com a chave trocada"
roda vm-b-mosh echo conectado && falhou "o Mosh conectou com a chave trocada"
echo "   ok: bloqueado (ssh e mosh)"
echo "OK"
