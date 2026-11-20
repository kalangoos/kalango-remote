#!/bin/bash
# Kalango Remote · bibliotecas de desenvolvimento que o núcleo do RustDesk precisa (codecs, PAM, xcb, áudio),
# baixadas do Ubuntu e extraídas em kalangoos/.deps, sem sudo. Depois:
#   source kalangoos/scripts/env.sh && cargo build -p kalango-remote --features linux-pkg-config
set -eu
cd "$(dirname "$0")/.."
ROOT=$PWD/.deps/root
mkdir -p .deps/debs "$ROOT"
pkgs="libvpx-dev libaom-dev libopus-dev libyuv-dev libyuv0 libpam0g-dev libxcb-randr0-dev libxcb-shape0-dev
      libxcb-xfixes0-dev libasound2-dev libpulse-dev libxdo-dev libva-dev"
need=""
for p in $pkgs; do
  dpkg -s "$p" >/dev/null 2>&1 && continue   # já instalado no sistema
  ls .deps/debs/"${p}"_*.deb >/dev/null 2>&1 || need="$need $p"
done
# as bibliotecas de execução que o sistema não tem vêm junto (ex.: libvpx9 sem o -dev)
for p in $pkgs; do
  for dep in $(apt-cache depends --no-recommends --no-suggests "$p" 2>/dev/null | awk '/Depends:/{print $2}' | grep -v '[<>|]'); do
    case "$dep" in lib*) dpkg -s "$dep" >/dev/null 2>&1 || ls .deps/debs/"${dep}"_*.deb >/dev/null 2>&1 || need="$need $dep";; esac
  done
done
need=$(echo $need | tr " " "\n" | sort -u | xargs)
[ -n "$need" ] && (cd .deps/debs && apt-get download $need)
for f in .deps/debs/*.deb; do dpkg -x "$f" "$ROOT"; done
# os .pc apontam para /usr: aponta para a pasta local
for pc in "$ROOT"/usr/lib/x86_64-linux-gnu/pkgconfig/*.pc "$ROOT"/usr/share/pkgconfig/*.pc; do
  [ -f "$pc" ] && sed -i "s#^prefix=/usr\$#prefix=$ROOT/usr#" "$pc"
done
# links .so quebrados: o alvo (a biblioteca de execução) já está no sistema
for l in "$ROOT"/usr/lib/x86_64-linux-gnu/*.so; do
  if [ -L "$l" ] && [ ! -e "$l" ]; then
    alvo=$(basename "$(readlink "$l")")
    [ -e "/usr/lib/x86_64-linux-gnu/$alvo" ] && ln -sf "/usr/lib/x86_64-linux-gnu/$alvo" "$l"
  fi
done
# o libyuv do Ubuntu não traz .pc: escreve um
cat > "$ROOT/usr/lib/x86_64-linux-gnu/pkgconfig/libyuv.pc" <<PC
prefix=$ROOT/usr
libdir=\${prefix}/lib/x86_64-linux-gnu
includedir=\${prefix}/include
Name: libyuv
Description: YUV conversion (Ubuntu libyuv-dev)
Version: 0.0
Libs: -L\${libdir} -lyuv
Cflags: -I\${includedir}
PC
echo "dependências em $ROOT"
