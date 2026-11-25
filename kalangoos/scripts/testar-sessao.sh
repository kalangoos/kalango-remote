#!/bin/bash
# Kalango Remote · sessão de verdade entre dois processos do núcleo na mesma máquina, cada um com a sua
# pasta pessoal descartável (nada toca a pasta pessoal real):
#   kalangoos/scripts/testar-sessao.sh [direto|tunel|codigo] [DISPLAY]
#     direto  o probe conecta em 127.0.0.1:PORTA com a identidade fixada (padrão)
#     tunel   o servidor abre um túnel rápido do Cloudflare; o probe conecta por WSS, sem cloudflared
#     codigo  o servidor escuta na rede (--lan); o probe acha o computador pelo código (UDP)
#     aprovar o servidor pede aprovação (--ask): a senha certa só faz o pedido chegar; o teste aprova
#             depois de 3 s e confere que nenhum quadro chegou antes
#     niri    o lado compartilhado é um niri aninhado (kalangoos/scripts/niri-aninhado.sh rodando): captura e
#             entrada por wlr; o probe clica numa janela de teste e digita, e o teste confere o que ela recebeu
#     fixa    acesso sem supervisão: o servidor pede aprovação (--ask), mas a senha fixa entra sem ninguém aprovar
#     duasetapas  verificação em duas etapas: sem o código do app autenticador a sessão não abre; com ele, abre
#     gravar  o servidor grava a sessão que recebe numa pasta própria; o teste confere o arquivo de vídeo
#     defesas  setup inválido derruba o núcleo; 5 senhas erradas travam o compartilhamento todo; um código de duas
#             etapas já usado não vale de novo
#     servidor  servidor próprio: sobe o hbbs/hbbr oficiais num docker (rustdesk/rustdesk-server, rede do host), o
#             núcleo compartilha registrando nele e o probe conecta pelo id e pela chave pública do servidor
#     impostor  o convite traz outra chave: a sessão tem de ser recusada (sai 0 se foi)
# O lado compartilhado captura o DISPLAY dado (um Xvfb, padrão :89). Sai 0 se o probe recebeu quadros
# numa sessão cifrada de ponta a ponta. MOVIMENTO=1 anima a tela; QUADROS=N (padrão 30).
# DUAS MÁQUINAS (kalangoos/scripts/duas-vms.sh): com MAQUINAS=1, KALANGO_SSH_A="ssh … a" e KALANGO_SSH_B="ssh … b"
# (e KALANGO_IP_B, o endereço da B na rede entre as duas), o lado compartilhado roda na B e o probe na A, de verdade,
# cada um com o seu Xvfb/pasta pessoal descartável; o servidor escuta na rede (--lan) e o alvo usa o endereço da B.
# MAQUINAS=wayland: o lado compartilhado captura a sessão niri de verdade da B (WAYLAND_DISPLAY=wayland-1, captura e
# entrada por wlr, como no sistema instalado) em vez de um Xvfb.
# Valem os modos direto, tunel, codigo, aprovar, fixa, duasetapas, gravar, defesas e impostor.
set -u
PROBE_EXTRA=()
MODO=${1:-direto}
DISP=${2:-:89}
cd "$(dirname "$0")/../.."
source kalangoos/scripts/env.sh
B=${KALANGO_CORE:-$PWD/target/debug/kalango-remote-core}
T=$(mktemp -d /tmp/kalango-remote-teste-XXXX)
mkdir -p "$T/srv" "$T/cli"
if [ -n "${MAQUINAS:-}" ]; then
  B=${KALANGO_CORE:-/usr/lib/kalango/remote/kalango-remote-core}
  SESS_ENV=
  if [ "$MAQUINAS" = wayland ]; then SESS_ENV="WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/\$(id -u) XDG_CURRENT_DESKTOP=niri NIRI_SOCKET=\$(ls /run/user/\$(id -u)/niri.*.sock | head -1)"; QUADROS=${QUADROS:-3}; fi
  [ "$MAQUINAS" = wayland ] || $KALANGO_SSH_B "pgrep -x Xvfb > /dev/null || (setsid Xvfb $DISP -screen 0 800x600x24 > /dev/null 2>&1 &) ; sleep 2"
  $KALANGO_SSH_B "mkdir -p $T/srv"
  $KALANGO_SSH_A "mkdir -p $T/cli"
