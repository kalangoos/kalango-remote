#!/bin/bash
# Kalango Remote · o fluxo inteiro pela interface, no niri aninhado (kalangoos/scripts/niri-aninhado.sh rodando):
# compartilhar (túnel de verdade do Cloudflare) → colar o link → senha → o pedido de acesso aparece → Permitir →
# a janela da sessão mostra a tela compartilhada. Capturas em target/app-teste/. Perfil descartável (--demo).
# Só mata o que ele mesmo abriu.
set -u
cd "$(dirname "$0")/../.."
source target/niri-aninhado/env
O=target/app-teste
mkdir -p "$O"
LOG=$O/app.log
X() { DISPLAY="$DISPLAY" xdotool "$@"; }
cap() { kalangoos/scripts/capturar.sh "$O/$1.png" "${2:-}"; echo "captura: $O/$1.png"; }
KALANGO_REMOTE_DEBUG=senha setsid kalangoos/scripts/rodar-app.sh --demo "$@" > "$LOG" 2>&1 &
AP=$!
trap 'kill $AP 2>/dev/null; for c in $(pgrep -P $AP); do kill $c; done' EXIT
sleep 4
X windowfocus --sync "$(X search --name '^niri$' | head -1)"
cap 01-inicio 1500
SOCK=$(grep -ao "IPC listening on: [^ ]*" target/niri-aninhado/niri.log | tail -1 | awk '{print $NF}' | sed 's/\x1b\[[0-9;]*m//g')
N=${KALANGO_NIRI:-${KALANGO_DEPS:-$PWD/../SDRustOS/poc/.deps}/src/niri/target/release/niri}
# as janelas do app (o diálogo do pedido e a sessão também: pelo processo, não pelo app_id)
janelas() { NIRI_SOCKET=$SOCK "$N" msg -j windows | python3 -c "import json,sys,os; ps={int(x) for x in sys.argv[1:]}; print(sum(1 for w in json.load(sys.stdin) if w['pid'] in ps))" $(pgrep -P $AP) $AP; }
# os pontos abaixo valem para a janela de 1500 px com a borda esquerda em x=404: desloca pela posição real
WX=$(NIRI_SOCKET=$SOCK "$N" msg -j windows | python3 -c "import json,sys; w=[w for w in json.load(sys.stdin) if w['app_id'].startswith('os.kalango.Remote')]; print(int(w[0]['layout']['tile_pos_in_workspace_view'][0]))")
OX=$(( WX - 404 ))
X mousemove $((518 + OX + 194)) 396 click 1
for _ in $(seq 60); do grep -q "debug compartilhando" "$LOG" && break; sleep 1; done
grep -q "debug compartilhando" "$LOG" || { echo "FALHOU: o túnel não abriu"; cap erro; exit 1; }
sleep 1
cap 02-compartilhando
P=$(grep -o "senha [^ ]*" "$LOG" | tail -1 | cut -d' ' -f2)
X mousemove $((774 + OX)) 217 click 1           # Copiar link
sleep 0.5
X mousemove $((1135 + OX)) 156 click 1 key ctrl+a ctrl+v
sleep 0.3
X key Return
sleep 1
X type --delay 30 "$P"
X key Return
sleep 2
cap 03-conectando
# o pedido de acesso é uma janela nova no niri (o túnel pode levar alguns segundos)
for _ in $(seq 60); do [ "$(janelas)" -ge 2 ] && break; sleep 1; done
sleep 2
cap 04-pedido
# o foco no diálogo (a janela mais nova do app) e Enter permite
D=$(NIRI_SOCKET=$SOCK "$N" msg -j windows | python3 -c "import json,sys; w=[w for w in json.load(sys.stdin) if w['title'] in ('Pedido de acesso remoto','Remote access request')]; print(w[0]['id'] if w else '')")
[ -n "$D" ] || { echo "FALHOU: o pedido de acesso não apareceu"; cap erro; exit 1; }
NIRI_SOCKET=$SOCK "$N" msg action focus-window --id "$D"
sleep 0.5
X key Return
for _ in $(seq 30); do [ "$(janelas)" -ge 2 ] && grep -q "dada ao núcleo\|janela da sessão" "$LOG" && break; sleep 1; done
sleep 6
cap 05-sessao
X mousemove 1400 60                              # o ponteiro no topo traz a barra
sleep 1
cap 06-sessao-barra
grep -q panicked "$LOG" && { echo "FALHOU: pânico no app"; grep -A3 panicked "$LOG"; exit 1; }
# a sessão abriu de verdade: a janela dela existe no niri
[ "$(janelas)" -ge 2 ] || { echo "FALHOU: a janela da sessão não abriu"; exit 1; }
if [ -n "${KALANGO_ESC_DUPLO:-}" ]; then
  # Esc Esc com o foco longe do aviso da sessão (na janela principal) encerra a sessão: o niri roda KALANGO_ESC_DUPLO
  aviso() { NIRI_SOCKET=$SOCK "$N" msg -j layers | python3 -c "import json,sys; print(sum(1 for l in json.load(sys.stdin) if l['namespace']=='kalango-remote-banner'))"; }
  [ "$(aviso)" -ge 1 ] || { echo "FALHOU: o aviso da sessão não está aberto"; NIRI_SOCKET=$SOCK "$N" msg -j layers > $O/layers.json 2>&1; NIRI_SOCKET=$SOCK "$N" msg -j windows > $O/windows.json 2>&1; exit 1; }
  P=$(NIRI_SOCKET=$SOCK "$N" msg -j windows | python3 -c "import json,sys; print([w['id'] for w in json.load(sys.stdin) if w['title']=='Kalango Remote'][0])")
  NIRI_SOCKET=$SOCK "$N" msg action focus-window --id "$P"
  sleep 0.5
  X key Escape; sleep 0.15; X key Escape
  for _ in $(seq 20); do [ "$(aviso)" -eq 0 ] && break; sleep 0.5; done
  cap 07-esc-esc
  [ "$(aviso)" -eq 0 ] || { echo "FALHOU: Esc Esc não encerrou a sessão"; exit 1; }
  echo "Esc Esc encerrou a sessão"
fi
echo OK
