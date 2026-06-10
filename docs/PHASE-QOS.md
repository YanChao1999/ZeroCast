# Phase QoS — adaptive streaming

Branch: `phase-qos/v1-foundation` (in progress)

## Goal

Start from the **primary monitor capability** (e.g. 1920×1080 @ 60 Hz) on the **sender**, but never exceed what the **receiver** and **network** can sustain. Stream as high as the path allows, and **step down automatically** when encode, decode, or Wi‑Fi metrics are bad.

The current **426×240 @ 15 fps** path is the **lowest ladder rung** — and the **default ceiling** for weak edge receivers (e.g. Raspberry Pi Zero W).

## Architecture

```text
Sender: display detect (WxH, Hz)     → sender ceiling
Receiver: device class + decode cap → recv ceiling (mDNS TXT)
Network: Wi‑Fi / loss / jitter       → effective ceiling
    → negotiated profile = min(sender, recv, network)
    → Profile ladder (1080p60 … 426p15)
    → Encoder registry (HW → ffmpeg pipe → one-shot SW)
    → RTP + RTCP metrics
    → QoS controller (upgrade / downgrade / fallback)
```

Cross-platform encoders (future `crates/media` or `transport` backends):

| Backend | Windows | macOS | Linux |
|---------|---------|-------|-------|
| NVENC / AMF / QSV | ✓ | — | QSV/VAAPI |
| VideoToolbox | — | ✓ | — |
| ffmpeg hwaccel pipe | ✓ | ✓ | ✓ |
| libx264 one-shot | fallback | fallback | fallback |

## Metrics (sender, Phase 1b+)

- Encode ms per frame (have)
- Actual fps vs target (have)
- Estimated video kbps (have)
- RTCP RR loss / jitter (partial)
- Receiver decode lag (TODO)
- UDP send drops (TODO)

## Profile ladder (initial)

| Rung | Resolution | FPS | Use |
|------|------------|-----|-----|
| High | min(display, 1920×1080) | min(display Hz, 60) | Good LAN + HW encode |
| Med | 1280×720 | 30 | Moderate load |
| Low | 426×240 | 15 | MVP, **Pi Zero W default**, bad Wi‑Fi |

QoS steps **down** one rung on sustained bad metrics; steps **up** slowly when stable. The **effective ceiling** is `min(sender auto profile, recv advertised max, network budget)`.

## Embedded receivers (Linux ARM / Wi‑Fi)

Target example: **Raspberry Pi Zero W** (ARM1176, 512 MB RAM, 2.4 GHz Wi‑Fi only) running `recv` headless or with a small display.

| Constraint | Implication |
|------------|-------------|
| **CPU** | Software H.264 decode at 1080p60 is not realistic; cap recv at **426×240–720p** |
| **RAM** | Limit framebuffer + decode buffers; prefer **recv-log** / headless on Pi |
| **Wi‑Fi** | 2.4 GHz, higher loss/jitter → QoS must downgrade aggressively; optional pacing |
| **No GPU encode/decode** | Sender uses desktop HW; recv uses **ffmpeg/libav SW decode** or future V4L2 M2M on Pi 4+ |
| **Cross-build** | `aarch64`/`armv6` Linux binary; avoid heavy deps on Zero W |

**Design rules**

1. **`recv` publishes recv capability**, not just listen port — extend mDNS TXT:
   - `max_w`, `max_h`, `max_fps` (hard decode ceiling)
   - `dev=pi0w` or `class=embedded` (optional hint for sender defaults)
2. **`stream --discover`** picks `min(sender profile, recv TXT)` — not sender 1080p60 into a Pi Zero.
3. **Default Pi Zero W profile**: `426×240 @ 15` (today’s MVP); QoS may drop fps before resolution on Wi‑Fi.
4. **Headless recv** on embedded: `recv-log` or framebuffer KMS later; minifb optional.
5. **Sender on desktop** remains high-capability; adaptation is asymmetric by design.

```text
Desktop sender (1080p60 capable)
        │  RTP over Wi‑Fi
        ▼
Pi Zero W recv (426×240 @ 15 capable)
        → decode + HDMI / SPI display
```

## TODO checklist

### Phase 1c — profiles + display (in progress)

- [x] `PrimaryDisplay` detection (scrap primary monitor)
- [x] `StreamProfile` + ladder in `zerocast_core`
- [x] CLI `--profile low|med|high|auto` on `stream`
- [x] Saner one-shot GOP (`-g fps`, SPS/PPS on IDR only)
- [ ] ffmpeg hwaccel probe (NVENC / QSV / VideoToolbox)
- [ ] Publish/display max capability in mDNS TXT (`max_w`, `max_h`, `max_fps`)
- [ ] Wire discover: `ReceiverCapability` + `min(sender, recv)` (type in core)

### Embedded recv (Linux ARM)

- [ ] Document Pi Zero W build (`armv6-unknown-linux-gnueabihf` / `aarch64`)
- [x] **aarch64 QEMU recv lab** — [docs/QEMU-ARM-RECV.md](QEMU-ARM-RECV.md) + `scripts/qemu-arm-recv.sh`
- [ ] Headless `recv` service mode (no minifb) for edge deploy
- [x] mDNS TXT `class=embedded` on `recv --profile low` (+ parse in discover)
- [ ] Recv-side decode lag metric → RTCP or feedback channel for QoS
- [ ] Optional: Pi 4+ V4L2 `h264_v4l2m2m` decode → dmabuf → **DRM/KMS HDMI**
- [ ] Pi Zero W: minimal-copy NV12 → KMS scale at 426×240 (no full RGB24 path)
- [ ] Wi‑Fi aware defaults (start low on `wlan0`, slow ramp-up)