fi
NIRI_ENV=()
if [ "$MODO" = niri ]; then
  source target/niri-aninhado/env || { echo "rode kalangoos/scripts/niri-aninhado.sh antes"; exit 2; }
  NIRI_ENV=(WAYLAND_DISPLAY="$WAYLAND_DISPLAY")
  DISP=$DISPLAY
fi
lado() { # pasta pessoal descartável + instância própria
  if [ -n "${MAQUINAS:-}" ]; then   # srv roda na B, o resto (cli) na A
    local ssh_=$KALANGO_SSH_A extra=; if [ "$1" = srv ] || [ "$1" = ruim ]; then ssh_=$KALANGO_SSH_B; extra=$SESS_ENV; fi
    $ssh_ "mkdir -p $T/$1; env LD_LIBRARY_PATH=/usr/lib/kalango/remote HOME=$T/$1 XDG_CONFIG_HOME=$T/$1/config XDG_DATA_HOME=$T/$1/data XDG_CACHE_HOME=$T/$1/cache XDG_SESSION_TYPE=x11 RUSTDESK_FORCED_DISPLAY_SERVER=x11 DISPLAY=$DISP $extra KALANGO_CORE_TEXT=1 KALANGO_REMOTE_INSTANCE=teste$1$$ $(printf '%q ' "${@:2}")"
    return
  fi
  env -u WAYLAND_DISPLAY "${NIRI_ENV[@]}" HOME="$T/$1" XDG_CONFIG_HOME="$T/$1/config" XDG_DATA_HOME="$T/$1/data" XDG_CACHE_HOME="$T/$1/cache" \
    XDG_SESSION_TYPE=x11 DISPLAY="$DISP" KALANGO_CORE_TEXT=1 KALANGO_REMOTE_INSTANCE="teste$1$$" "${@:2}"
}
case "$MODO" in
  direto) ARGS=(--no-tunnel) ;;
  tunel)  ARGS=() ;;
  codigo) ARGS=(--no-tunnel --lan --port 0) ;;
  impostor) ARGS=(--no-tunnel) ;;
  niri) ARGS=(--no-tunnel)
    WAYLAND_DISPLAY="${NIRI_ENV[0]#WAYLAND_DISPLAY=}" GDK_BACKEND=wayland /usr/bin/python3 kalangoos/scripts/janela-teste.py > "$T/janela.log" 2>&1 &
    JP=$!
    sleep 3 ;;
  aprovar) ARGS=(--no-tunnel --ask) ;;
  fixa) ARGS=(--no-tunnel --ask --setup-stdin)
    SETUP='{"cmd":"setup","fixed_password":"senha-fixa-de-teste-1","otp_length":6}'
    MOVIMENTO=1; QUADROS=1 ;;
  duasetapas) ARGS=(--no-tunnel --setup-stdin)
    SEGREDO=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ   # 20 bytes ("12345678901234567890"); o núcleo exige 128 bits ou mais
    SETUP="{\"cmd\":\"setup\",\"otp_length\":6,\"two_factor\":\"$SEGREDO\"}"
    QUADROS=1 ;;
  defesas) ARGS=(--no-tunnel --setup-stdin)
    SEGREDO=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ
    SETUP="{\"cmd\":\"setup\",\"otp_length\":6,\"fixed_password\":\"senha-fixa-de-teste-1\",\"two_factor\":\"$SEGREDO\"}"
    QUADROS=1 ;;
  servidor) ARGS=(--no-tunnel --setup-stdin)
    command -v docker > /dev/null || { echo "precisa do docker (rustdesk/rustdesk-server)"; exit 2; }
    HB=$T/hbbs; mkdir -p "$HB"
    docker run -d --name "kr-teste-hbbr-$$" --network host -v "$HB:/root" rustdesk/rustdesk-server hbbr > /dev/null
    docker run -d --name "kr-teste-hbbs-$$" --network host -v "$HB:/root" rustdesk/rustdesk-server hbbs -r 127.0.0.1:21117 > /dev/null
    for _ in $(seq 20); do [ -s "$HB/id_ed25519.pub" ] && break; sleep 0.5; done
    CHAVE=$(cat "$HB/id_ed25519.pub")
    SETUP="{\"cmd\":\"setup\",\"otp_length\":6,\"server\":{\"id\":\"127.0.0.1\",\"relay\":\"\",\"key\":\"$CHAVE\"}}"
    QUADROS=1 ;;
  gravar) ARGS=(--no-tunnel --setup-stdin)
    SETUP="{\"cmd\":\"setup\",\"otp_length\":6,\"record_dir\":\"$T/srv/Vídeos/Kalango Remote\"}"
    MOVIMENTO=1 ;;
  *) echo "modo desconhecido: $MODO"; exit 2 ;;
