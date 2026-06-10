# ZeroCast Protocol v1 (draft)

**Version:** 1 (TXT key `v=1`)  
**Scope:** LAN-only screen casting — discovery, video, optional audio, control (future).

This document is the normative reference for interoperable ZeroCast senders and receivers.
Constants are mirrored in `crates/protocol`.

---

## 1. Overview

```text
┌─────────────┐   mDNS (_zerocast._udp)    ┌─────────────┐
│   Sender    │ ─────────────────────────► │   Receiver  │
│  (desktop)  │                            │ (desktop/Pi)│
└──────┬──────┘                            └──────┬──────┘
       │                                          │
       │  RTP/UDP video (H.264)  port P           │
       ├─────────────────────────────────────────►│
       │  RTCP (SR/RR)             port P+1       │
       ├─────────────────────────────────────────►│
       │                                          │
       │  RTP/UDP audio (Opus)     port A  (opt)  │
       ├─────────────────────────────────────────►│
       │  RTCP                       port A+1     │
       └─────────────────────────────────────────►│
```

- **Transport:** RTP over UDP (no TCP/WebRTC in v1).
- **Discovery:** DNS-SD / mDNS (`mdns-sd`), service type `_zerocast._udp.local.`
- **Video codec:** H.264 (Annex B NALUs), RFC 6184 packetization.
- **Audio codec (v1.1):** Opus, 48 kHz, mono or stereo.
- **Control (future):** QUIC or TCP on a separate port (not defined in v1).

---

## 2. Default ports

| Stream | RTP | RTCP | Notes |
|--------|-----|------|-------|
| Video | **5000** | **5001** | Primary session; SRV record port |
| Audio | **5002** | **5003** | Optional; only when TXT `audio=1` |

**Port derivation:** when audio is co-located with video on the same host:

```text
A = P + 2        (audio RTP)
RTCP(video) = P + 1
RTCP(audio) = A + 1
```

Receivers MAY advertise a different audio port via TXT `a_port`. Senders MUST NOT assume `P+2` if `a_port` is present.

Both RTP and RTCP port pairs MUST be free on the receiver before bind (same rule as today’s video path).

---

## 3. Discovery (mDNS-SD)

### 3.1 Service type

```text
_zerocast._udp.local.
```

### 3.2 SRV

- **Target:** receiver hostname (e.g. `my-pi.local.`)
- **Port:** video RTP port `P` (default 5000)
- **Priority / weight:** 0 / 0 (unused in v1)

### 3.3 TXT records (mandatory and optional)

| Key | Required | Example | Meaning |
|-----|----------|---------|---------|
| `v` | yes | `1` | Protocol version (this document) |
| `w` | yes* | `426` | Session / window width (pixels) |
| `h` | yes* | `240` | Session height |
| `fps` | yes* | `15` | Session frame rate |
| `max_w` | no | `426` | Decode width ceiling |
| `max_h` | no | `240` | Decode height ceiling |
| `max_fps` | no | `15` | Decode fps ceiling |
| `class` | no | `embedded` | Device hint (`embedded`, `desktop`, …) |
| `audio` | no | `1` | `1` = receiver accepts Opus RTP |
| `a_port` | no | `5002` | Audio RTP port (default `P+2`) |
| `a_sr` | no | `48000` | Opus sample rate (Hz); default 48000 |
| `a_ch` | no | `2` | Channels: `1` mono, `2` stereo (default 2) |

\*Required for receivers that publish video dimensions. Headless log receivers SHOULD publish `w`/`h`/`fps` matching the negotiated profile.

### 3.4 Browse / pick

- Senders browse for `_zerocast._udp.local.` (multicast UDP 5353).
- Same-host fallback: registry under `$XDG_RUNTIME_DIR/ZeroCast/receivers/` (implementation detail).
- Multiple instances: sender uses `ZERO_CAST_PICK=N` or user selection (future UI).

---

## 4. Video RTP

| Field | Value |
|-------|-------|
| Payload type | **96** (dynamic) |
| Clock rate | **90 000** Hz |
| MIME | `H264` |
| Packetization | RFC 6184 (single NAL, STAP-A, FU-A) |
| Max payload | ≤ 1200 bytes (typical LAN MTU) |
| SSRC | Random per sender session |
| Timestamp | `pts_ns * 90000 / 1e9` (wrap u32) |

Access units MUST include SPS/PPS before IDR/slice data when required by the decoder (see project encoder notes).

RTCP Sender Reports (SR) SHOULD be sent ~1 Hz on port `P+1` for future A/V sync and QoS.

---

## 5. Audio RTP (v1.1)

| Field | Value |
|-------|-------|
| Payload type | **111** (dynamic) |
| Clock rate | **48 000** Hz |
| MIME | `opus` |
| Frame duration | **20 ms** (960 samples @ 48 kHz) |
| Channels | 1 or 2 (advertised in TXT `a_ch`) |
| SSRC | **Distinct** from video SSRC |
| Timestamp | Increment **960** per 20 ms frame (48 kHz clock) |

Each RTP packet payload is one **Opus packet** (not RFC 6716 framing — raw Opus toc + frames as produced by libopus).

- **Marker bit:** MAY be set on talk-spurt boundaries; v1 senders MAY leave unset.
- **RTCP:** SR on port `A+1` ~1 Hz (same NTP mapping as video).

### 5.1 Sender without microphone (test / CI)

Senders MAY emit a synthetic PCM source (e.g. sine tone) encoded as Opus for transport testing.

### 5.2 Playback (future)

Receivers decode Opus and play via cpal/ALSA/PulseAudio. v1.1 implementation logs RTP byte counts only (`recv-log --audio`).

---

## 6. A/V synchronization (v1.1 foundation)

Shared **session origin:** wall-clock `Instant` at sender when both streams start.

| Stream | PTS source |
|--------|------------|
| Video | `frame_index` paced at `1/fps`; RTP ts from encoder PTS ns |
| Audio | Sample count since origin; RTP ts = `(samples * 48000) / sample_rate` |

**v1.1:** log skew between video frame index time and audio sample time.  
**v1.2 (planned):** RTCP SR cross-correlation + small recv playout buffer.

---

## 7. QoS hints (informational)

Existing TXT `class=embedded` and `max_*` caps apply to **video** only in v1.  
Audio bitrate is low (~32–64 kbps Opus); no ladder in v1.1.

Future TXT keys: `tier`, `out=drm` (see [PHASE-QOS.md](../../PHASE-QOS.md)).

---

## 8. Security

v1 is **trusted LAN** only: no encryption, no authentication.  
Do not expose RTP ports to the public Internet without a future DTLS/SRTP layer.

---

## 9. Changelog

| Version | Change |
|---------|--------|
| **1.0** | mDNS, H.264 video RTP/RTCP, profiles, negotiation |
| **1.1** | Opus audio RTP/RTCP, TXT `audio`, `a_port`, `a_sr`, `a_ch` (draft) |

---

## 10. References

- RFC 3550 — RTP
- RFC 6184 — H.264 RTP payload
- RFC 6716 — Opus codec
- RFC 6762 / 6763 — mDNS / DNS-SD
