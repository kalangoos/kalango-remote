#!/bin/bash
# Kalango Remote · os sons da sessão: o app pede `kalango-remote-start`/`-stop` ao shell do KalangoOS pela ação
# play-sound (org.gtk.Actions no barramento da sessão). Num barramento só deste teste (dbus-run-session), com um
# substituto do shell que anota o pedido; e sem ninguém com o nome: o pedido falha na hora, em silêncio.
set -u
cd "$(dirname "$0")/../.."
T=$(mktemp -d /tmp/kalango-remote-sons-XXXX)
cat > "$T/shell.py" <<'PY'
import sys
from gi.repository import Gio, GLib
app = Gio.Application(application_id="os.kalango.Shell", flags=Gio.ApplicationFlags.IS_SERVICE)
a = Gio.SimpleAction.new("play-sound", GLib.VariantType.new("s"))
a.connect("activate", lambda _a, p: (print("pedido", p.get_string(), flush=True)))
app.add_action(a)
app.set_inactivity_timeout(20000)
app.run([])
PY
/usr/bin/dbus-run-session -- bash -c '
  cargo test -q -p kalango-remote --offline shell_sound::tests::asks -- --ignored 2>&1 | grep -E "test result|panicked" | sed "s/^/  sem o shell: /"
  /usr/bin/python3 "'"$T"'/shell.py" > "'"$T"'/shell.log" 2>&1 & P=$!
  sleep 1
  KALANGO_REMOTE_TESTE_SHELL=sim KALANGO_REMOTE_DEBUG=1 cargo test -q -p kalango-remote --offline shell_sound::tests::asks -- --ignored --nocapture 2>&1 | grep -E "debug som|test result|panicked" | sed "s/^/  com o shell: /"
  sleep 0.5; kill $P
  grep -q "pedido kalango-remote-start" "'"$T"'/shell.log" && echo "  ok o substituto do shell recebeu kalango-remote-start" || { echo "  FALHOU o pedido não chegou"; exit 1; }'
