#!/bin/bash
# Kalango Remote · o núcleo que não abre (kalangoos/scripts/niri-aninhado.sh rodando): KALANGO_REMOTE_CORE aponta para
# um programa que escreve um erro de biblioteca e sai com 127. Compartilhar mostra "O núcleo do Kalango Remote não
# abriu." com a linha do erro, o processo é recolhido (nenhum zumbi) e "Tentar de novo" espera 2 s, depois 4 s.
# Depois, com um núcleo de verdade já servindo nesta instância, Compartilhar diz que outro Kalango Remote já
# compartilha este computador. Tudo numa instância própria do teste (KALANGO_REMOTE_INSTANCE, XDG temporários).
# Capturas em target/app-teste/. Só mata o que ele mesmo abriu.
set -u
cd "$(dirname "$0")/../.."
source target/niri-aninhado/env
O=target/app-teste
mkdir -p "$O"
T=$(mktemp -d /tmp/kalango-remote-nucleo-XXXX)
falhou() { echo "FALHOU: $*"; exit 1; }
cat > "$T/nucleo" <<SH
#!/bin/sh
echo x >> "$T/vezes"
echo "kalango-remote-core: error while loading shared libraries: libyuv.so.0: cannot open shared object file: No such file or directory" >&2
exit 127
SH
chmod +x "$T/nucleo"
X() { DISPLAY="$DISPLAY" xdotool "$@"; }
cap() { kalangoos/scripts/capturar.sh "$O/$1.png" "${2:-}"; echo "captura: $O/$1.png"; }
vezes() { [ -f "$T/vezes" ] && wc -l < "$T/vezes" || echo 0; }
export KALANGO_REMOTE_INSTANCE=nucleo$$ KALANGO_REMOTE_DEMO="$T/perfil"
KALANGO_REMOTE_CORE="$T/nucleo" setsid kalangoos/scripts/rodar-app.sh --demo > "$T/app.log" 2>&1 &
AP=$!
trap 'kill $AP ${HP:-} 2>/dev/null; echo "registros em $T"' EXIT
sleep 4
X windowfocus --sync "$(X search --name '^niri$' | head -1)"
SOCK=$(grep -ao "IPC listening on: [^ ]*" target/niri-aninhado/niri.log | tail -1 | awk '{print $NF}' | sed 's/\x1b\[[0-9;]*m//g')
N=${KALANGO_NIRI:-${KALANGO_DEPS:-$PWD/../SDRustOS/poc/.deps}/src/niri/target/release/niri}
# o botão roxo largo de compartilhar, achado na captura (a janela muda de tamanho conforme a saída)
DISPLAY="$DISPLAY" xdotool mousemove 1500 600   # o ponteiro longe (sem o realce de passar por cima)
sleep 0.5
cap 19-antes
read SX SY < <(python3 - "$O/19-antes.png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB"); w, h = im.size
best = None
for y in range(0, h, 2):
    xs = [x for x in range(0, w, 2) if (lambda p: 155 <= p[0] <= 195 and 150 <= p[1] <= 190 and 215 <= p[2] <= 240)(im.getpixel((x, y)))]
    if len(xs) > 150 and (best is None or len(xs) > best[0]):
        best = (len(xs), (xs[0] + xs[-1]) // 2, y)
print(best[1], best[2])
PY
)
APP=$(pgrep -P $AP -x kalango-remote || echo $AP)
X mousemove "$SX" "$SY" click 1                    # Compartilhar este computador
sleep 0.8
# "Tentar de novo" logo depois do erro ainda está esperando (2 s): o clique não roda o núcleo de novo
# (ele fica 150 px acima de onde estava o botão de compartilhar)
RX=$((SX - 76)); RY=$((SY - 150))
X mousemove $RX $RY click 1
sleep 0.5
[ "$(vezes)" -eq 1 ] || falhou "o botão não esperou ($(vezes) tentativas)"
cap 20-nucleo-falhou
[ "$(vezes)" -eq 1 ] || falhou "o núcleo rodou $(vezes) vezes, esperava 1"
grep -q "libyuv.so.0" "$T/app.log" || falhou "o erro do núcleo não chegou ao registro"
Z=$(ps --ppid "$APP" -o stat= | grep -c Z)
[ "$Z" -eq 0 ] || falhou "$Z processo(s) zumbi do núcleo"
echo "   ok: 1 tentativa, erro mostrado, nenhum zumbi"
sleep 2
X mousemove $RX $RY click 1                        # depois da espera: roda de novo, e a próxima espera é maior
sleep 1.5
cap 21-nucleo-falhou-de-novo
[ "$(vezes)" -eq 2 ] || falhou "o núcleo rodou $(vezes) vezes, esperava 2"
Z=$(ps --ppid "$APP" -o stat= | grep -c Z)
[ "$Z" -eq 0 ] || falhou "$Z processo(s) zumbi do núcleo"
python3 - "$O/21-nucleo-falhou-de-novo.png" "$RX" "$RY" <<'PY' || falhou "a segunda espera não é maior (o botão devia estar desligado)"
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
x, y = int(sys.argv[2]), int(sys.argv[3])
# desligado, o texto do botão fica apagado: nenhum pixel claro na faixa do rótulo
light = sum(1 for dx in range(-80, 80) for dy in range(-6, 6) if sum(im.getpixel((x + dx, y + dy))) > 600)
sys.exit(0 if light == 0 else 1)
PY
echo "   ok: o botão esperou e a segunda tentativa também foi recolhida (agora espera mais)"

