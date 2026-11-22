# Kalango Remote · variáveis para compilar com as bibliotecas de kalangoos/.deps (ver deps.sh). Use com `source`.
_KR_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)/.deps/root"
export PKG_CONFIG_PATH="$_KR_ROOT/usr/lib/x86_64-linux-gnu/pkgconfig:$_KR_ROOT/usr/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LIBRARY_PATH="$_KR_ROOT/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$_KR_ROOT/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export C_INCLUDE_PATH="$_KR_ROOT/usr/include${C_INCLUDE_PATH:+:$C_INCLUDE_PATH}"
export CPLUS_INCLUDE_PATH="$_KR_ROOT/usr/include${CPLUS_INCLUDE_PATH:+:$CPLUS_INCLUDE_PATH}"
export BINDGEN_EXTRA_CLANG_ARGS="-I$_KR_ROOT/usr/include"
# o núcleo do RustDesk é grande: sem incremental e sem símbolos de depuração o target/ cabe em ~5 GB
export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
# gtk4-layer-shell (o contorno e o aviso "está controlando" por cima da tela): o KalangoOS traz; aqui vem da
# cópia compilada pelo KalangoOS em SDRustOS/poc/.deps, quando ela existe ao lado deste repositório
_KR_LS="${KALANGO_DEPS:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../../.." && pwd)/SDRustOS/poc/.deps}/root/usr/lib/x86_64-linux-gnu"
if [ -f "$_KR_LS/pkgconfig/gtk4-layer-shell-0.pc" ]; then
  export PKG_CONFIG_PATH="$PKG_CONFIG_PATH:$_KR_LS/pkgconfig" LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$_KR_LS"
fi
