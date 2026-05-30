<!-- Copilot contributor guidance -->
# Copilot / AI Contributor Instructions

Purpose: short, human-facing guidance for Copilot and AI contributors.

- Repo quick start: see `README.md`.
- Keep PRs small and focused; prefer one feature per branch.
- Run `cargo build --workspace` and `cargo test --workspace` before asking for code changes.
- When changing low-level media or transport code, open a design PR with rationale and testing plan.

Suggested prompts for AI contributors:

- "Implement a minimal PipeWire-based receiver crate that decodes H.264 and renders to wgpu." 
- "Add mDNS-based discovery using `mdns-sd` and publish a sample TXT record format."
