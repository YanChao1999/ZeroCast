# Phase QoS — adaptive streaming

Branch: TBD (`phase-qos/adaptive` after 1c media backends)

## Goal

Start from the **primary monitor capability** (e.g. 1920×1080 @ 60 Hz), stream as high as the path allows, and **step down automatically** when encode or network/recvr metrics are bad. The current **426×240 @ 15 fps** ffmpeg path is the **lowest ladder rung**, not the product default.

## Architecture

```text
Display detect (WxH, Hz)
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
| Low | 426×240 | 15 | MVP / emergency fallback |

QoS steps **down** one rung on sustained bad metrics; steps **up** slowly when stable.

## TODO checklist

### Phase 1c — profiles + display (in progress)

- [x] `PrimaryDisplay` detection (scrap primary monitor)
- [x] `StreamProfile` + ladder in `zerocast_core`
- [x] CLI `--profile low|med|high|auto` on `stream`
- [x] Saner one-shot GOP (`-g fps`, SPS/PPS on IDR only)
- [ ] ffmpeg hwaccel probe (NVENC / QSV / VideoToolbox)
- [ ] Publish/display max capability in mDNS TXT (`max_w`, `max_h`, `max_fps`)

### Phase QoS v1 — closed loop

- [ ] `QosController` module (metrics window, hysteresis)
- [ ] Downgrade on: encode_ms > budget, loss > threshold, recv lag
- [ ] Upgrade after stable window
- [ ] Reconfigure encoder + capture scale without restart
- [ ] Dynamic mDNS re-advertise or control channel for profile change

### Phase QoS v2 — network polish

- [ ] RTP pacing / token bucket
- [ ] Optional DSCP (`AF41`) on LAN
- [ ] FEC or NACK policy (optional)

### Phase 2c — GPU path

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

Discover still uses recv TXT dimensions unless `--profile` overrides (override TODO).

## Related

- [PHASE-1B.md](PHASE-1B.md) — encoder pipe, sender stats
- [PHASE-2A.md](PHASE-2A.md) — mDNS discovery
