# Phase Audio v0/v1 — Opus RTP transport

Branch: `main` (v1.1 merged); v1.2 in `phase-audio/v1.2-persistent-sync`

## Goal

Add **optional Opus audio** as a parallel RTP session (see [spec/std/zerocast-protocol-v1.md](spec/std/zerocast-protocol-v1.md)).

Vertical slices:
- **v0:** sender emits sine tone → receiver logs `audio rtp bytes=…`
- **v1:** mic or tone → decode + cpal playback; A/V skew logging; discover `a_port`

## Spec

Normative: [docs/spec/std/zerocast-protocol-v1.md](spec/std/zerocast-protocol-v1.md) §5–6.

Constants: `crates/protocol`.

## Scope

| Item | Status |
|------|--------|
| `crates/protocol` | Shared ports, PT, mDNS keys |
| `crates/audio` | Sine PCM, Opus encode/decode (ffmpeg) |
| Video + audio RTP in `stream --audio` | Done |
| `recv-log --audio` dual bind | Done |
| mDNS TXT `audio=1`, `a_port` | Done |
| `stream --discover` uses TXT `a_port` | Done (v1) |
| cpal mic capture | Feature `audio-io` / `--test-tone` fallback |
| Opus decode + cpal playback | `--audio-play` + `audio-io` feature |
| A/V drift correction | Log skew every 30 video frames (v1) |

## Build deps

```bash
# ffmpeg with libopus (already required for video)
ffmpeg -encoders 2>/dev/null | grep libopus

# Mic + speaker (Linux):
sudo apt-get install -y libasound2-dev
cargo build -p zerocast_desktop --features audio-io
```

## Smoke test

```bash
# Terminal 1 — log only
cargo run -p zerocast_desktop -- recv-log 0.0.0.0:5000 --audio

# Terminal 2 — test tone (reliable without mic)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 127.0.0.1:5000 --profile low --audio --test-tone --frames 11
```

**Playback** (requires `audio-io` + ALSA/Pulse):

```bash
cargo run -p zerocast_desktop --features audio-io -- recv-log 0.0.0.0:5000 --audio --audio-play
cargo run -p zerocast_desktop --features audio-io -- stream 0.0.0.0:0 127.0.0.1:5000 --profile low --audio --test-tone --frames 60
```

Expect audible 440 Hz tone and `av-sync: … skew_ms=…` lines.

## QEMU ARM lab

Forward UDP 5002/5003 in user-net mode; bridged `virbr0` passes audio like video.

## Next (v1.2)

Branch: `phase-audio/v1.2-persistent-sync`

| Item | Status |
|------|--------|
| Persistent Opus encode/decode (`opus-rs`, raw RTP) | Done |
| ffmpeg Ogg fallback (`ffmpeg-opus` feature) | Done |
| Audio RTCP SR receive (port A+1) | Done |
| RTCP cross-correlation skew logging | Done |
| Recv playout buffer (60 ms, RTP-scheduled) | Done (`audio-playback`) |
| Require TXT `audio=1` when sender uses `--audio --discover` | TODO |
| cpal resample non–48 kHz devices | TODO |
