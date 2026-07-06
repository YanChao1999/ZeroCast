# ARM QEMU — embedded receiver lab

Validate `zerocast_desktop` as an **aarch64 Linux recv** before real Pi hardware. The host stays x86 (fast sender); the VM plays the edge receiver.

## What this tests

| Validates | Does not simulate |
|-----------|-------------------|
| Cross-built ARM64 binary runs | Pi Zero W (armv6) performance |
| `recv` / `recv-log` on Linux ARM | 2.4 GHz Wi‑Fi loss/jitter |
| mDNS publish + `stream --discover` (bridged LAN) | Hardware decode (V4L2) |
| Negotiation → 426×240 @ 15 with `class=embedded` | |

## Prerequisites (host)

Ubuntu/Debian:

```bash
./scripts/qemu-arm-recv.sh deps-host
sudo apt-get install -y $(./scripts/qemu-arm-recv.sh deps-host)

# Cross-link libraries for aarch64 minifb (one-time) — only if you cross-build **with**
# the `display` feature (`cargo build ... --features display`). Default cross-build is
# headless (`recv-log` only) and does not need these:
#
# sudo dpkg --add-architecture arm64
# sudo apt-get update
# sudo apt-get install -y $(./scripts/qemu-arm-recv.sh deps-host-cross)
#
# If apt fails (404 / mirror errors), use the default headless cross-build instead.
```

Also need **Rust** on the host (`cargo`). If `cargo` is missing or rustup downloads time out:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# Mirror (if static.rust-lang.org / crates.io are slow)
export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
rustup toolchain install stable
rustup default stable
```

If `cargo build` fails on crates.io, uncomment the `[source.tuna]` block in [`.cargo/config.toml`](../.cargo/config.toml).

For **mDNS discover** tests, use a bridged network (`virbr0` from libvirt). User-mode QEMU networking (`-netdev user`) forwards UDP ports but **does not** carry mDNS multicast.

Optional (bridged mode):

```bash
sudo virsh net-start default
sudo virsh net-autostart default
# Allow QEMU to attach to virbr0 (once):
sudo mkdir -p /etc/qemu
echo 'allow virbr0' | sudo tee /etc/qemu/bridge.conf
# setuid bit (often already set by the qemu package):
sudo chmod u+s /usr/lib/qemu/qemu-bridge-helper 2>/dev/null || true
```

## Quick start

```bash
# 1. Cross-build recv binary for aarch64 Linux
./scripts/cross-build-aarch64.sh

# 2. Create cloud image + start VM (first run downloads ~700 MB)
./scripts/qemu-arm-recv.sh vm-start

# 3. Wait for SSH, deploy binary, install VM deps
./scripts/qemu-arm-recv.sh deploy
./scripts/qemu-arm-recv.sh vm-setup

# 4a. RTP smoke (manual IP — works with any network mode)
./scripts/qemu-arm-recv.sh recv start --log   # terminal 1 (SSH → VM)
./scripts/qemu-arm-recv.sh stream-manual      # terminal 2 (host)

# 4b. Full discover smoke (bridged virbr0 only)
./scripts/qemu-arm-recv.sh recv start --log   # VM: mDNS + headless recv
./scripts/qemu-arm-recv.sh stream-discover    # host: --discover --profile auto

# Stop receiver on VM (free port 5000) without stopping QEMU:
./scripts/qemu-arm-recv.sh recv stop
```

Stop the VM:

```bash
./scripts/qemu-arm-recv.sh vm-stop
```

Artifacts live under `.qemu-arm/` (gitignored).

## Commands reference

| Script | Purpose |
|--------|---------|
| `scripts/cross-build-aarch64.sh` | `cargo build --release --target aarch64-unknown-linux-gnu` |
| `scripts/qemu-arm-recv.sh vm-start` | Boot Ubuntu 24.04 arm64 cloud image on `virbr0` (or user net fallback) |
| `scripts/qemu-arm-recv.sh deploy` | `scp` release binary to VM |
| `scripts/qemu-arm-recv.sh recv start --log` | SSH: headless RTP stats + mDNS |
| `scripts/qemu-arm-recv.sh recv start --screen` | SSH: `recv --profile low` under `xvfb-run` |
| `scripts/qemu-arm-recv.sh recv stop` | SSH: stop `zerocast_desktop` on VM |
| `scripts/qemu-arm-recv.sh stream-manual` | Host: stream to VM IP, `--test-cycle` |
| `scripts/qemu-arm-recv.sh stream-discover` | Host: `stream --discover --profile auto` |

Environment overrides:

| Variable | Default | Meaning |
|----------|---------|---------|
| `ZERO_CAST_QEMU_DIR` | `.qemu-arm` | Image, PID, SSH key cache |
| `ZERO_CAST_VM_IP` | auto / `192.168.122.2` | VM address for deploy/SSH |
| `ZERO_CAST_VM_USER` | `zerocast` | Cloud-init user |
| `ZERO_CAST_VM_SSH_PORT` | `2222` | Host port forwarded to VM:22 (user net) |

## Network modes

### Bridged (`virbr0`) — use for `--discover`

```text
Host (192.168.122.1)  ── virbr0 ──  VM (192.168.122.x DHCP)
         stream --discover                    recv --profile low
```

Multicast mDNS (UDP 5353) and RTP (5000/5001) behave like a real LAN segment.

### User networking — RTP only

QEMU maps `localhost:2222 → VM:22` and can forward UDP 5000. Use `stream-manual` with `127.0.0.1:5000`; skip `--discover`.

## Troubleshooting

**SSH refused after `vm-start`:** cloud-init can take 1–3 minutes on first boot. Run `./scripts/qemu-arm-recv.sh vm-wait`.

**`aarch64-linux-gnu-gcc` not found:** install `gcc-aarch64-linux-gnu` (listed in `deps-host`).

**Discover finds nothing:** confirm recv runs on the VM (not the host), both sides on `virbr0`, firewall allows UDP 5353 on `virbr0`:
`sudo ufw allow in on virbr0`.

**Decode errors in VM:** ensure `ffmpeg` is installed (`vm-setup`).

## Related

- [PHASE-QOS.md](PHASE-QOS.md#embedded-receivers-linux-arm--wifi) — Pi / embedded roadmap
- [PHASE-1D.md](PHASE-1D.md) — Pi ARM recv spike (next actionable after LAN test)