esac
# (entre duas máquinas, o servidor escuta na rede em vez de só no loopback; o túnel e o código já são assim)
[ -n "${MAQUINAS:-}" ] && [ "$MODO" != tunel ] && [ "$MODO" != codigo ] && ARGS+=(--lan --port 0)
mkfifo "$T/entrada"
# MOVIMENTO=1: uma bola animada a 30 qps na tela compartilhada (o RustDesk só manda quadros quando algo muda)
if [ "${MOVIMENTO:-0}" = 1 ]; then
  if [ -n "${MAQUINAS:-}" ]; then
    $KALANGO_SSH_B "DISPLAY=$DISP setsid gst-launch-1.0 -q videotestsrc pattern=ball ! video/x-raw,width=800,height=600,framerate=30/1 ! ximagesink > /dev/null 2>&1 < /dev/null &"
  else
    DISPLAY="$DISP" gst-launch-1.0 -q videotestsrc pattern=ball ! video/x-raw,width=800,height=600,framerate=30/1 ! ximagesink > /dev/null 2>&1 &
    GP=$!
  fi
fi
lado srv "$B" serve "${ARGS[@]}" < "$T/entrada" > "$T/serve.out" 2> "$T/serve.err" &
SP=$!
exec 3> "$T/entrada"   # mantém a entrada do servidor aberta
[ -n "${SETUP:-}" ] && echo "$SETUP" >&3   # --setup-stdin: o primeiro pacote é a configuração
trap '[ -n "${MAQUINAS:-}" ] && $KALANGO_SSH_B "pkill -x gst-launch-1.0" 2>/dev/null; exec 3>&-; [ "$MODO" = servidor ] && docker rm -f "kr-teste-hbbs-$$" "kr-teste-hbbr-$$" > /dev/null 2>&1; kill $SP ${GP:-} ${JP:-} 2>/dev/null; wait $SP 2>/dev/null; echo "registros em $T"' EXIT
campo() { python3 -c "import json,sys
for l in open('$T/serve.out'):
    d=json.loads(l)
    if d.get('event')=='$1': print(d['$2']); break"; }
senha_atual() { python3 -c "import json
p=''
for l in open('$T/serve.out'):
    d=json.loads(l)
    if d.get('event') in ('listening','password'): p=d['password']
print(p)"; }
for _ in $(seq 60); do grep -q '"listening"' "$T/serve.out" && break; sleep 0.5; done
PW=$(campo listening password); ID=$(campo listening id); PK=$(campo listening pk); ADDR=$(campo listening addr); CODE=$(campo listening code)
[ -n "${MAQUINAS:-}" ] && ADDR=${KALANGO_IP_B:?}:${ADDR##*:}
case "$MODO" in
  direto) ALVO="https://$ADDR/#k=$ID.$PK" ;;
  codigo) ALVO="$CODE" ;;
  niri) ALVO="https://$ADDR/#k=$ID.$PK"; PROBE_EXTRA=(--click 320,400 --type "Kalango Remote 123") ;;
  aprovar) ALVO="https://$ADDR/#k=$ID.$PK"
    ( for _ in $(seq 60); do grep -q '"event":"connection"' "$T/serve.out" && break; sleep 0.5; done
      sleep 3; CID=$(campo connection id); date +%s.%N > "$T/aprovado-em"
      echo "{\"cmd\":\"authorize\",\"id\":$CID,\"keyboard\":true,\"clipboard\":true,\"audio\":false,\"file\":false}" >&3 ) & ;;
  servidor) ALVO=$ID; PROBE_EXTRA=(--server 127.0.0.1 --key "$CHAVE"); sleep 4 ;;   # espera o registro no servidor
  fixa|duasetapas|gravar|defesas) ALVO="https://$ADDR/#k=$ID.$PK" ;;
  impostor) ALVO="https://$ADDR/#k=$ID.$(python3 -c "import base64,os; print(base64.urlsafe_b64encode(os.urandom(32)).decode().rstrip('='))")" ;;
  tunel)
    for _ in $(seq 60); do grep -q '"tunnel_ready"' "$T/serve.out" && grep -q '"tunnel"' "$T/serve.out" && break; sleep 0.5; done
    ALVO=$(campo tunnel link) ;;   # o núcleo só anuncia o link quando o nome já resolve
