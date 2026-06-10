#!/usr/bin/env bash
# ARM64 QEMU embedded-receiver lab for ZeroCast.
# See docs/QEMU-ARM-RECV.md
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QEMU_DIR="${ZERO_CAST_QEMU_DIR:-$ROOT/.qemu-arm}"
VM_USER="${ZERO_CAST_VM_USER:-zerocast}"
VM_SSH_PORT="${ZERO_CAST_VM_SSH_PORT:-2222}"
VM_IP="${ZERO_CAST_VM_IP:-}"
BINARY="$ROOT/target/aarch64-unknown-linux-gnu/release/zerocast_desktop"
CLOUD_IMG="$QEMU_DIR/ubuntu-24.04-server-cloudimg-arm64.img"
CLOUD_OVERLAY="$QEMU_DIR/ubuntu-24.04-server-cloudimg-arm64.qcow2"
SEED_ISO="$QEMU_DIR/cloud-init-seed.iso"
PID_FILE="$QEMU_DIR/qemu.pid"
SSH_KEY="$QEMU_DIR/id_ed25519"
SSH_PUB="$SSH_KEY.pub"
CLOUD_INIT_SRC="$ROOT/scripts/cloud-init/aarch64-recv-user-data.yaml"
UBUNTU_CLOUD_URL="https://cloud-images.ubuntu.com/releases/noble/release/ubuntu-24.04-server-cloudimg-arm64.img"

usage() {
  sed -n '2,40p' "$ROOT/docs/QEMU-ARM-RECV.md" | head -n 5 || true
  cat <<EOF

Usage: $0 <command>

Commands:
  deps-host          Print apt packages for the host
  deps-host-cross    Print arm64 :arm64 libs for cross-linking minifb
  deps-vm            Print apt packages for the VM (reference)
  build              Run scripts/cross-build-aarch64.sh
  vm-image           Download Ubuntu arm64 cloud image (first run)
  vm-init            Build cloud-init seed ISO
  vm-start           Start QEMU VM (background)
  vm-stop            Stop QEMU VM
  vm-reset           Stop VM and delete disk overlay (fresh cloud-init)
  vm-wait            Wait until SSH is up
  vm-ip              Print best guess VM IP
  deploy             scp binary to VM
  vm-setup           Install runtime deps on VM (usually cloud-init already did)
  recv stop          SSH: stop zerocast_desktop on VM (free port 5000)
  recv start --log   SSH: headless recv-log + mDNS on VM
  recv start --screen  SSH: recv window under xvfb (needs display build)
  stream-manual      Host: stream to VM with --test-cycle
  stream-discover    Host: stream --discover --profile auto
  status             Show VM / binary status

Legacy aliases: recv-log, recv-stop (deprecated)

Env: ZERO_CAST_QEMU_DIR, ZERO_CAST_VM_IP, ZERO_CAST_VM_USER, ZERO_CAST_VM_SSH_PORT
Doc: docs/QEMU-ARM-RECV.md
EOF
}

deps_host() {
  echo "gcc-aarch64-linux-gnu qemu-system-arm qemu-utils cloud-image-utils genisoimage curl ssh qemu-efi-aarch64"
}

deps_host_desktop() {
  echo "libx11-dev libxcb1-dev libxcb-shm0-dev libxcb-randr0-dev"
}

deps_host_cross() {
  echo "libxcb1-dev:arm64 libxcb-shm0-dev:arm64 libxcb-randr0-dev:arm64 libx11-dev:arm64"
}

deps_vm() {
  echo "ffmpeg xvfb libx11-6 libxcb1 libxcb-shm0 libxcb-randr0"
}

ensure_ssh_key() {
  mkdir -p "$QEMU_DIR"
  if [[ ! -f "$SSH_KEY" ]]; then
    ssh-keygen -t ed25519 -N "" -f "$SSH_KEY" -q -C "zerocast-qemu-arm"
  fi
}

