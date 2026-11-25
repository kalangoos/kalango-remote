#!/bin/bash
# Kalango Remote · liga duas VMs da ISO do KalangoOS lado a lado, como duas máquinas separadas, para testar o Remote
# de uma para a outra sem hardware: cada VM tem
#   - uma placa "internet" (NAT do QEMU: saída para a internet, sem entrada) e
#   - uma placa "LAN" ligada à outra VM por um socket (um cabo virtual de camada 2): 192.168.77.1 (A) e .2 (B)
# Sem root. Uso: duas-vms.sh <pasta de trabalho> <iso> [memória MB]   (cria <pasta>/a e <pasta>/b)
# Console serial de cada uma: socat - UNIX-CONNECT:<pasta>/a/serial.sock ; monitor: <pasta>/a/monitor.sock
set -euo pipefail
W=${1:?pasta de trabalho}; ISO=${2:?ISO}; MEM=${3:-4096}
OVMF_CODE=/usr/share/OVMF/OVMF_CODE_4M.fd; OVMF_VARS=/usr/share/OVMF/OVMF_VARS_4M.fd
PORTA=${PORTA_CABO:-47311}
ligar() {  # ligar nome papel número  (ssh da A em 127.0.0.1:2201 e da B em :2202; QMP em <pasta>/x/qmp.sock)
  local n=$1 papel=$2 mac=$3 d="$W/$1"
  mkdir -p "$d"; cp -f "$OVMF_VARS" "$d/uefi-vars.fd"; rm -f "$d/serial.sock" "$d/monitor.sock" "$d/qmp.sock"
  qemu-system-x86_64 -name "kalango-$n" -enable-kvm -cpu host -machine q35 -m "$MEM" -smp 3 \
    -drive "if=pflash,format=raw,readonly=on,file=$OVMF_CODE" -drive "if=pflash,format=raw,file=$d/uefi-vars.fd" \
    -drive "file=$ISO,media=cdrom,readonly=on,if=none,id=cd" -device ide-cd,drive=cd,bootindex=1 \
    -device virtio-vga -display none -device virtio-keyboard-pci -device virtio-tablet-pci \
    -nic "user,model=virtio-net-pci,mac=52:54:00:aa:00:0${mac},hostfwd=tcp:127.0.0.1:220${mac}-:22" \
    -netdev "socket,id=lan,$papel=127.0.0.1:$PORTA" -device "virtio-net-pci,netdev=lan,mac=52:54:00:bb:00:0${mac}" \
    -chardev socket,id=s0,path="$d/serial.sock",server=on,wait=off,logfile="$d/console.log" -serial chardev:s0 \
    -monitor unix:"$d/monitor.sock",server,nowait -qmp unix:"$d/qmp.sock",server,nowait -daemonize -pidfile "$d/qemu.pid"
}
ligar a listen 1
sleep 1
ligar b connect 2
echo "VMs ligadas: $W/a e $W/b (pids em qemu.pid)"
