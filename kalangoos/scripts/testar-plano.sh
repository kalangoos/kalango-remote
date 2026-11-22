#!/bin/bash
# Kalango Remote · o plano do agente pela interface (kalangoos/scripts/testar-plano.sh [DISPLAY], padrão :89, um Xvfb
# de pelo menos 1600×1000 sem gerenciador de janelas): `kalango-remote agent-plan` mostra o painel; cada botão
# encerra o comando com a decisão no stdout e o código de saída combinado (aprovar 0, cancelar 1, ajustar 3);
# aprovar começa a conexão (a senha do outro computador é pedida como sempre). Perfil descartável (--demo).
set -u
cd "$(dirname "$0")/../.."
DISP=${1:-:89}
T=$(mktemp -d /tmp/kalango-remote-plano-XXXX)
cat > "$T/plano.json" <<'JSON'
{"device":"Notebook da mãe","request":"conecte no notebook da minha mãe e instale a impressora",
 "steps":[{"title":"Conectar no Notebook da mãe","detail":"via Cloudflare","approval":"you"},
          {"title":"Adicionar a impressora","detail":"pode pedir a senha dela","approval":"them"},
          {"title":"Encerrar a sessão"}],
 "limits":"só a janela Configurações"}
JSON
export DISPLAY=$DISP GDK_BACKEND=x11 KALANGO_REMOTE_DEMO=$T/perfil LANGUAGE=pt_BR
# o painel (plano de 3 passos): 560 de largura encostado à direita da janela de 1500 px; os botões ficam em y=462
BOTOES=(${BOTOES_TESTE:-"aprovar:1377:462:0" "cancelar:979:462:1" "ajustar:1085:462:3"})
FALHAS=0
for b in "${BOTOES[@]}"; do
  IFS=: read -r nome x y esperado <<< "$b"
  kalangoos/scripts/rodar-app.sh --demo > "$T/app.log" 2>&1 &
  AP=$!
  sleep 6
  WID=$(xdotool search --onlyvisible --name "Kalango Remote" | head -1)
  xdotool windowmove "$WID" 0 0 windowsize "$WID" 1500 1000
  sleep 1
  kalangoos/scripts/rodar-app.sh --demo agent-plan "$T/plano.json" > "$T/saida.txt" 2>&1 &
  CP=$!
  sleep 3
  [ -n "${CAPTURA:-}" ] && ffmpeg -loglevel error -y -f x11grab -video_size 1600x1000 -i "$DISP" -frames:v 1 "$CAPTURA"
  xdotool mousemove "$x" "$y" click 1
  wait $CP; R=$?
  kill $AP 2>/dev/null; pkill -P $AP 2>/dev/null; sleep 1
  if [ "$R" -eq "$esperado" ] && grep -q "\"decision\":\"$([ "$nome" = aprovar ] && echo approved || { [ "$nome" = cancelar ] && echo cancelled || echo adjust; })\"" "$T/saida.txt"; then
    echo "ok: $nome → saída $R, $(cat "$T/saida.txt")"
  else
    echo "FALHOU: $nome → saída $R, esperado $esperado: $(cat "$T/saida.txt")"; FALHAS=1
  fi
done
echo "registros em $T"
exit $FALHAS
