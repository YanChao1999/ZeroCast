# Phase 1d — Persistent decode (deferred)

Branch: `phase-1d/persistent-decoder` (housekeeping only until decode path is chosen)

## Goal

Reuse one decode pipeline on `recv` (symmetric to the live encode pipe). **Not started in code** — cast already hits ~13 fps @ low profile with no user perf complaints.

## Blocker (ffmpeg CLI)

Raw H.264 on `ffmpeg -f h264 -i pipe:0` does **not** emit RGB24 until stdin closes on Windows (verified with ffmpeg 8.1). A long-lived stdin pipe blocks on `read_exact` even after flush + second access unit.

## Viable next implementations (pick one when perf matters)

| Approach | Effort | Notes |
|----------|--------|--------|
| `libav` / `ffmpeg-next` decode context | Medium | Optional `libav` feature; true persistent decode |
| RTP-fed ffmpeg (`-i rtp://127.0.0.1:PORT`) | Medium | Forward UDP/RTP to local ffmpeg; one process |
| MPEG-TS mux per AU into pipe | High | Framed elementary stream without EOF |

Until then, recv keeps **one-shot** `decode_access_unit_rgb24` (correct, slightly higher CPU).

## Shipped on this branch

- [x] `.gitignore` for `*.h264` / `*.rgb`
- [x] [PHASE-QOS.md](PHASE-QOS.md) / [PHASE-1C.md](PHASE-1C.md) status sync

## Recommended order (unchanged)

1. ~~Housekeeping~~ (this branch)
2. Persistent decode — **defer** (blocker above; no user pressure)
3. **LAN two-machine** mDNS test (next actionable)
4. Pi / ARM `recv` spike
5. QoS v1 — only when metrics justify it
