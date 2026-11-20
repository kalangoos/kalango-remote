#!/bin/bash
# Kalango Remote · um niri aninhado num Xvfb para testar a captura e a entrada por wlr (sem tocar na sua
# sessão). Usa o niri do KalangoOS (KALANGO_NIRI, ou o compilado em SDRustOS/poc/.deps).
#   kalangoos/scripts/niri-aninhado.sh [DISPLAY] [LARGURAxALTURA]    (padrão :89 1280x800)
# Escreve WAYLAND_DISPLAY em $SAIDA/env (SAIDA padrão: target/niri-aninhado) e fica rodando até ser morto.
set -u
DISP=${1:-:89}
TAM=${2:-1280x800}
cd "$(dirname "$0")/../.."
SAIDA=${SAIDA:-$PWD/target/niri-aninhado}
N=${KALANGO_NIRI:-$PWD/../SDRustOS/poc/.deps/src/niri/target/release/niri}
mkdir -p "$SAIDA"
cat > "$SAIDA/niri.kdl" <<KDL
output "winit" {
    mode "${TAM}"
}
hotkey-overlay {
    skip-at-startup
}
prefer-no-csd
KDL
if ! xdpyinfo -display "$DISP" >/dev/null 2>&1; then
  Xvfb "$DISP" -screen 0 "${TAM%x*}x$((${TAM#*x} + 200))x24" >/dev/null 2>&1 &
  XP=$!
  sleep 1
fi
env -u WAYLAND_DISPLAY DISPLAY="$DISP" __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json LIBGL_ALWAYS_SOFTWARE=1 \
  dbus-run-session -- "$N" --config "$SAIDA/niri.kdl" > "$SAIDA/niri.log" 2>&1 &
NP=$!
trap 'kill $NP ${XP:-} 2>/dev/null' EXIT
for _ in $(seq 40); do grep -q "listening on Wayland socket" "$SAIDA/niri.log" && break; sleep 0.5; done
S=$(grep -o "listening on Wayland socket: [^ ]*" "$SAIDA/niri.log" | awk '{print $NF}')
# a saída do niri aninhado segue o tamanho da janela X dele
W=$(DISPLAY="$DISP" xdotool search --name "^niri$" | head -1)
[ -n "$W" ] && DISPLAY="$DISP" xdotool windowsize "$W" "${TAM%x*}" "${TAM#*x}" && sleep 1
echo "export WAYLAND_DISPLAY=$S DISPLAY=$DISP" > "$SAIDA/env"
echo "niri aninhado em $S (source $SAIDA/env)"
wait $NP
