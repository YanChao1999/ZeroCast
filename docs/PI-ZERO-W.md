# Raspberry Pi Zero W — embedded receiver

Target: **Pi Zero W** (ARM1176, 512 MB RAM, 2.4 GHz Wi‑Fi) running headless `recv-log`.

## Recommended profile

```bash
zerocast_desktop recv-log 0.0.0.0:5000 --profile low
```

This advertises via mDNS:

- **426×240 @ 15 fps**
- `class=embedded`
- RTCP QoS feedback to the sender (loss, jitter, decode lag)

Desktop sender:

```bash
zerocast_desktop stream 0.0.0.0:0 --discover --profile auto
```

QoS closed loop will step down if Wi‑Fi loss or decode lag is high.

## Cross-build (aarch64 — Pi 3/4/5 class)

```bash
./scripts/cross-build-aarch64.sh
# binary: target/aarch64-unknown-linux-gnu/release/zerocast_desktop
```

Copy to the Pi and install `ffmpeg` on the device.

## Pi Zero W (armv6)

Pi Zero W needs **`armv6-unknown-linux-gnueabihf`**. Use a native build on the device (slow but reliable):

```bash
# On the Pi (Raspberry Pi OS 32-bit)
sudo apt-get install -y ffmpeg rustc cargo gcc
git clone https://github.com/YanChao1999/ZeroCast.git
cd ZeroCast
cargo build -p zerocast_desktop --release --no-default-features
```

Run headless:

```bash
./target/release/zerocast_desktop recv-log 0.0.0.0:5000 --profile low
```

Optional cross-build from x86 (experimental):

```bash
./scripts/cross-build-armv6.sh
```

Requires `gcc-arm-linux-gnueabihf` and Rust target `armv6-unknown-linux-gnueabihf`.

## QEMU lab (aarch64)

For automated testing without hardware, use the QEMU ARM recv lab:

```bash
./scripts/qemu-arm-recv.sh build
./scripts/qemu-arm-recv.sh vm-start
./scripts/qemu-arm-recv.sh deploy
./scripts/qemu-arm-recv.sh recv start --log
```

See [QEMU-ARM-RECV.md](QEMU-ARM-RECV.md).

## systemd service (headless)

```ini
[Unit]
Description=ZeroCast receiver
After=network-online.target

[Service]
ExecStart=/usr/local/bin/zerocast_desktop recv-log 0.0.0.0:5000 --profile low
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

## Related

- [PHASE-QOS.md](PHASE-QOS.md) — QoS closed loop + embedded rules
- [PHASE-2A.md](PHASE-2A.md) — mDNS discovery
