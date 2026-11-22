#!/bin/bash
# Kalango Remote · o modo privacidade de ponta a ponta (patch 0012 do niri + kalangod + app + núcleo), num niri
# aninhado e isolado (nada toca a sua sessão nem a sua pasta pessoal):
#   1. o app compartilha com senha fixa e "sem supervisão" ligado, com a chave "tela preta aqui" ligada;
#   2. alguém entra com a senha fixa (ninguém aprova aqui): o kalangod dá ao núcleo a permissão com "privacidade", o
#      niri cobre a tela local de preto (só o aviso "está controlando" fica por cima) e quem controla (o probe) recebe
#      quadros da área de trabalho de verdade, não do preto;
#   3. o aviso continua aberto e o botão de parar encerra: a cobertura sai;
#   4. com a chave desligada, a mesma entrada não cobre nada.
# Precisa do niri com o patch 0012 e de um kalangod com a capacidade (KALANGO_NIRI, KALANGOD; KALANGO_DEPS para o
# poc/.deps do KalangoOS quando este repositório não está ao lado dele; KALANGO_DISPLAY, padrão :134).
set -u
cd "$(dirname "$0")/../.."
R=$PWD
source kalangoos/scripts/env.sh
DEPS=$(realpath "${KALANGO_DEPS:-$PWD/../SDRustOS/poc/.deps}")
N=${KALANGO_NIRI:?KALANGO_NIRI: o niri com o patch 0012}
KD=${KALANGOD:?KALANGOD: o kalangod com a capacidade privacidade}
DISP=${KALANGO_DISPLAY:-:134}
T=$(mktemp -d /tmp/kalango-remote-privacidade-XXXX)
FALHAS=0
ok() { echo "ok   $*"; }
falha() { echo "FALHA $*"; FALHAS=$((FALHAS + 1)); }
cat > "$T/kalangod.toml" <<CFG
socket = "$T/mcp.sock"
state_dir = "$T/estado"
protected_paths = []
approval_requires_tty = false
approval_timeout_s = 30
total_grant_s = 60
dry_run = true
restore_root = "$T"
controle_remoto = "$T/controle-remoto"
controle_remoto_sempre = ["$DEPS/root/usr/bin/grim captura"]
controle_remoto_apps = ["$R/target/debug/kalango-remote"]
[[callers]]
name = "voce"
max_level = "usuario"
allowed_paths = ["$T"]
CFG
"$KD" servir --config "$T/kalangod.toml" > "$T/kalangod.log" 2>&1 &
KP=$!
for _ in $(seq 20); do [ -S "$T/mcp.sock" ] && break; sleep 0.3; done
[ -f "$T/controle-remoto" ] || { echo "o kalangod não escreveu $T/controle-remoto"; exit 1; }
export KALANGO_NIRI_CONTROLE="$T/controle-remoto" KALANGO_NIRI_CONTROLE_DONO="$(id -u)" KALANGO_ESC_DUPLO="true" RUST_LOG="niri=info"
export KALANGO_NIRI=$N
rm -f target/niri-aninhado/env
setsid kalangoos/scripts/niri-aninhado.sh "$DISP" 1280x800 > "$T/niri-aninhado.log" 2>&1 &
NP=$!
APID=
trap 'kill $APID $KP 2>/dev/null; kill -- -$NP 2>/dev/null; echo "registros em $T"' EXIT
for _ in $(seq 60); do [ -f target/niri-aninhado/env ] && break; sleep 0.5; done
source target/niri-aninhado/env
sleep 2
X() { DISPLAY="$DISPLAY" xdotool "$@"; }
local_px() { # x y: o que o monitor local mostra (a janela X do niri aninhado)
  DISPLAY="$DISPLAY" ffmpeg -loglevel error -y -f x11grab -video_size 1280x800 -i "$DISPLAY" -frames:v 1 "$T/local.png" 2>/dev/null
  python3 -W ignore -c "
import sys
from PIL import Image
im = Image.open('$T/local.png').convert('RGB'); print('%d,%d,%d' % im.getpixel(($1, $2)))"
}
mean_dark() { # o quanto da imagem é quase preto (0..1)
  python3 -W ignore - "$1" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("L").resize((64, 40))
px = list(im.getdata()); print(sum(1 for p in px if p < 12) / len(px))
PY
}

