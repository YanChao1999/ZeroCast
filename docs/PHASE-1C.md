# Phase 1c — Discover negotiation

Branch: `phase-1c/discover-negotiation`

## Goal

When streaming with `--discover`, respect receiver decode limits from mDNS TXT so a desktop sender does not override a low-cap recv (e.g. Pi Zero W at 426×240) with `--profile auto`.

## Changes

| Item | Description |
|------|-------------|
| mDNS TXT | `max_w`, `max_h`, `max_fps` (decode ceiling; default = session `w`/`h`/`fps`) |
| Local registry | Same keys for same-PC discovery |
| `stream --discover` | `min(sender profile, recv caps)` via `StreamProfile::capped_for_receiver` |
| Discover list | Shows `cap WxH @ fps` when caps differ from session |

## Usage

```powershell
# Recv advertises session + cap 426x240 @ 15
cargo run -p zerocast_desktop -- recv 0.0.0.0:5000 --profile low

# Sender auto (1080p60) is negotiated down to recv cap
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover --profile auto
# discover: negotiated 426x240 @ 15 fps (recv cap 426x240 @ 15 fps)

# No --profile: use recv session size (unchanged)
cargo run -p zerocast_desktop -- stream 0.0.0.0:0 --discover
```

## Status

- [x] mDNS TXT `max_w` / `max_h` / `max_fps`
- [x] Discover negotiation with `capped_for_receiver`
- [ ] Optional `class=embedded` TXT hint
- [ ] `QosController` runtime ladder (Phase QoS v1)

## Related

- [PHASE-QOS.md](PHASE-QOS.md)
- [PHASE-2A.md](PHASE-2A.md)
