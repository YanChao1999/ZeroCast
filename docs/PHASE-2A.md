# Phase 2a — mDNS discovery

Branch: `phase-2a/mdns-discovery`

## Goal

Zero-config LAN cast: receivers find senders without typing IP/port or resolution.

## Service definition

| Field | Value |
|-------|--------|
| Type | `_zerocast._udp.local.` |
| TXT `v` | `1` (protocol version) |
| TXT `w` | stream width |
| TXT `h` | stream height |
| TXT `fps` | frame rate |
| Port | RTP UDP port (same as today) |

## Crate

`zerocast_discovery` — publish (`StreamPublisher`) and browse (`browse_streams`).

## CLI (planned)

- `stream … --discover` or auto-publish when streaming (register on start, unregister on exit)
- `recv --discover` — list instances, pick one, use TXT `w`/`h` for recv

## Status

- [x] Branch + `zerocast_discovery` scaffold
- [ ] Wire `stream` to publish on start
- [ ] Wire `recv --discover` to browse + connect
- [ ] Integration test (optional, LAN-only)