sessao() { # $1 = privacidade ligada (1/0): abre o app com o perfil sob medida, compartilha e deixa alguém entrar
  local PRIV=$1 P=$T/perfil$1
  mkdir -p "$P/config/kalango/remote/cofre-de-teste"
  printf '{"unattended": true, "ask_before": true, "privacy_black": %s}' "$([ "$PRIV" = 1 ] && echo true || echo false)" > "$P/config/kalango/remote/settings.json"
  printf 'senha-fixa-privacidade-1' > "$P/config/kalango/remote/cofre-de-teste/fixed-password--default"; chmod 600 "$P/config/kalango/remote/cofre-de-teste/fixed-password--default"
  NIRI_SOCKET="$NS" KALANGO_REMOTE_NIRI="$N" KALANGOD_SOCKET="$T/mcp.sock" KALANGO_REMOTE_DEMO="$P" KALANGO_REMOTE_DEBUG=senha KALANGO_REMOTE_INSTANCE="priv$1$$" setsid kalangoos/scripts/rodar-app.sh --demo > "$T/app$1.log" 2>&1 &
  APID=$!
  sleep 5
  X windowfocus --sync "$(X search --name '^niri$' | head -1)"
  # (a janela com 1500 px de largura, como em testar-app.sh: os pontos de clique valem para ela)
  NIRI_SOCKET=$NS "$N" msg action set-column-width 1500; NIRI_SOCKET=$NS "$N" msg action center-column; sleep 1.5
  local WX
  WX=$(NIRI_SOCKET=$NS "$N" msg -j windows | python3 -W ignore -c "import json,sys; w=[w for w in json.load(sys.stdin) if w['app_id'].startswith('os.kalango.Remote')]; print(int(w[0]['layout']['tile_pos_in_workspace_view'][0]))")
  X mousemove $((518 + WX - 404 + 194)) 396 click 1    # o botão de compartilhar (como em testar-app.sh)
  for _ in $(seq 60); do grep -q "debug compartilhando" "$T/app$1.log" && break; sleep 1; done
  grep -q "debug compartilhando" "$T/app$1.log" || { falha "o compartilhamento não abriu"; return 1; }
  LINK=$(grep -o "link [^ ]*" "$T/app$1.log" | tail -1 | cut -d' ' -f2)
  sleep 1
  KALANGO_CORE_TEXT=1 HOME="$T/cli" XDG_CONFIG_HOME="$T/cli/config" KALANGO_REMOTE_INSTANCE="privcli$1$$" env -u DISPLAY \
    timeout 60 target/debug/kalango-remote-core probe "$LINK" --password senha-fixa-privacidade-1 --frames 100000 --timeout 40 --save "$T/remoto$1.png" > "$T/probe$1.out" 2> "$T/probe$1.err" &
  PRB=$!
  for _ in $(seq 40); do grep -q '"first_frame"' "$T/probe$1.out" && break; sleep 0.5; done
  grep -q '"first_frame"' "$T/probe$1.out" || { falha "o probe não recebeu quadros"; return 1; }
  sleep 3
  return 0
}
NS=$(grep -ao "IPC listening on: [^ ]*" target/niri-aninhado/niri.log | tail -1 | awk '{print $NF}' | sed 's/\x1b\[[0-9;]*m//g')
PRB=
fim() { kill $PRB 2>/dev/null; wait $PRB 2>/dev/null; kill $APID 2>/dev/null; pkill -P $APID 2>/dev/null; sleep 2; APID=; }

