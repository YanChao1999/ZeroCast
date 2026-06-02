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
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover
```

Look for:

```text
encoder: live ffmpeg pipe (426x240 @ 15 fps)
stream stats: 14.2 fps, 12 ms encode avg, 850 kbps (last 30 frames)
```

## Status

- [x] Windows pipe encoder with oneshot fallback
- [x] Periodic sender stats in `stream` loop
- [ ] Hardware encoder (NVENC/QSV) — see [PHASE-QOS.md](PHASE-QOS.md)
- [ ] Persistent decoder on recv — future

## Next (1c / QoS foundation)

See [PHASE-QOS.md](PHASE-QOS.md) for profile ladder and QoS TODO.
