# Phase Security — pairing, control, and SRTP

Branch: TBD (`phase-security/v0-pairing`)

## Goal

Add **optional authenticated, encrypted casting** while keeping v1 plain RTP working on trusted LANs.

Security requires an **out-of-band (OOB) trust anchor** at least once (PIN, QR, or physical confirm). In-band channels alone (mDNS, RTP, QUIC on the LAN) cannot establish initial trust against an active LAN attacker.

Normative wire details: [spec/std/zerocast-protocol-v2.md](spec/std/zerocast-protocol-v2.md).

---

## Threat model

| Threat | v1 (today) | v2 target |
|--------|------------|-----------|
| Unauthorized cast to receiver | Anyone on LAN | Paired / PIN / prompt only |
| Eavesdrop screen or audio | Plain RTP | SRTP (AES-GCM) |
| Fake receiver (rogue mDNS) | Possible | OOB pairing binds receiver identity |
| RTP inject / replay | Possible | SRTCP auth + session keys |
| Internet exposure | Dangerous | Still unsupported; SRTP is LAN hardening |

**Non-goals (v2):** cloud accounts, enterprise PKI, DRM (`out=drm` remains a future QoS hint).

---

## Why out-of-band is required

In-band paths are attacker-controllable on a shared LAN:

```text
mDNS browse     →  anyone can advertise _zerocast._udp
RTP/UDP         →  sniff, inject, flood
QUIC control    →  MITM unless peers already share a secret
```

Without OOB (or pre-shared trust from a prior OOB pairing), a network attacker can impersonate the receiver and relay the “secure” handshake (classic MITM).

**OOB** means a channel or action **outside** the cast media path, for example:

| Method | When |
|--------|------|
| **PIN on receiver** | User types 6-digit code on sender (recommended v0) |
| **QR code** | Mobile scans TV display (v1+) |
| **Physical confirm** | “Allow cast from laptop?” on receiver UI |
| **Stored trust** | OS keychain after first successful OOB pair |

OOB is needed for **first pairing** and **re-pair after revoke**, not every session.

---

## Architecture

```text
┌─────────────┐                              ┌─────────────┐
│   Sender    │                              │  Receiver   │
└──────┬──────┘                              └──────┬──────┘
       │  mDNS browse (hints only, untrusted)       │
       ├───────────────────────────────────────────►│
       │                                              │
       │  QUIC + TLS 1.3  control port C            │
       │  Pair / SessionStart / QoS / Stop           │
       ├───────────────────────────────────────────►│
       │◄───────────────────────────────────────────│
       │                                              │
       │  SRTP keys derived after Pair or resume     │
       │                                              │
       │  SRTP video RTP/RTCP   P, P+1               │
       ├───────────────────────────────────────────►│
       │  SRTP audio RTP/RTCP   A, A+1  (optional)  │
       └───────────────────────────────────────────►│
```

- **Discovery** stays mDNS; TXT advertises capability (`sec=1`), never secrets.
- **Control** on QUIC (`quinn`) — pairing, session lifecycle, QoS (future).
- **Media** stays UDP; **SRTP** wraps existing H.264 / Opus packetization in `crates/transport`.

Default control port: **`C = P + 4`** unless TXT `c_port` is set.

---

## Pairing flow (v0 — PIN + SPAKE2)

```text
Receiver                          Sender
   │                                 │
   │  display PIN "482913"           │
   │                                 │  user picks receiver, enters PIN
   │◄──── QUIC: PairRequest ─────────│  SPAKE2 (PIN never on wire)
   │──── PairAccept + device_id ────►│
   │                                 │  store device_id → Ed25519 pubkey
   │──── SessionStart ──────────────►│  subsequent casts: no PIN
   │◄─── SessionAccept + SRTP mat ───│
   │◄─── SRTP video/audio RTP ───────│
```

1. Receiver generates ephemeral Ed25519 keypair; shows random 6-digit PIN (rotates every 5 min while waiting).
2. Sender connects to `c_port`, runs **SPAKE2** with PIN as password; both derive `pairing_secret`.
3. `PairAccept` includes receiver `device_id` and long-term public key.
4. Sender stores trust in platform keychain (`crates/platform`).
5. `SessionStart` / `SessionAccept` derive per-session SRTP keys via HKDF (see v2 spec).

**Re-pair:** receiver `RevokeTrust(device_id)` or user clears trust store.

