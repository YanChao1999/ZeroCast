# Phase 1b — Streaming quality

Branch: `phase-1b/encoder-perf`

## Goal

Raise sustained frame rate and observability on the desktop cast path before audio/QoS.

## Changes

| Item | Description |
|------|-------------|
| Live ffmpeg pipe | Try persistent `ffmpeg` stdin/stdout on all OSes (Windows uses `repeat-headers=0` + cached SPS/PPS) |
| Oneshot fallback | If the pipe stalls or fails to spawn, fall back to one-shot encode per frame |
| Sender stats | Every 30 frames: actual fps, average encode ms, estimated video kbps |

## Usage

Same as today:

```powershell
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240 15
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 --profile low
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile low
```

Look for:

```text
encoder: live ffmpeg pipe (426x240 @ 15 fps)
stream: frame 0 3 nalus [type 7 22b, type 8 4b, type 5 20106b]
stream stats: 12.9 fps, 1 ms encode avg, 2102 kbps (last 30 frames)
```

## Test plan

- [x] `cargo test --workspace`
- [x] Same PC: `recv --profile low` + `stream 0.0.0.0:0 --discover --profile low` (330+ frames, no decode errors)
- [x] Same PC: `recv` + `stream --discover` with explicit `426 240 15` (300+ frames)
- [x] `recv --profile auto` / `stream --discover --profile auto` smoke test (1080p60; ~7 fps, LAN bandwidth heavy)
- [x] Pipe mode at 426×240 @ 15 fps (live pipe; ~1 ms encode, ~2 Mbps; no frame-0 oneshot fallback in latest run)
- [x] NAL sanity: 3 NALs per frame (SPS ~22 B, PPS ~4 B, one IDR slice); recv shows no `non-existing PPS` errors

Manual commands:

```powershell
cargo test --workspace
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 --profile low
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile low
```

## Status

- [x] Windows pipe encoder with oneshot fallback
- [x] Periodic sender stats in `stream` loop
- [ ] Hardware encoder (NVENC/QSV) — see [PHASE-QOS.md](PHASE-QOS.md)
- [ ] Persistent decoder on recv — future

## Next (1c / QoS foundation)

See [PHASE-QOS.md](PHASE-QOS.md) for profile ladder and QoS TODO.