ssh_opts() {
  echo -o StrictHostKeyChecking=no -o UserKnownHostsFile="$QEMU_DIR/known_hosts" -o ConnectTimeout=5
}

# Cached address from vm-wait; invalid when switching user ↔ bridge.
vm_ip_cached() {
  if [[ ! -f "$QEMU_DIR/vm.ip" ]]; then
    return 1
  fi
  local cached
  cached="$(tr -d '[:space:]' <"$QEMU_DIR/vm.ip")"
  if [[ -z "$cached" ]]; then
    return 1
  fi
  if [[ "$(network_mode)" == bridge && "$cached" == "127.0.0.1" ]]; then
    return 1
  fi
  echo "$cached"
}

# user netdev → localhost:2222; bridged virbr0 → VM DHCP address:22
ssh_target() {
  if [[ "$(network_mode)" == user ]]; then
    echo "127.0.0.1"
    return
  fi
  if [[ -n "$VM_IP" ]]; then
    echo "$VM_IP"
    return
  fi
  if ip="$(vm_ip_cached)"; then
    echo "$ip"
    return
  fi
  echo "192.168.122.2"
}

ssh_port() {
  if [[ "$(network_mode)" == user ]]; then
    echo "$VM_SSH_PORT"
  else
    echo 22
  fi
}

vm_ssh() {
  ensure_ssh_key
  local host port
  host="$(ssh_target)"
  port="$(ssh_port)"
  ssh -i "$SSH_KEY" -p "$port" $(ssh_opts) "${VM_USER}@${host}" "$@"
}

vm_scp() {
  ensure_ssh_key
  local host port
  host="$(ssh_target)"
  port="$(ssh_port)"
  scp -i "$SSH_KEY" -P "$port" $(ssh_opts) "$@"
}

network_mode() {
  if ip link show virbr0 >/dev/null 2>&1; then
    echo bridge
  else
    echo user
  fi
}

# Likely VM addresses on virbr0 (DHCP/ARP only — no /24 sweep).
bridge_fast_ips() {
  local ip f
  if [[ -n "$VM_IP" ]]; then
    echo "$VM_IP"
  fi
  if ip="$(vm_ip_cached)"; then
    echo "$ip"
  fi
  f="/var/lib/libvirt/dnsmasq/virbr0.status"
  if [[ -r "$f" ]]; then
    grep -oE '"ip-address": "[0-9.]+"' "$f" 2>/dev/null | sed 's/"ip-address": "//;s/"$//'
  fi
  ip -4 neigh show dev virbr0 2>/dev/null | awk '
    $1 ~ /^192\.168\.122\./ && $1 != "192.168.122.1" { print $1 }'
  if command -v virsh >/dev/null 2>&1; then
    virsh net-dhcp-leases default 2>/dev/null | awk '{gsub(/\047/,"",$5); print $5}'
  fi
}

bridge_try_ssh() {
  local host port timeout seen=()
  port="$(ssh_port)"
  timeout=15
  while IFS= read -r host; do
    [[ -z "$host" || "$host" == "192.168.122.1" ]] && continue
    [[ " ${seen[*]} " == *" $host "* ]] && continue
    seen+=("$host")
    if ssh -i "$SSH_KEY" -p "$port" -o ConnectTimeout="$timeout" -o BatchMode=yes $(ssh_opts) \
      "${VM_USER}@${host}" true 2>/dev/null; then
      echo "$host" >"$QEMU_DIR/vm.ip"
      echo "SSH ready (${VM_USER}@${host}:${port})"
      return 0
    fi
  done < <(bridge_fast_ips | awk '!seen[$0]++')
  return 1
}

