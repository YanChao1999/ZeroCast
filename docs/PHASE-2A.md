# Phase 2a — mDNS discovery

Branch: `phase-2a/mdns-discovery`

## Goal

Zero-config LAN cast: sender finds the receiver without typing IP, port, or resolution.

## Service definition

| Field | Value |
|-------|--------|
| Type | `_zerocast._udp.local.` |
| TXT `v` | `1` (protocol version) |
| TXT `w` | stream width |
| TXT `h` | stream height |
| TXT `fps` | frame rate |
| Port | RTP UDP port the receiver is listening on |

**Publisher:** `recv` (receiver listening).  
**Browser:** `stream --discover` (sender).

## Usage

```powershell
# Terminal 1 — receiver (publishes mDNS by default)
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240 15

# Terminal 2 — sender (discovers receiver)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover
```

Manual mode (unchanged):

```powershell
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 426 240 --no-mdns
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 192.168.1.10:5000 426 240 15
```

If multiple receivers are visible, set `ZERO_CAST_PICK=0` (index) or stop extra receivers.

## Status

- [x] `zerocast_discovery` crate
- [x] `recv` publishes on start
- [x] `stream --discover` browses and connects
- [x] Same-PC discovery via local registry (Windows fallback when mDNS is blocked between processes)
- [ ] LAN integration test on two machines (optional)
