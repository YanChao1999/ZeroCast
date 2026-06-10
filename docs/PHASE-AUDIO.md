# Phase Audio v0 — Opus RTP transport

Branch: `phase-audio/v0-transport` (proposed)

## Goal

Add **optional Opus audio** as a parallel RTP session (see [spec/std/zerocast-protocol-v1.md](spec/std/zerocast-protocol-v1.md)).

First vertical slice: sender emits audio (sine tone or mic) → receiver logs `audio rtp bytes=…`.

## Spec

Normative: [docs/spec/std/zerocast-protocol-v1.md](spec/std/zerocast-protocol-v1.md) §5–6.

Constants: `crates/protocol`.

## Scope (v0)

| Item | Status |
|------|--------|
| `crates/protocol` | Shared ports, PT, mDNS keys |
| `crates/audio` | Sine PCM source, Opus encode |
| Video + audio RTP in `stream --audio` | |
| `recv-log --audio` dual bind | |
| mDNS TXT `audio=1`, `a_port` | |
| cpal mic capture | Optional feature `capture` |
| Playback on recv | Deferred |
| A/V drift correction | Log only |

## Build deps

```bash
# ffmpeg with libopus (already required for video)
ffmpeg -encoders 2>/dev/null | grep libopus

# Optional mic capture feature:
# sudo apt-get install -y libasound2-dev
# cargo build -p zerocast_audio --features capture
```

## Smoke test

```bash
# Terminal 1
cargo run -p zerocast_desktop -- recv-log 0.0.0.0:5000 --audio

# Terminal 2
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 127.0.0.1:5000 --profile low --audio --frames 11
```

Expect `audio rtp bytes=…` lines alongside video access units.

## QEMU ARM lab

Forward UDP 5002/5003 in user-net mode; bridged `virbr0` passes audio like video.

## Next (v1)

- cpal capture default on desktop sender
- Opus decode + cpal playback on recv
- RTCP-based A/V sync
- Discover: require TXT `audio=1` when sender uses `--audio`