vm_image() {
  mkdir -p "$QEMU_DIR"
  if [[ ! -f "$CLOUD_IMG" ]]; then
    echo "==> downloading Ubuntu 24.04 arm64 cloud image"
    curl -L --fail -o "$CLOUD_IMG.part" "$UBUNTU_CLOUD_URL"
    mv "$CLOUD_IMG.part" "$CLOUD_IMG"
  fi
  if [[ ! -f "$CLOUD_OVERLAY" ]]; then
    echo "==> creating qcow2 overlay"
    qemu-img create -f qcow2 -b "$CLOUD_IMG" -F qcow2 "$CLOUD_OVERLAY" 8G
  fi
  echo "OK: $CLOUD_OVERLAY"
}

vm_init() {
  ensure_ssh_key
  mkdir -p "$QEMU_DIR"
  local userdata="$QEMU_DIR/user-data"
  sed "s|__SSH_PUBKEY__|$(cat "$SSH_PUB")|" "$CLOUD_INIT_SRC" >"$userdata"
  echo "instance-id: zerocast-recv-$(date +%s)" >"$QEMU_DIR/meta-data"
  cloud-localds "$SEED_ISO" "$userdata" "$QEMU_DIR/meta-data"
  echo "OK: $SEED_ISO"
}

vm_uefi_code() {
  local uefi="${ZERO_CAST_UEFI_CODE:-/usr/share/AAVMF/AAVMF_CODE.fd}"
  if [[ ! -f "$uefi" ]]; then
    uefi="/usr/share/qemu-efi-aarch64/QEMU_EFI.fd"
  fi
  if [[ ! -f "$uefi" ]]; then
    echo "error: ARM UEFI firmware not found (install qemu-efi-aarch64)" >&2
    exit 1
  fi
  echo "$uefi"
}

vm_start() {
  vm_image
  vm_init
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "VM already running (pid $(cat "$PID_FILE"))"
    return 0
  fi

  local mode net_args uefi_code
  uefi_code="$(vm_uefi_code)"
  mode="$(network_mode)"
  echo "==> starting QEMU (network: $mode, UEFI: $uefi_code)"

  if [[ "$mode" == bridge ]]; then
    net_args=(-netdev "bridge,id=net0,br=virbr0" -device virtio-net-pci,netdev=net0)
  else
    echo "note: virbr0 not found - using user networking (no mDNS discover)"
    echo "      start libvirt default net for bridged mode: sudo virsh net-start default"
    net_args=(-netdev "user,id=net0,hostfwd=tcp::${VM_SSH_PORT}-:22,hostfwd=udp::5000-:5000,hostfwd=udp::5001-:5001,hostfwd=udp::5002-:5002,hostfwd=udp::5003-:5003" -device virtio-net-pci,netdev=net0)
  fi

  : >"$QEMU_DIR/qemu.log"
  rm -f "$QEMU_DIR/vm.ip"
  setsid qemu-system-aarch64 \
    -machine virt,highmem=on \
    -cpu cortex-a72 \
    -smp 2 \
    -m 2048 \
    -bios "$uefi_code" \
    -drive "if=virtio,file=$CLOUD_OVERLAY,format=qcow2" \
    -drive "if=virtio,file=$SEED_ISO,format=raw" \
    "${net_args[@]}" \
    -nographic \
    </dev/null >>"$QEMU_DIR/qemu.log" 2>&1 &
  echo $! >"$PID_FILE"
  disown
  echo "VM pid $(cat "$PID_FILE"), log: $QEMU_DIR/qemu.log"
  echo "first boot on emulated ARM often takes 3–5 min — do not Ctrl+C (Ctrl+C kills the VM)"
  if [[ "${ZERO_CAST_VM_NO_WAIT:-}" == 1 ]]; then
    echo "started in background — run: $0 vm-wait"
    return 0
  fi
  vm_wait
}

vm_stop() {
  if [[ -f "$PID_FILE" ]]; then
    local pid
    pid="$(cat "$PID_FILE")"
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" || true
      sleep 1
      kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
    echo "stopped VM (was pid $pid)"
  else
    echo "no pid file - VM not running?"
  fi
}

