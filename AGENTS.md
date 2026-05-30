# AI Agent Baseline (AGENTS.md)

Purpose: give AI coding agents a concise, actionable baseline for working in this repository.

Key links
- Repo README: [README.md](README.md)
- Workspace manifest: [Cargo.toml](Cargo.toml)
- Core crate: [crates/core/src/lib.rs](crates/core/src/lib.rs)

Quick start

Build entire workspace:

```bash
cargo build --workspace
```

Run the minimal desktop placeholder:

```bash
cargo run -p zerocast_desktop
```

Run tests (workspace):

```bash
cargo test --workspace
```

Repository layout (important paths)
- apps/desktop — minimal desktop app: [apps/desktop/Cargo.toml](apps/desktop/Cargo.toml)
- crates/core — shared core logic: [crates/core](crates/core)
- crates/discovery, transport, protocol, media, audio, video, renderer, ui, platform — focused crates in `crates/`

High-level architectural notes (concise)
- Shared Rust core with thin platform adapters.
- Prefer native GPU APIs + hardware encoders; zero-copy media pipeline.
- LAN-first: mDNS discovery, RTP over UDP transport, QUIC/TCP control channel.
- UI: egui + wgpu for controls + native surface for video rendering.

Agent guidance (how AI agents should operate in this repo)
- Keep changes focused: prefer small, well-scoped edits that update only relevant crates/files.
- Follow workspace conventions: Rust 2021 edition, use existing crate structure and Cargo workspace.
- For new platform integrations, open a design proposal first as a PR draft and link to it in the issue.
- When modifying core/shared crates, run `cargo test` and `cargo build --workspace` locally (or in CI) before committing.
- Avoid adding heavy dependencies without justification; prefer platform-native bindings for performance-sensitive code.

When to ask the maintainer
- Adding new external dependencies that affect binary size or licensing.
- Changing the transport or codec choices (e.g., switching to AV1 or full WebRTC).
- Introducing new CI steps or cross-compilation pipelines.

Suggested next agent customizations
- `.github/copilot-instructions.md`: short user-facing guidance and contributor notes.
- A small skill to run `cargo build --workspace` and report failures with suggested fixes.

Contact/Context
- This file is intentionally minimal and links to project docs. See [README.md](README.md) for broader context.
