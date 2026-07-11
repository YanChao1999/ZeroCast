# ZeroCast protocol specifications

Normative documents for the on-the-wire ZeroCast LAN casting protocol.

| Document | Status | Description |
|----------|--------|-------------|
| [std/zerocast-protocol-v1.md](std/zerocast-protocol-v1.md) | **Draft v1** | Discovery, video RTP, audio RTP, ports, A/V timing |
| [std/zerocast-protocol-v2.md](std/zerocast-protocol-v2.md) | **Draft v2** | OOB pairing, QUIC control, SRTP (security extensions) |
| [../PHASE-AUDIO.md](../PHASE-AUDIO.md) | In progress | Audio implementation tracker |
| [../PHASE-SECURITY.md](../PHASE-SECURITY.md) | Planned | Security implementation tracker |

Implementation crates:

- `crates/protocol` — shared constants and helpers (ports, payload types, mDNS keys)
- `crates/discovery` — mDNS-SD publish/browse
- `crates/transport` — RTP send/receive (H.264 video, Opus audio)
- `crates/audio` — PCM capture and Opus encode/decode

When the spec and code disagree, **update the spec first**, then align code in the same PR.