vm_is_running() {
  [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

require_vm() {
  if ! vm_is_running; then
    echo "error: QEMU VM is not running" >&2
    echo "  start it first: $0 vm-start" >&2
    echo "  check status:   $0 status" >&2
    exit 1
  fi
  if [[ ! -f "$BINARY" ]]; then
    echo "error: ARM binary missing — run: $0 build" >&2
    exit 1
  fi
}

vm_reset() {
  vm_stop
  rm -f "$CLOUD_OVERLAY" "$SEED_ISO" "$QEMU_DIR/vm.ip"
  echo "removed VM overlay (next vm-start = fresh cloud-init)"
}

vm_wait() {
  ensure_ssh_key
  local i host port mode
  mode="$(network_mode)"
  port="$(ssh_port)"

  if [[ "$mode" == bridge ]]; then
    echo "waiting for SSH (${VM_USER}@virbr0:${port}) ..."
    for i in $(seq 1 120); do
      if bridge_try_ssh; then
        return 0
      fi
      if (( i % 5 == 0 )); then
        echo "  still waiting (${i}/120) — emulated ARM boot can take 3–5 min"
        if [[ -r /var/lib/libvirt/dnsmasq/virbr0.status ]]; then
          echo "  dhcp: $(grep -oE '"ip-address": "[0-9.]+"' /var/lib/libvirt/dnsmasq/virbr0.status | head -1 | sed 's/.*"//;s/"$//')"
        fi
        echo "  tip: tail -f $QEMU_DIR/qemu.log  or  ZERO_CAST_VM_IP=192.168.122.x $0 vm-wait"
      fi
      sleep 3
    done
  else
    host="127.0.0.1"
    echo "waiting for SSH (${VM_USER}@${host}:${port}) ..."
    for i in $(seq 1 120); do
      if ssh -i "$SSH_KEY" -p "$port" -o ConnectTimeout=10 $(ssh_opts) "${VM_USER}@${host}" true 2>/dev/null; then
        echo "$host" >"$QEMU_DIR/vm.ip"
        echo "SSH ready (${VM_USER}@${host}:${port})"
        return 0
      fi
      if (( i % 5 == 0 )); then
        echo "  still waiting (${i}/120) — emulated ARM first boot can take 3–5 min"
        echo "  tip: tail -f $QEMU_DIR/qemu.log"
      fi
      sleep 3
    done
  fi
  echo "error: SSH not ready after 360s — see $QEMU_DIR/qemu.log" >&2
  echo "  hint: $0 vm-start   or set ZERO_CAST_VM_SSH_PORT / ZERO_CAST_VM_IP" >&2
  return 1
}

vm_ip() {
  local vm_addr
  if [[ "$(network_mode)" == user ]]; then
    echo "127.0.0.1"
    return
  fi
  if [[ -n "$VM_IP" ]]; then
    echo "$VM_IP"
    return
  fi
  if ip="$(vm_ip_cached)"; then
    echo "$ip"
    return
  fi
  if [[ -r /var/lib/libvirt/dnsmasq/virbr0.status ]]; then
    ip="$(grep -oE '"ip-address": "[0-9.]+"' /var/lib/libvirt/dnsmasq/virbr0.status | head -1 | sed 's/"ip-address": "//;s/"$//')"
    if [[ -n "$ip" ]]; then
      echo "$ip"
      return
    fi
  fi
  vm_addr="$(ip -4 neigh show dev virbr0 2>/dev/null | awk '$1 ~ /^192\.168\.122\./ && $1 != "192.168.122.1" {print $1; exit}')"
  if [[ -n "$vm_addr" ]]; then
    echo "$vm_addr"
    return
  fi
  echo "192.168.122.2"
}

deploy() {
  if [[ ! -f "$BINARY" ]]; then
    echo "error: binary missing — run: $0 build" >&2
    exit 1
  fi
  vm_wait
  ensure_ssh_key
  local dest="/home/${VM_USER}/zerocast_desktop"
  local tmp="${dest}.new"
  # Upload to .new first — scp cannot truncate a running executable (recv-log holds it open).
  vm_scp "$BINARY" "${VM_USER}@$(ssh_target):${tmp}"
  vm_ssh "chmod +x ${tmp} && mv -f ${tmp} ${dest}"
  echo "deployed to VM:${dest}"
}

vm_setup() {
  vm_wait
  if vm_ssh "command -v ffmpeg >/dev/null 2>&1"; then
    echo "OK: VM runtime deps already installed"
    return 0
  fi
  echo "==> installing VM packages (slow on emulated ARM; cloud-init may hold apt first)..."
  vm_ssh "set -e
    if command -v cloud-init >/dev/null 2>&1; then
      echo 'waiting for cloud-init...'
      cloud-init status --wait 2>/dev/null || true
    fi
    echo 'waiting for apt lock...'
    for i in \$(seq 1 60); do
      if sudo fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1 \
        || sudo fuser /var/lib/apt/lists/lock >/dev/null 2>&1; then
        echo \"  apt busy (\$i/60)\"
        sleep 5
      else
        break
      fi
    done
    sudo DEBIAN_FRONTEND=noninteractive apt-get update
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y $(deps_vm)
    command -v ffmpeg"
}

vm_stop_recv() {
  require_vm
  vm_wait
  if vm_ssh "pgrep -f '[/]zerocast_desktop' >/dev/null 2>&1"; then
    vm_ssh "pkill -f '[/]zerocast_desktop' 2>/dev/null || true; sleep 1"
    echo "stopped zerocast_desktop on VM"
  else
    echo "no zerocast_desktop process on VM"
  fi
}

recv_usage() {
  cat <<EOF
Usage: $0 recv stop
       $0 recv start --log
       $0 recv start --screen

  --log     headless RTP stats + mDNS (default cross-build)
  --screen  video window via xvfb (needs display-enabled ARM binary)
EOF
}

recv_stop() {
  vm_stop_recv
}

recv_prepare() {
  require_vm
  echo "==> deploying ARM binary to VM..."
  deploy
  if ! vm_ssh "command -v ffmpeg >/dev/null 2>&1" 2>/dev/null; then
    echo "==> VM missing ffmpeg — run once: $0 vm-setup  (or wait if apt is locked)"
    vm_setup
  fi
  echo "==> stopping any previous recv on VM..."
  vm_stop_recv
}

recv_start_log() {
  recv_prepare
  echo "==> starting recv-log on VM with mDNS (Ctrl+C to stop)"
  echo "    stream: $0 stream-manual  or  $0 stream-discover"
  vm_ssh "/home/${VM_USER}/zerocast_desktop recv-log 0.0.0.0:5000 --profile low --audio"
}

recv_start_screen() {
  recv_prepare
  echo "==> starting recv with video window on VM (xvfb, Ctrl+C to stop)"
  echo "    note: cross-build is headless — rebuild with display for --screen"
  vm_ssh "xvfb-run -a /home/${VM_USER}/zerocast_desktop recv 0.0.0.0:5000 --profile low"
}

recv_start() {
  local mode=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --log) mode=log; shift ;;
      --screen) mode=screen; shift ;;
      -h|--help|help)
        recv_usage
        return 0
        ;;
      *)
        echo "error: unknown recv start option: $1" >&2
        recv_usage >&2
        exit 1
        ;;
    esac
  done
  case "$mode" in
    log) recv_start_log ;;
    screen) recv_start_screen ;;
    *)
      echo "error: recv start requires --log or --screen" >&2
      recv_usage >&2
      exit 1
      ;;
  esac
}

