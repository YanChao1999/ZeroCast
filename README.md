# ZeroCast — Cross-Platform Zero-Config LAN Screen Casting

Lightweight, zero-configuration LAN screen casting built around a shared Rust core and thin platform adapters.

Goals
- One shared core codebase
- Native performance, low latency, small binaries
- LAN-only: no cloud, no accounts
- Cross-platform: Windows, macOS, Linux, Android, iOS
- Could deploy on some edge device

Quick start

Build the workspace:

```bash
cargo build --workspace
```

Run the desktop app (requires `ffmpeg` on PATH for H.264 streaming):

```bash
# Terminal 1 — receiver (video window; match stream width/height)
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240

# Terminal 2 — screen capture → encode → RTP
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 127.0.0.1:5000 426 240 15
```

Install [ffmpeg](https://ffmpeg.org/download.html) on your PATH. On Windows/macOS/Linux the app uses the primary display via `scrap` (falls back to a test pattern if capture is unavailable).

Run tests:

```bash
cargo test --workspace
```

Architecture (high level)

- Shared Rust Core + Thin Native Platform Adapters + Cross-Platform UI
- Media pipeline: Capture → GPU Texture → Hardware Encoder → RTP → Decoder → GPU Renderer
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

Docs for agents and maintainers
- AI agent baseline: [AGENTS.md](AGENTS.md)
- Agent skill and instructions: [SKILL.md](SKILL.md)
- Copilot contributor guidance: [.github/copilot-instructions.md](.github/copilot-instructions.md)
- Recommended dependency snippets: [DEPS.md](DEPS.md)

Roadmap (short)

1. Desktop MVP — screen capture, H.264 encode, RTP streaming, rendering
2. Audio + mDNS discovery + sync
3. Android receiver, then sender
4. iOS support (ReplayKit + VideoToolbox)