---

## Cryptography

| Layer | Mechanism |
|-------|-----------|
| Device identity | Ed25519 long-term key (generated on first run) |
| Pairing | SPAKE2 + HKDF |
| Control transport | QUIC + TLS 1.3 |
| Video / audio RTP | SRTP AES-128-GCM (RFC 3711) |
| RTCP | SRTCP (same key context) |

Key derivation after pairing or session resume:

```text
master_secret = HKDF(pairing_secret | session_nonce, "zerocast/session")
video_key, video_salt = HKDF(master_secret, "zerocast/video")
audio_key, audio_salt = HKDF(master_secret, "zerocast/audio")
```

Separate SRTP contexts per SSRC (video vs audio).

---

## mDNS TXT (v2 hints)

| Key | Required | Example | Meaning |
|-----|----------|---------|---------|
| `sec` | no | `1` | Security required; plain v1 RTP rejected |
| `auth` | no | `pin` | `open` \| `pin` \| `trusted` \| `prompt` |
| `c_port` | no | `5004` | QUIC control port (default `P+4`) |

Never publish PINs, keys, or session tokens in TXT.

---

## Receiver authorization policies

| Policy | Behavior |
|--------|----------|
| `open` | v1 plain RTP (lab / trusted VLAN only) |
| `pin` | Unknown senders must OOB pair; known senders auto-accept |
| `trusted` | Only paired senders; drop others |
| `prompt` | UI asks per session |

Suggested defaults: desktop **`pin`**, headless Pi **`trusted`** (pair once from desktop).

---

## Phased rollout

### Security v0 — PIN + SRTP (MVP)

- [ ] `crates/security` — device keys, trust store trait, SPAKE2, HKDF
- [ ] QUIC control messages: `Pair`, `SessionStart`, `SessionStop`, `Ping`
- [ ] SRTP wrap/unwrap in `transport` (video + RTCP; audio behind `audio` feature)
- [ ] mDNS publish/browse `sec`, `auth`, `c_port`
- [ ] CLI: `recv --require-pin`, `stream --discover --secure`
- [ ] Draft spec: [zerocast-protocol-v2.md](spec/std/zerocast-protocol-v2.md)

### Security v1 — hardening

- [ ] TOFU pubkey pinning after first pair
- [ ] Session rekey every N minutes
- [ ] Rate limits on control + RTP
- [ ] Bind recv to specific LAN IP; firewall notes in docs

### Security v2 — UX

- [ ] QR pairing payload
- [ ] Per-session “Allow cast?” prompt
- [ ] Receiver audit log

---

## Crate mapping

| Crate | Responsibility |
|-------|----------------|
| `protocol` | v2 constants, TXT keys, control message IDs |
| `security` (new) | Pairing, trust store, SRTP session state |
| `transport` | `SecureSender` / `SecureReceiver` over existing RTP |
| `discovery` | Publish and parse `sec`, `auth`, `c_port` |
| `platform` | Keychain / credential storage per OS |
| `apps/desktop` | PIN entry, `--secure`, trust management UI |

Cargo feature: `secure` (optional; v1 builds unchanged without it).

---

## Operational guidance (until v2 ships)

v1 is **trusted LAN only** ([protocol v1 §8](spec/std/zerocast-protocol-v1.md#8-security)):

1. Do not port-forward RTP ports (5000–5003) to the Internet.
2. Isolate cast devices on a home VLAN or guest Wi‑Fi without untrusted peers.
3. Use `recv --no-mdns` + manual IP on shared networks.
4. Bind recv to a specific LAN address when multi-homed.

---

## Open decisions

1. **Dual-stack vs secure-only port** — plain RTP on `P` and secure on `P+8`, or reject plain when `sec=1`?
2. **SRTP library** — `webrtc-srtp` vs FFI to libsrtp (Pi Zero W binary size).
3. **PIN length** — 6 digits (UX) vs longer for high-threat environments.
4. **Mutual TLS on QUIC** — client certs for senders vs SPAKE2-only identity.

---

## References

- [zerocast-protocol-v1.md](spec/std/zerocast-protocol-v1.md) — v1 plain LAN mode
- [zerocast-protocol-v2.md](spec/std/zerocast-protocol-v2.md) — v2 security extensions (draft)
- RFC 3711 — SRTP
- RFC 7748 / SPAKE2 — password-authenticated key exchange