recv_cmd() {
  local sub="${1:-}"
  shift || true
  case "$sub" in
    stop) recv_stop ;;
    start) recv_start "$@" ;;
    -h|--help|help|"") recv_usage ;;
    *)
      echo "error: unknown recv subcommand: ${sub:-}" >&2
      recv_usage >&2
      exit 1
      ;;
  esac
}

stream_manual() {
  local ip target host_bin
  ip="$(vm_ip)"
  if [[ "$(network_mode)" == user ]]; then
    target="127.0.0.1:5000"
  else
    target="${ip}:5000"
  fi
  echo "==> streaming to $target (--profile low --test-cycle)"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found on host for sender" >&2
    exit 1
  fi
  cd "$ROOT"
  # Build headless sender once; invoke binary directly (cargo run eats `--profile` as its own flag).
  cargo build -p zerocast_desktop --no-default-features -q
  host_bin="$ROOT/target/debug/zerocast_desktop"
  "$host_bin" stream 0.0.0.0:0 "$target" --profile low --frames 11 --test-cycle --audio
}

stream_discover() {
  if [[ "$(network_mode)" == user ]]; then
    echo "error: stream-discover needs bridged virbr0 (mDNS multicast)" >&2
    echo "  sudo virsh net-start default" >&2
    exit 1
  fi
  require_vm
  vm_wait
  if ! vm_ssh "pgrep -f '[/]zerocast_desktop' >/dev/null 2>&1"; then
    echo "error: no receiver on VM — run in another terminal: $0 recv start --log" >&2
    exit 1
  fi
  if ! vm_ssh "ss -H -ulnp 2>/dev/null | grep -q ':5353'"; then
    echo "error: VM recv is running but mDNS (UDP 5353) is not active" >&2
    echo "  deploy does not restart a running recv — old binary may lack mDNS" >&2
    echo "  fix: $0 recv stop && $0 recv start --log" >&2
    exit 1
  fi
  echo "==> VM recv + mDNS OK; browsing on virbr0 (15s)..."
  export ZERO_CAST_BROWSE_TIMEOUT="${ZERO_CAST_BROWSE_TIMEOUT:-15}"
  cd "$ROOT"
  cargo build -p zerocast_desktop --no-default-features -q
  "$ROOT/target/debug/zerocast_desktop" stream 0.0.0.0:0 --discover --profile auto --frames 11 --test-cycle
}