### Phase QoS v1 — closed loop

**Status: v1 foundation shipped (closed loop partial).** Discover negotiation + sender hot reconfigure on ladder change work; RTCP/recv lag feedback and recv-side resize are not done yet.

| Done (v1 foundation) | Not done (v1 closed loop) |
|----------------------|---------------------------|
| `QosController` + hysteresis | RTCP loss / recv decode lag inputs |
| `class=embedded` mDNS TXT | Dynamic mDNS re-advertise on profile change |
| Negotiated session floor (no downgrade below discover caps) | |
| Warmup windows; fps ratio ignored at session rung | |
| Hot reconfigure capture + encoder on ladder change | |
| Same-PC registry caps for direct `stream`; decoder RGB size guard (no red shear) | |

- [x] `QosController` module (metrics window, hysteresis) in `zerocast_core`
- [x] Downgrade hints on: low fps ratio, encode_ms over frame budget (sender stats window)
- [x] Upgrade hints after stable window
- [x] Session floor: no downgrade below negotiated recv caps
- [x] Reconfigure encoder + capture scale without restart (`stream_loop` applies ladder rung)
- [ ] Downgrade on: RTCP loss, recv decode lag
- [ ] Dynamic mDNS re-advertise or control channel for profile change

**CI:** `.github/workflows/ci.yml` — `cargo test --workspace`, `ffmpeg_integration` (720p IDR size + decode roundtrip), `zerocast_core` QoS unit tests.

### Phase QoS v2 — network polish

- [ ] RTP pacing / token bucket
- [ ] Optional DSCP (`AF41`) on LAN
- [ ] FEC or NACK policy (optional)

### Phase 2c — Zero-copy GPU / HDMI path

Today (many copies):

```text
SENDER   scrap BGRA → CPU RGB24 scale → ffmpeg libx264 → RTP
RECV     RTP → Annex-B → ffmpeg decode → CPU RGB24 → minifb blit
```

Target (tier `CopyTier::ZeroCopy` / `HwAccel`):

```text
SENDER   GPU capture (DXGI/SCKit/PipeWire texture)
           → HW encoder (NVENC/QSV/VT/VAAPI) → RTP
RECV     RTP → HW decode (dmabuf / NV12)
           → DRM/KMS plane → HDMI/DSI   (Linux embedded)
           → GPU texture → wgpu/GL      (desktop / Pi 4+)
```

| Stage | Today | Zero-copy target | Platform notes |
|-------|--------|------------------|----------------|
| Capture | CPU RGB24 | GPU texture / dmabuf | DXGI, ScreenCaptureKit, PipeWire |
| Encode | ffmpeg CPU | NVENC, QSV, VT, VAAPI | Same subprocess or native API |
| Network | RTP UDP | RTP (+ pacing for Wi‑Fi) | Unchanged |
| Decode | ffmpeg → RGB24 | V4L2 M2M, VA-API, VT, MediaCodec | Pi 4+: `h264_v4l2m2m`; Zero W: SW only |
| Display | minifb CPU | **DRM/KMS direct**, HDMI | No full-frame RGB on Pi/desktop |

**Receiver output modes** (`ReceiverOutput` in `zerocast_core`):

| Mode | Use | Copy tier |
|------|-----|-----------|
| `CpuWindow` | Desktop dev (minifb) | `FullCpu` — current MVP |
| `DrmKms` | Pi / embedded HDMI dongle | `HwAccel` — decode → KMS framebuffer |
| `GpuTexture` | Future egui/wgpu UI | `HwAccel` → `ZeroCopy` |

**Pi Zero W note:** No practical full zero-copy at 720p+; aim for **HwAccel** at 426×240 (V4L2 where available) or minimal-copy **NV12 → KMS scale**. QoS keeps bitrate low so Wi‑Fi and decode keep up.

**TODO — zero-copy**

- [ ] `VideoDecoder` trait + HW backends (symmetric to `VideoEncoder`)
- [ ] Linux recv: `DrmKmsDisplay` backend (replace minifb on embedded)
- [ ] Pi 4+: V4L2 `h264_v4l2m2m` decode → dmabuf → KMS
- [ ] Desktop recv: VA-API / D3D11VA decode → wgpu texture
- [ ] Sender: DXGI texture → NVENC (no RGB24 middle buffer)
- [ ] Advertise `out=drm` / `tier=hw` in mDNS TXT for discover
- [ ] Metric: `copy_tier` + decode_ms in recv stats → QoS

Legacy section (sender capture only):

- [ ] DXGI / ScreenCaptureKit texture → HW encoder (zero-copy)
- [ ] Remove CPU RGB24 scale from hot path at high rungs

### Mobile

- [ ] Same ladder + metrics on Android (MediaCodec) / iOS (VideoToolbox)

## Usage today (1c)

```powershell
# Auto-pick stream size from primary display (capped at 1080p60)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 192.168.1.10:5000 --profile auto

# Fixed ladder rung
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile med
```

Discover uses recv TXT dimensions unless `--profile` overrides on the sender.

**Embedded recv (future):** Pi Zero W should advertise e.g. `426×240 @ 15`; desktop sender must not exceed that when using `--discover` without override.

```bash
# Edge receiver (Linux ARM, same LAN)
zerocast_desktop recv 0.0.0.0:5000 426 240 15

# Desktop sender — profile capped by recv + QoS
zerocast_desktop stream 0.0.0.0:0 --discover --profile auto
```

## Related

- [PHASE-1B.md](PHASE-1B.md) — encoder pipe, sender stats
- [PHASE-2A.md](PHASE-2A.md) — mDNS discovery
