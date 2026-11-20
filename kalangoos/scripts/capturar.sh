#!/bin/bash
# Kalango Remote · captura a tela do niri aninhado (kalangoos/scripts/niri-aninhado.sh) com o grim.
#   kalangoos/scripts/capturar.sh SAIDA.png [LARGURA-DA-COLUNA]
#   (com LARGURA, a coluna da janela focada fica com essa largura em px antes da captura)
set -u
cd "$(dirname "$0")/../.."
source target/niri-aninhado/env
DEPS=$PWD/../SDRustOS/poc/.deps
SOCK=$(grep -ao "IPC listening on: [^ ]*" target/niri-aninhado/niri.log | tail -1 | awk '{print $NF}' | sed 's/\x1b\[[0-9;]*m//g')
if [ -n "${2:-}" ]; then
  NIRI_SOCKET=$SOCK "$DEPS/src/niri/target/release/niri" msg action set-column-width "$2"
  NIRI_SOCKET=$SOCK "$DEPS/src/niri/target/release/niri" msg action center-column
  sleep 1.2
fi
LD_LIBRARY_PATH=$DEPS/root/usr/lib/x86_64-linux-gnu "$DEPS/root/usr/bin/grim" "$1"