status_cmd() {
  echo "QEMU dir: $QEMU_DIR"
  echo "Network:  $(network_mode)"
  if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    echo "VM:       running (pid $(cat "$PID_FILE"))"
  else
    echo "VM:       stopped"
  fi
  if [[ -f "$BINARY" ]]; then
    echo "Binary:   $BINARY"
    file "$BINARY" 2>/dev/null || true
  else
    echo "Binary:   missing (run: $0 build)"
  fi
  echo "VM IP:    $(vm_ip 2>/dev/null || echo unknown)"
}

cmd="${1:-}"
case "$cmd" in
  deps-host) deps_host ;;
  deps-host-cross) deps_host_cross ;;
  deps-host-desktop) deps_host_desktop ;;
  deps-vm) deps_vm ;;
  build) "$ROOT/scripts/cross-build-aarch64.sh" ;;
  vm-image) vm_image ;;
  vm-init) vm_init ;;
  vm-start) vm_start ;;
  vm-stop) vm_stop ;;
  vm-reset) vm_reset ;;
  vm-wait) vm_wait ;;
  vm-ip) vm_ip ;;
  deploy) deploy ;;
  vm-setup) vm_setup ;;
  recv)
    shift
    recv_cmd "$@"
    ;;
  recv-log)
    echo "note: recv-log is deprecated — use: $0 recv start --log" >&2
    recv_start_log
    ;;
  recv-stop)
    echo "note: recv-stop is deprecated — use: $0 recv stop" >&2
    recv_stop
    ;;
  stream-manual) stream_manual ;;
  stream-discover) stream_discover ;;
  status) status_cmd ;;
  -h|--help|help|"") usage ;;
  *)
    echo "unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac
