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
#     servidor  servidor próprio: sobe o hbbs/hbbr oficiais num docker (rustdesk/rustdesk-server, rede do host), o
#             núcleo compartilha registrando nele e o probe conecta pelo id e pela chave pública do servidor
#     impostor  o convite traz outra chave: a sessão tem de ser recusada (sai 0 se foi)
# O lado compartilhado captura o DISPLAY dado (um Xvfb, padrão :89). Sai 0 se o probe recebeu quadros
# numa sessão cifrada de ponta a ponta. MOVIMENTO=1 anima a tela; QUADROS=N (padrão 30).
set -u
PROBE_EXTRA=()
MODO=${1:-direto}
DISP=${2:-:89}
cd "$(dirname "$0")/../.."
source kalangoos/scripts/env.sh
B=${KALANGO_CORE:-$PWD/target/debug/kalango-remote-core}
T=$(mktemp -d /tmp/kalango-remote-teste-XXXX)
mkdir -p "$T/srv" "$T/cli"
NIRI_ENV=()
if [ "$MODO" = niri ]; then
  source target/niri-aninhado/env || { echo "rode kalangoos/scripts/niri-aninhado.sh antes"; exit 2; }
  NIRI_ENV=(WAYLAND_DISPLAY="$WAYLAND_DISPLAY")
  DISP=$DISPLAY
fi
lado() { # pasta pessoal descartável + instância própria
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
    QUADROS=1 ;;
  duasetapas) ARGS=(--no-tunnel --setup-stdin)
    SEGREDO=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ   # 20 bytes ("12345678901234567890"); o núcleo exige 128 bits ou mais
    SETUP="{\"cmd\":\"setup\",\"otp_length\":6,\"two_factor\":\"$SEGREDO\"}"
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
mkfifo "$T/entrada"
# MOVIMENTO=1: uma bola animada a 30 qps na tela compartilhada (o RustDesk só manda quadros quando algo muda)
if [ "${MOVIMENTO:-0}" = 1 ]; then
  DISPLAY="$DISP" gst-launch-1.0 -q videotestsrc pattern=ball ! video/x-raw,width=800,height=600,framerate=30/1 ! ximagesink > /dev/null 2>&1 &
  GP=$!
fi
lado srv "$B" serve "${ARGS[@]}" < "$T/entrada" > "$T/serve.out" 2> "$T/serve.err" &
SP=$!
exec 3> "$T/entrada"   # mantém a entrada do servidor aberta
[ -n "${SETUP:-}" ] && echo "$SETUP" >&3   # --setup-stdin: o primeiro pacote é a configuração
trap 'exec 3>&-; [ "$MODO" = servidor ] && docker rm -f "kr-teste-hbbs-$$" "kr-teste-hbbr-$$" > /dev/null 2>&1; kill $SP ${GP:-} ${JP:-} 2>/dev/null; wait $SP 2>/dev/null; echo "registros em $T"' EXIT
campo() { python3 -c "import json,sys
for l in open('$T/serve.out'):
    d=json.loads(l)
    if d.get('event')=='$1': print(d['$2']); break"; }
for _ in $(seq 60); do grep -q '"listening"' "$T/serve.out" && break; sleep 0.5; done
PW=$(campo listening password); ID=$(campo listening id); PK=$(campo listening pk); ADDR=$(campo listening addr); CODE=$(campo listening code)
case "$MODO" in
  direto) ALVO="https://$ADDR/#k=$ID.$PK" ;;
  codigo) ALVO="$CODE" ;;
  niri) ALVO="https://$ADDR/#k=$ID.$PK"; PROBE_EXTRA=(--click 320,400 --type "Kalango Remote 123") ;;
  aprovar) ALVO="https://$ADDR/#k=$ID.$PK"
    ( for _ in $(seq 60); do grep -q '"event":"connection"' "$T/serve.out" && break; sleep 0.5; done
      sleep 3; CID=$(campo connection id); date +%s.%N > "$T/aprovado-em"
      echo "{\"cmd\":\"authorize\",\"id\":$CID,\"keyboard\":true,\"clipboard\":true,\"audio\":false,\"file\":false}" >&3 ) & ;;
  servidor) ALVO=$ID; PROBE_EXTRA=(--server 127.0.0.1 --key "$CHAVE"); sleep 4 ;;   # espera o registro no servidor
  fixa|duasetapas|gravar) ALVO="https://$ADDR/#k=$ID.$PK" ;;
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
    python3 - "$T" "$INICIO" <<'PY' && exit 0
import json, sys
t = sys.argv[1]; inicio = float(sys.argv[2]); aprovado = float(open(t + "/aprovado-em").read())
ff = [json.loads(l) for l in open(t + "/probe.out") if '"first_frame"' in l][0]
quando = inicio + ff["after_ms"] / 1000
print(f"aprovado {aprovado - inicio:.1f} s depois do início; primeiro quadro {quando - inicio:.1f} s")
if quando < aprovado: sys.exit(1)
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
    echo "OK: a senha fixa entrou sem ninguém aprovar (acesso sem supervisão), cifrada de ponta a ponta"; exit 0; fi
  echo "FALHOU: acesso sem supervisão"; tail -5 "$T/serve.err"; exit 1
fi
if [ "$MODO" = gravar ]; then
  sleep 2
  ARQ=$(find "$T/srv/Vídeos/Kalango Remote" -type f -size +1k 2>/dev/null | head -1)
  if [ $R -eq 0 ] && [ -n "$ARQ" ]; then echo "OK: a sessão recebida foi gravada em $ARQ ($(stat -c %s "$ARQ") bytes)"; exit 0; fi
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