esac
echo "alvo: $ALVO  código: $CODE"
if [ "$MODO" = fixa ]; then
  # a senha de uso único, com "perguntar antes" ligado, continua esperando aprovação: nenhum quadro
  lado cli timeout 60 "$B" probe "$ALVO" --password "$PW" --frames 1 --timeout 8 > "$T/uso-unico.out" 2> "$T/uso-unico.err"
  if grep -q '"first_frame"' "$T/uso-unico.out"; then echo "FALHOU: a senha de uso único entrou sem aprovação"; exit 1; fi
  echo "ok: a senha de uso único ficou esperando a aprovação (nenhum quadro)"
  PW=senha-fixa-de-teste-1
fi
codigo_totp() { python3 - "$SEGREDO" <<'PY'
import base64, hmac, hashlib, struct, sys, time
k = base64.b32decode(sys.argv[1]); n = struct.pack(">Q", int(time.time()) // 30)
h = hmac.new(k, n, hashlib.sha1).digest(); o = h[19] & 15
print("%06d" % ((struct.unpack(">I", h[o:o+4])[0] & 0x7fffffff) % 10**6))
PY
}
if [ "$MODO" = duasetapas ]; then
  # sem o código: pede as duas etapas e nenhum quadro chega
  lado cli timeout 60 "$B" probe "$ALVO" --password "$PW" --frames 5 --timeout 8 > "$T/sem-codigo.out" 2> "$T/sem-codigo.err"
  if grep -q '"first_frame"' "$T/sem-codigo.out" || ! grep -q 'input-2fa' "$T/sem-codigo.out"; then echo "FALHOU: sem o código a sessão tinha de pedir as duas etapas e não abrir"; cat "$T/sem-codigo.out"; exit 1; fi
  echo "ok: sem o código pediu as duas etapas e nenhum quadro chegou"
  PROBE_EXTRA=(--2fa "$(codigo_totp)")
fi
if [ "$MODO" = defesas ]; then
  # 1. um setup que não entende ou duas etapas sem segredo válido: o núcleo sai com erro, nunca roda sem proteção
  for ruim in 'isto nao e json' '{"cmd":"setup","two_factor":"123"}' '{"cmd":"setup","server":{"id":"s.example","relay":"","key":""}}'; do
    mkdir -p "$T/ruim"
    echo "$ruim" | lado ruim timeout 20 "$B" serve --no-tunnel --setup-stdin > "$T/ruim.out" 2> "$T/ruim.err"; RR=$?
    if [ $RR -eq 0 ] || ! grep -q '"kind":"setup"' "$T/ruim.out"; then echo "FALHOU: setup ruim aceito: $ruim (saiu $RR)"; tail -3 "$T/ruim.err"; exit 1; fi
  done
  echo "ok: setup inválido faz o núcleo sair com erro"
  tenta() { lado cli timeout 60 "$B" probe "$ALVO" --password "$1" --frames 1 --timeout 8 "${@:2}" > "$T/t.out" 2> "$T/t.err"; }
  # 2. código usado uma vez não vale de novo
  C=$(codigo_totp); tenta senha-fixa-de-teste-1 --2fa "$C"
  grep -q '"first_frame"' "$T/t.out" || { echo "FALHOU: o primeiro código não entrou"; cat "$T/t.out"; exit 1; }
  sleep 1; tenta senha-fixa-de-teste-1 --2fa "$C"
  if grep -q '"first_frame"' "$T/t.out"; then echo "FALHOU: um código de duas etapas já usado entrou de novo"; exit 1; fi
  echo "ok: o código usado não vale de novo"
  # 3. cinco senhas erradas travam tudo: até a certa é recusada
  for i in 1 2 3 4 5; do tenta errada-$i; done
  tenta senha-fixa-de-teste-1 --2fa "$(codigo_totp)"
  if grep -q '"first_frame"' "$T/t.out" || ! grep -q "Too many wrong attempts" "$T/t.out"; then echo "FALHOU: depois de 5 erros a senha certa tinha de ser recusada por bloqueio"; tail -3 "$T/t.out"; exit 1; fi
  grep -q '"event":"lockout"' "$T/serve.out" || { echo "FALHOU: o app não foi avisado do bloqueio"; exit 1; }
  echo "ok: 5 senhas erradas bloqueiam o compartilhamento todo e o app é avisado"
  # 4. um setup que não pode ser aplicado com o compartilhamento aberto derruba o núcleo (o app mostra o motivo)
  echo '{"cmd":"setup","two_factor":"123"}' >&3
  for _ in $(seq 10); do kill -0 $SP 2>/dev/null || break; sleep 0.5; done
  if kill -0 $SP 2>/dev/null || ! grep -q '"kind":"setup"' "$T/serve.out"; then echo "FALHOU: o setup inválido durante o compartilhamento não derrubou o núcleo"; exit 1; fi
  echo "ok: setup inválido com o compartilhamento aberto derruba o núcleo e diz o motivo"
  echo "OK: defesas"; exit 0
fi
INICIO=$(date +%s.%N)
lado cli timeout 120 "$B" probe "$ALVO" --password "$PW" --frames "${QUADROS:-30}" --save "$T/primeiro-quadro.png" "${PROBE_EXTRA[@]}" > "$T/probe.out" 2> "$T/probe.err"
R=$?
grep -v '"quality"' "$T/probe.out"
grep '"quality"' "$T/probe.out" | tail -1
echo "tempo total: $(python3 -c "print(round($(date +%s.%N)-$INICIO,1))") s"
if [ "$MODO" = aprovar ]; then
  grep '"event":"connection"' "$T/serve.out"
  if [ $R -eq 0 ] && grep -q '"secured":true' "$T/probe.out" && [ -f "$T/aprovado-em" ]; then
    # o primeiro quadro chegou depois da aprovação?
    # (entre duas máquinas o relógio do probe começa depois do ssh que o abre: 1 s de folga; sem aprovação os quadros chegariam em ~1 s)
python3 - "$T" "$INICIO" "${MAQUINAS:+1.0}" <<'PY' && exit 0
import json, sys
t = sys.argv[1]; inicio = float(sys.argv[2]); folga = float(sys.argv[3] or 0); aprovado = float(open(t + "/aprovado-em").read())
ff = [json.loads(l) for l in open(t + "/probe.out") if '"first_frame"' in l][0]
quando = inicio + ff["after_ms"] / 1000
print(f"aprovado {aprovado - inicio:.1f} s depois do início; primeiro quadro {quando - inicio:.1f} s")
if quando + folga < aprovado: sys.exit(1)
print("OK: nenhum quadro antes da aprovação; sessão cifrada depois dela")
PY
  fi
  echo "FALHOU: aprovação"; exit 1
fi
if [ "$MODO" = niri ]; then
  sleep 1
  grep -a "CLIQUE\|TEXTO" "$T/janela.log" | tail -2
  if [ $R -eq 0 ] && grep -aq CLIQUE "$T/janela.log" && grep -aq "TEXTO Kalango Remote 123" "$T/janela.log"; then
    echo "OK: tela do niri capturada por wlr; clique e texto chegaram à janela"; exit 0
  fi
  echo "FALHOU: niri"; tail -5 "$T/serve.err"; exit 1
fi
if [ "$MODO" = fixa ]; then
  if [ $R -eq 0 ] && grep -q '"authorized":true' "$T/serve.out" && grep -q '"secured":true' "$T/probe.out"; then
    echo "ok: a senha fixa entrou sem ninguém aprovar (acesso sem supervisão), cifrada de ponta a ponta"
    # a senha fixa não dá arquivos (só a sessão aprovada por clique pode ganhá-los no pedido)
    lado cli timeout 60 "$B" probe "$ALVO" --password senha-fixa-de-teste-1 --files --frames 1 --timeout 8 > "$T/arq-fixa.out" 2> /dev/null
    if grep -q '"event":"connected"' "$T/arq-fixa.out"; then echo "FALHOU: a senha fixa abriu uma sessão de arquivos"; exit 1; fi
    echo "ok: com a senha fixa a sessão de arquivos é recusada"
    ( for _ in $(seq 60); do grep -q '"authorized":false.*"file_transfer":true' "$T/serve.out" && break; sleep 0.5; done
      CID=$(python3 -c "import json
for l in open('$T/serve.out'):
    d=json.loads(l)
    if d.get('event')=='connection' and not d.get('authorized') and d.get('file_transfer'): print(d['id'])" | tail -1)
      echo "{\"cmd\":\"authorize\",\"id\":$CID,\"keyboard\":false,\"clipboard\":false,\"audio\":false,\"file\":true}" >&3 ) &
    lado cli timeout 60 "$B" probe "$ALVO" --password "$(senha_atual)" --files --frames 1 --timeout 12 > "$T/arq-clique.out" 2> /dev/null
    if ! grep -q '"event":"connected"' "$T/arq-clique.out"; then echo "FALHOU: a sessão de arquivos aprovada por clique foi recusada"; cat "$T/arq-clique.out" | tail -3; exit 1; fi
    echo "ok: a sessão de arquivos aprovada por clique abre"
    # trocar a senha fixa expulsa quem entrou com a antiga
    lado cli timeout 40 "$B" probe "$ALVO" --password senha-fixa-de-teste-1 --frames 100000 --timeout 30 > "$T/viva.out" 2> /dev/null &
    VP=$!
    for _ in $(seq 40); do grep -q '"first_frame"' "$T/viva.out" && break; sleep 0.5; done
    grep -q '"first_frame"' "$T/viva.out" || { echo "FALHOU: a sessão viva não abriu"; exit 1; }
    VID=$(python3 -c "import json
for l in open('$T/serve.out'):
    d=json.loads(l)
    if d.get('event')=='connection' and d.get('unattended'): v=d['id']
print(v)")
    echo '{"cmd":"setup","otp_length":6,"fixed_password":"outra-senha-fixa-2"}' >&3
    for _ in $(seq 16); do grep -q "\"event\":\"removed\"" <(grep "\"id\":$VID[,}]" "$T/serve.out") && break; sleep 0.5; done
    kill $VP 2>/dev/null
    if ! grep "\"id\":$VID[,}]" "$T/serve.out" | grep -q '"event":"removed"'; then echo "FALHOU: trocar a senha fixa não encerrou a sessão $VID que entrou com a antiga"; exit 1; fi
    echo "ok: trocar a senha fixa encerrou na hora a sessão que entrou com a antiga"
    # desligar o acesso sem supervisão vale já: o servidor recebe a configuração nova e recusa a senha fixa
    echo '{"cmd":"setup","otp_length":6}' >&3; sleep 1
    lado cli timeout 60 "$B" probe "$ALVO" --password senha-fixa-de-teste-1 --frames 1 --timeout 8 > "$T/revogada.out" 2> /dev/null
    if grep -q '"first_frame"' "$T/revogada.out"; then echo "FALHOU: a senha fixa continuou valendo depois de desligar o acesso sem supervisão"; exit 1; fi
    echo "OK: depois de desligar, a senha fixa é recusada na hora"; exit 0; fi
  echo "FALHOU: acesso sem supervisão"; tail -5 "$T/serve.err"; exit 1
fi
if [ "$MODO" = gravar ]; then
  sleep 2
  if [ -n "${MAQUINAS:-}" ]; then   # a gravação fica na B: confere lá e traz as permissões
    ARQ=$($KALANGO_SSH_B "find '$T/srv/Vídeos/Kalango Remote' -type f -size +1k 2>/dev/null | head -1")
    PD=$($KALANGO_SSH_B "stat -c %a '$T/srv/Vídeos/Kalango Remote'"); PA=$($KALANGO_SSH_B "stat -c %a '$ARQ' 2>/dev/null")
  else
  ARQ=$(find "$T/srv/Vídeos/Kalango Remote" -type f -size +1k 2>/dev/null | head -1)
  PD=$(stat -c %a "$T/srv/Vídeos/Kalango Remote"); PA=$(stat -c %a "$ARQ" 2>/dev/null)
  fi
  [ "$PD" = 700 ] && [ "$PA" = 600 ] || { echo "FALHOU: permissões da gravação: pasta $PD arquivo $PA (esperado 700 e 600)"; exit 1; }
  grep -q "being recorded" "$T/probe.out" || { echo "FALHOU: quem foi gravado não recebeu o aviso"; exit 1; }
  if [ $R -eq 0 ] && [ -n "$ARQ" ]; then echo "OK: a sessão recebida foi gravada em $ARQ (${MAQUINAS:+na B, }$(if [ -n "${MAQUINAS:-}" ]; then $KALANGO_SSH_B "stat -c %s '$ARQ'"; else stat -c %s "$ARQ"; fi) bytes)"; exit 0; fi
  echo "FALHOU: nenhum vídeo gravado"; ls -R "$T/srv" | head; tail -5 "$T/serve.err"; exit 1
fi
if [ "$MODO" = impostor ]; then
  if [ $R -ne 0 ] && grep -q "does not match the invite" "$T/probe.out" && ! grep -q '"first_frame"' "$T/probe.out"; then
    echo "OK: chave diferente da do convite, sessão recusada sem cair para texto aberto"; exit 0
  fi
  echo "FALHOU: o impostor não foi recusado"; exit 1
fi
if [ $R -eq 0 ] && grep -q '"secured":true' "$T/probe.out"; then echo "OK: sessão cifrada de ponta a ponta, quadros recebidos ($T/primeiro-quadro.png)"; exit 0; fi
echo "FALHOU (probe saiu $R)"; tail -5 "$T/probe.err"; exit 1