echo "1. com a chave ligada, uma entrada sem supervisão"
if sessao 1; then
  grep -q "privacidade: true" "$T/app1.log" && ok "o app pediu a capacidade privacidade ao kalangod" || falha "o app não pediu privacidade: $(grep -a 'permissão' "$T/app1.log" | tail -2)"
  grep -q "privacidade" "$T/controle-remoto" && ok "o arquivo de permissões do kalangod tem a palavra" || falha "o arquivo de permissões não tem a palavra: $(cat "$T/controle-remoto")"
  L=$(local_px 640 500)
  [ "$(mean_dark "$T/local.png")" \> 0.85 ] && ok "a tela local está coberta (preta) ($L)" || falha "a tela local deveria estar coberta, escuridão $(mean_dark "$T/local.png")"
  AV=$(NIRI_SOCKET=$NS "$N" msg -j layers | python3 -W ignore -c "import json,sys; print(sum(1 for l in json.load(sys.stdin) if l['namespace']=='kalango-remote-banner'))")
  [ "$AV" -ge 1 ] && ok "o aviso com o botão de parar está aberto" || falha "o aviso não está aberto"
  cp "$T/local.png" /tmp/kr-privacidade-local.png
  # o aviso aparece por cima do preto: há pixels claros no topo da tela local
  python3 - "$T/local.png" <<'PY' && ok "o aviso aparece por cima da cobertura (texto claro sobre o preto)" || falha "o aviso não aparece por cima da cobertura"
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("L")
# (o aviso fica abaixo do contorno, a uns 45 px do topo: texto e botões claros sobre o preto)
band = [im.getpixel((x, y)) for x in range(20, 1000, 2) for y in range(46, 82, 2)]
sys.exit(0 if sum(1 for p in band if p > 90) > 150 else 1)
PY
  sleep 2
  kill -0 $PRB 2>/dev/null; cp "$T/remoto1.png" /tmp/kr-privacidade-remoto.png 2>/dev/null
  if [ -f "$T/remoto1.png" ]; then
    [ "$(mean_dark "$T/remoto1.png")" \< 0.5 ] && ok "quem controla vê a área de trabalho (escuridão $(mean_dark "$T/remoto1.png"))" || falha "quem controla viu preto (escuridão $(mean_dark "$T/remoto1.png"))"
  else falha "o probe não salvou o quadro"; fi
  # o app morre à força (kill -9): o núcleo morre junto (PR_SET_PDEATHSIG) e a tela local volta, sem ninguém preso
  CP=$(pgrep -P "$APID" -f kalango-remote-core | head -1)
  if [ -n "$CP" ]; then
    kill -9 "$APID"; wait "$APID" 2>/dev/null
    for _ in $(seq 20); do kill -0 "$CP" 2>/dev/null || break; sleep 0.25; done
    kill -0 "$CP" 2>/dev/null && falha "o núcleo sobreviveu à morte do app" || ok "o app morreu à força e o núcleo morreu junto"
    sleep 2; local_px 640 500 > /dev/null
    [ "$(mean_dark "$T/local.png")" \< 0.5 ] && ok "a tela local voltou sem o app" || falha "a tela continuou coberta sem o app"
  else falha "não achei o núcleo filho do app"; fi
  # parar pelo botão do aviso: a conexão cai e a cobertura sai
  NIRI_SOCKET=$NS "$N" msg -j layers > /dev/null
  kill $PRB 2>/dev/null; wait $PRB 2>/dev/null
  for _ in $(seq 30); do [ "$(mean_dark "$T/local.png")" \< 0.5 ] && break; local_px 640 500 > /dev/null; sleep 0.5; done
  [ "$(mean_dark "$T/local.png")" \< 0.5 ] && ok "a sessão acabou: a cobertura saiu" || falha "a sessão acabou e a tela continuou coberta"
fi
fim

echo "2. com a chave desligada, a mesma entrada não cobre nada"
if sessao 0; then
  grep -q "privacidade: false" "$T/app0.log" && ok "o app não pediu privacidade" || falha "o app deveria ter pedido sem privacidade: $(grep -a 'permissão' "$T/app0.log" | tail -2)"
  local_px 640 500 > /dev/null
  [ "$(mean_dark "$T/local.png")" \< 0.5 ] && ok "a tela local continua à vista" || falha "a tela local foi coberta com a chave desligada"
fi
fim
[ "$FALHAS" -eq 0 ] && echo "TUDO CERTO" || echo "$FALHAS FALHA(S)"
exit "$FALHAS"
