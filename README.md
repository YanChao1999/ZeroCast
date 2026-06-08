# ZeroCast — Cross-Platform Zero-Config LAN Screen Casting

**Version 0.2.0** (workspace; see root `Cargo.toml` `[workspace.package]`)

Lightweight, zero-configuration LAN screen casting built around a shared Rust core and thin platform adapters.

Goals
- One shared core codebase
- Native performance, low latency, small binaries
- LAN-only: no cloud, no accounts
- Cross-platform: Windows, macOS, Linux, Android, iOS
- Edge receivers: Linux ARM boards (e.g. Raspberry Pi Zero W over Wi‑Fi)

Quick start

Build the workspace:

```bash
cargo build --workspace
cargo test --workspace
# FFmpeg roundtrip (426p + 720p IDR size); CI runs this automatically:
cargo test -p zerocast_transport --test ffmpeg_integration
# includes 10-frame cycle live-pipe roundtrip at 426p
```

Run the desktop app (requires `ffmpeg` on PATH for H.264 streaming):

**Zero-config (mDNS, Phase 2a):**

```bash
# Terminal 1 — receiver (publishes listen port + resolution via mDNS)
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240 15

# Terminal 2 — sender (discovers receiver on the LAN)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover
```

**Profiles (`low` | `med` | `high` | `auto`)** — same dimensions on both sides; `auto` uses your primary display (e.g. 1920×1080 @ 60):

```bash
# Pi Zero W–class / low bandwidth (426×240 @ 15 fps) — recommended for same-PC smoke tests
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 --profile low
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile low

# 720p30
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 --profile med
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile med

# Full display resolution (capped to 1080p, refresh up to 60 Hz)
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 --profile auto
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile auto
```

With `--discover` and no `--profile`, the sender uses the receiver’s advertised width/height/fps from mDNS.

**Manual IP (same machine or fixed address):**

```bash
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240 15
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 127.0.0.1:5000 426 240 15

# Or pick a profile on the sender only (receiver window size is still from recv args)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 127.0.0.1:5000 --profile low
```

Install [ffmpeg](https://ffmpeg.org/download.html) on your PATH. On Windows/macOS/Linux the app uses the primary display via `scrap` (falls back to a test pattern if capture is unavailable).

Run tests:

```bash
cargo test --workspace
```

Architecture (high level)

- Shared Rust Core + Thin Native Platform Adapters + Cross-Platform UI
- Media pipeline: Capture → GPU Texture → Hardware Encoder → RTP → **HW Decode** → GPU/HDMI (zero-copy target; MVP uses CPU RGB24 + minifb)
- Discovery: mDNS; Transport: RTP over UDP; Control: QUIC/TCP

Recommended stack (summary)
- Language: Rust
- UI: egui + wgpu
- Async runtime: tokio
- QUIC: quinn
- Device discovery: mdns-sd
- Audio: cpal + opus
- Video codec: H.264 (start)

Project layout

- `apps/desktop` — desktop CLI (`send`, `recv`, `cap`, `stream`)
- `crates/platform` — cross-platform screen capture (`scrap` on desktop OSes)
- `crates/transport` — RTP/RTCP, H.264 encode (ffmpeg CLI), streaming loop
- `crates/core` — shared core (minimal today)
- `crates/discovery` — mDNS-SD publish/browse (Phase 2a)

Docs for agents and maintainers
- AI agent baseline: [AGENTS.md](AGENTS.md)
- Agent skill and instructions: [SKILL.md](SKILL.md)
- Copilot contributor guidance: [.github/copilot-instructions.md](.github/copilot-instructions.md)
- Recommended dependency snippets: [DEPS.md](DEPS.md)

Roadmap (short)

1. Desktop MVP — screen capture, H.264 encode, RTP streaming, rendering ✅
2. **Phase 2a** — mDNS + same-PC discovery ✅ — [docs/PHASE-2A.md](docs/PHASE-2A.md)
3. **Phase 1b** — encoder pipe + sender stats ✅ — [docs/PHASE-1B.md](docs/PHASE-1B.md)
4. **Phase 1c** — discover negotiation ✅ — [docs/PHASE-1C.md](docs/PHASE-1C.md)
5. **Phase QoS v1 foundation** — metrics hints, `class=embedded` (closed loop not done) — [docs/PHASE-QOS.md](docs/PHASE-QOS.md)
6. Audio + A/V sync
7. **Phase QoS closed loop** — hot reconfigure, RTCP/recv lag — [docs/PHASE-QOS.md](docs/PHASE-QOS.md)
8. Android receiver, then sender
9. iOS support (ReplayKit + VideoToolbox)
10. **Embedded recv** — Linux ARM (Pi Zero W, Wi‑Fi) — [docs/PHASE-QOS.md](docs/PHASE-QOS.md#embedded-receivers-linux-arm--wifi)