# outro núcleo desta mesma instância já servindo (como outra janela compartilhando)
kill $AP; sleep 1
mkfifo "$T/entrada"
( source kalangoos/scripts/env.sh; exec env XDG_CONFIG_HOME="$T/perfil/config" KALANGO_CORE_TEXT=1 target/debug/kalango-remote-core serve --no-tunnel < "$T/entrada" > "$T/outro.out" 2> "$T/outro.err" ) &
HP=$!
exec 3> "$T/entrada"
for _ in $(seq 40); do grep -q '"listening"' "$T/outro.out" && break; sleep 0.5; done
grep -q '"listening"' "$T/outro.out" || falhou "o outro núcleo não começou"
setsid kalangoos/scripts/rodar-app.sh --demo > "$T/app2.log" 2>&1 &
AP=$!
sleep 4
X mousemove 1500 600; sleep 0.5
X mousemove "$SX" "$SY" click 1
sleep 2
cap 22-ja-compartilhado
python3 - "$O/22-ja-compartilhado.png" "$SX" <<'PY' || falhou "o cartão não diz que já está compartilhado (o selo dourado)"
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
# the gold badge at the top right of the card (#E5C98C text)
gold = sum(1 for x in range(0, im.size[0], 1) for y in range(100, 140) if (lambda p: abs(p[0]-229) < 25 and abs(p[1]-201) < 25 and abs(p[2]-140) < 30)(im.getpixel((x, y))))
sys.exit(0 if gold > 20 else 1)
PY
kill -0 $HP 2>/dev/null || falhou "o núcleo que já servia caiu"
echo "   ok: com outro compartilhando, o cartão diz isso e o outro continua de pé"

# um núcleo que começa (diz "listening") e para 1,5 s depois: o cartão não volta ao início calado
kill $AP $HP; exec 3>&-; sleep 1
cat > "$T/para-depois" <<SH
#!/bin/sh
"$PWD/target/debug/kalango-remote-core" "\$@" &
P=\$!
sleep 1.5
kill \$P
wait \$P
SH
chmod +x "$T/para-depois"
KALANGO_REMOTE_CORE="$T/para-depois" setsid kalangoos/scripts/rodar-app.sh --demo > "$T/app3.log" 2>&1 &
AP=$!
sleep 4
X mousemove 1500 600; sleep 0.5
X mousemove "$SX" "$SY" click 1
sleep 4
cap 23-nucleo-parou
grep -q "o núcleo saiu" "$T/app3.log" || falhou "o app não registrou a saída do núcleo"
python3 - "$O/23-nucleo-parou.png" <<'PY' || falhou "o cartão não mostra o núcleo parado (selo coral)"
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
coral = sum(1 for x in range(im.size[0]) for y in range(100, 140) if (lambda p: abs(p[0]-239) < 25 and abs(p[1]-162) < 30 and abs(p[2]-166) < 30)(im.getpixel((x, y))))
sys.exit(0 if coral > 20 else 1)
PY
echo "   ok: o núcleo que para depois de começar também aparece como erro ($(grep -a 'o núcleo saiu' "$T/app3.log" | head -1 | cut -c1-80))"
echo OK
