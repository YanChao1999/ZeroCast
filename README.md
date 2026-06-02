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
```

Run the desktop app (requires `ffmpeg` on PATH for H.264 streaming):

**Zero-config (mDNS, Phase 2a):**

```bash
# Terminal 1 — receiver (publishes listen port + resolution via mDNS)
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240 15

# Terminal 2 — sender (discovers receiver on the LAN)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover
```

**Manual IP (same machine or fixed address):**

```bash
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 127.0.0.1:5000 426 240 15
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
4. **Phase 1c (in progress)** — display detect, profiles, bitrate — [docs/PHASE-QOS.md](docs/PHASE-QOS.md)
5. Audio + A/V sync
6. **Phase QoS** — adaptive ladder, HW encoders, fallback — [docs/PHASE-QOS.md](docs/PHASE-QOS.md)
7. Android receiver, then sender
8. iOS support (ReplayKit + VideoToolbox)
9. **Embedded recv** — Linux ARM (Pi Zero W, Wi‑Fi) — [docs/PHASE-QOS.md](docs/PHASE-QOS.md#embedded-receivers-linux-arm--wifi)
