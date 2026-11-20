#!/bin/bash
# Kalango Remote · baixa o cloudflared (Apache-2.0) da versão fixada, confere o SHA-256 e deixa em
# kalangoos/.deps/bin/cloudflared. O binário nunca entra no repositório.
#   kalangoos/scripts/cloudflared.sh            (imprime o caminho do binário)
set -eu
VERSAO=2026.10.7
case "$(uname -m)" in
  x86_64)  ARQ=amd64; SHA=1f99fb518b83dec2ba8d870a7cc991d82a77e6196295a3d2754ba17ab9f91872 ;;
  aarch64) ARQ=arm64; SHA=3df43aedc9ff291d1735b5cb576cb97e0b157488796db2e32b28aecabb445775 ;;
  *) echo "arquitetura sem cloudflared fixado: $(uname -m)" >&2; exit 1 ;;
esac
DEST="$(cd "$(dirname "$0")/.." && pwd)/.deps/bin"
BIN="$DEST/cloudflared"
mkdir -p "$DEST"
if [ -x "$BIN" ] && echo "$SHA  $BIN" | sha256sum -c --status; then echo "$BIN"; exit 0; fi
TMP=$(mktemp "$DEST/.cloudflared.XXXX")
trap 'rm -f "$TMP"' EXIT
curl -fsSL --retry 3 -o "$TMP" "https://github.com/cloudflare/cloudflared/releases/download/$VERSAO/cloudflared-linux-$ARQ"
if ! echo "$SHA  $TMP" | sha256sum -c --status; then
  echo "SHA-256 do cloudflared $VERSAO não confere; nada foi instalado" >&2; exit 1
fi
chmod +x "$TMP"; mv "$TMP" "$BIN"; trap - EXIT
echo "$BIN"
