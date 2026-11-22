#!/bin/bash
# Kalango Remote · a permissão de captura e de entrada no niri do KalangoOS (patch 0008 do niri + kalangod):
#   1. um programa qualquer da sessão (o grim) não consegue capturar a tela;
#   2. o núcleo do Kalango Remote rodando sozinho (sem o app, sem aprovação) também não;
#   3. pelo app, depois da aprovação, a sessão mostra a tela (o kalangod deu a permissão ao núcleo) e a linha do
#      tempo do kalangod registra o "permitir" e o "revogar";
#   4. Esc Esc com o foco em outra janela encerra a sessão (patch 0009 do niri).
# Precisa do niri com o patch 0008 e de um kalangod com o controle remoto (KALANGO_NIRI, KALANGOD; padrão: a
# árvore SDRustOS-worktrees/niri-remoto ao lado). Só mata o que ele mesmo abriu.
set -u
cd "$(dirname "$0")/../.."
R=$PWD
WT=${KALANGO_WT:-$PWD/../SDRustOS-worktrees/niri-remoto}
export KALANGO_NIRI=${KALANGO_NIRI:-$WT/poc/.deps/src/niri/target/release/niri}
KD=${KALANGOD:-$WT/target/debug/kalangod}
DEPS=$(realpath "${KALANGO_DEPS:-$PWD/../SDRustOS/poc/.deps}")
DISP=${KALANGO_DISPLAY:-:89}   # o Xvfb em que o niri aninhado roda (um que já exista ou o próprio niri-aninhado.sh abre)
T=$(mktemp -d /tmp/kalango-remote-permissao-XXXX)
falhou() { echo "FALHOU: $*"; exit 1; }
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
# the test's own screenshots (capturar.sh) use this grim; step 1 tries a copy elsewhere, which stays refused
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
[ -f "$T/controle-remoto" ] || falhou "o kalangod não escreveu $T/controle-remoto"
# Esc Esc (no teclado deste computador, com uma sessão acontecendo) encerra a sessão do app de teste
cat > "$T/esc-duplo" <<SH
#!/bin/sh
exec env DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-}" KALANGO_REMOTE_APP_ID=os.kalango.RemoteDemo "$R/kalangoos/scripts/rodar-app.sh" end-sessions >> "$T/esc-duplo.log" 2>&1
SH
chmod +x "$T/esc-duplo"
export KALANGO_ESC_DUPLO="$T/esc-duplo"
export KALANGO_NIRI_CONTROLE="$T/controle-remoto" KALANGO_NIRI_CONTROLE_DONO="$(id -u)" RUST_LOG="niri=info,niri::remote_control=debug"
rm -f target/niri-aninhado/env
setsid kalangoos/scripts/niri-aninhado.sh "$DISP" 1920x1080 > /dev/null 2>&1 &
NP=$!
trap 'kill $KP 2>/dev/null; kill -- -$NP 2>/dev/null; echo "registros em $T"' EXIT
for _ in $(seq 60); do [ -f target/niri-aninhado/env ] && break; sleep 0.5; done
source target/niri-aninhado/env
sleep 1

echo "1. um programa qualquer tenta capturar"
cp "$DEPS/root/usr/bin/grim" "$T/grim-qualquer"
if LD_LIBRARY_PATH=$DEPS/root/usr/lib/x86_64-linux-gnu "$T/grim-qualquer" "$T/grim.png" 2> "$T/grim.err"; then
  falhou "o grim capturou a tela sem permissão"
fi
echo "   ok: $(head -1 "$T/grim.err")"

echo "2. o núcleo sozinho, sem aprovação"
source kalangoos/scripts/env.sh
export KALANGO_CORE_TEXT=1
mkfifo "$T/entrada"
# como um processo da sessão do niri: NIRI_SOCKET, sem o X11 do Xvfb que está por baixo do niri aninhado
NS=$(grep -ao "IPC listening on: [^ ]*" target/niri-aninhado/niri.log | tail -1 | awk '{print $NF}' | sed 's/\x1b\[[0-9;]*m//g')
env -u DISPLAY NIRI_SOCKET="$NS" HOME="$T/srv" XDG_CONFIG_HOME="$T/srv/config" KALANGO_REMOTE_INSTANCE=permissao$$ target/debug/kalango-remote-core serve --no-tunnel < "$T/entrada" > "$T/serve.out" 2> "$T/serve.err" &
SP=$!
exec 3> "$T/entrada"
for _ in $(seq 40); do grep -q '"listening"' "$T/serve.out" && break; sleep 0.5; done
grep -q '"listening"' "$T/serve.out" || { tail -5 "$T/serve.err"; falhou "o núcleo não começou"; }
L=$(python3 -c "import json; d=[json.loads(l) for l in open('$T/serve.out') if l.startswith('{')][0]; print('https://%s/#k=%s.%s %s'%(d['addr'],d['id'],d['pk'],d['password']))")
HOME="$T/cli" XDG_CONFIG_HOME="$T/cli/config" KALANGO_REMOTE_INSTANCE=permissaocli$$ timeout 40 target/debug/kalango-remote-core probe "${L% *}" --password "${L#* }" --frames 3 --timeout 25 > "$T/probe.out" 2>&1
R2=$?
exec 3>&-; kill $SP 2>/dev/null; wait $SP 2>/dev/null
grep -q '"first_frame"' "$T/probe.out" && falhou "o núcleo capturou sem aprovação"
grep -ao "no zwlr_screencopy_manager_v1[^\"]*" "$T/serve.err" | head -1 | sed 's/^/   ok: /'
[ $R2 -ne 0 ] || falhou "o probe não devia ter recebido quadros"

echo "3. pelo app, com aprovação"
unset KALANGO_CORE_TEXT
KALANGOD_SOCKET="$T/mcp.sock" kalangoos/scripts/testar-app.sh > "$T/app.out" 2>&1
tail -3 "$T/app.out" | sed 's/^/   /'
grep -q "^OK" "$T/app.out" || falhou "o fluxo pela interface"
python3 - "$T/estado" <<'PY' || falhou "a linha do tempo do kalangod"
import json, glob, sys
linhas = [json.loads(l) for f in glob.glob(sys.argv[1] + "/*.jsonl") for l in open(f)]
p = [l for l in linhas if l["tool"].startswith("controle_remoto")]
for l in p:
    print("   kalangod:", l["tool"], l["status"], l["summary"])
assert any(l["tool"] == "controle_remoto.permitir" and l["status"] == "ok" for l in p)
PY
grep -a "remote control: pid" target/niri-aninhado/niri.log | sed 's/\x1b\[[0-9;]*m//g' | awk '{$1=""; print "   niri:" $0}' | sort -u | tail -6
grep -aq "kalango-remote-core) Screencopy → true" target/niri-aninhado/niri.log || falhou "o niri não deu a captura ao núcleo"
grep -q "^Esc Esc encerrou" "$T/app.out" || falhou "Esc Esc"
echo OK
