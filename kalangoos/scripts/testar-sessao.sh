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
trap 'exec 3>&-; kill $SP ${GP:-} ${JP:-} 2>/dev/null; wait $SP 2>/dev/null; echo "registros em $T"' EXIT
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
  impostor) ALVO="https://$ADDR/#k=$ID.$(python3 -c "import base64,os; print(base64.urlsafe_b64encode(os.urandom(32)).decode().rstrip('='))")" ;;
  tunel)
    for _ in $(seq 60); do grep -q '"tunnel_ready"' "$T/serve.out" && grep -q '"tunnel"' "$T/serve.out" && break; sleep 0.5; done
    ALVO=$(campo tunnel link) ;;   # o núcleo só anuncia o link quando o nome já resolve
esac
echo "alvo: $ALVO  código: $CODE"
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
if [ "$MODO" = impostor ]; then
  if [ $R -ne 0 ] && grep -q "does not match the invite" "$T/probe.out" && ! grep -q '"first_frame"' "$T/probe.out"; then
    echo "OK: chave diferente da do convite, sessão recusada sem cair para texto aberto"; exit 0
  fi
  echo "FALHOU: o impostor não foi recusado"; exit 1
fi
if [ $R -eq 0 ] && grep -q '"secured":true' "$T/probe.out"; then echo "OK: sessão cifrada de ponta a ponta, quadros recebidos ($T/primeiro-quadro.png)"; exit 0; fi
echo "FALHOU (probe saiu $R)"; tail -5 "$T/probe.err"; exit 1
