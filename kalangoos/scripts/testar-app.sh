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
# a janela de 1500 px fica centrada na saída de 1920: os pontos abaixo são do design deslocados pela margem
OX=$(( (1920 - 1500) / 2 - 210 ))
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
for _ in $(seq 40); do grep -q "connection from" "$LOG" && break; sleep 1; done
sleep 3
cap 04-pedido
X key Return                                     # Enter permite
sleep 8
cap 05-sessao
X mousemove 1400 60                              # o ponteiro no topo traz a barra
sleep 1
cap 06-sessao-barra
grep -q panicked "$LOG" && { echo "FALHOU: pânico no app"; grep -A3 panicked "$LOG"; exit 1; }
echo OK
