# ZeroCast Protocol v2 — security extensions (draft)

**Status:** Draft  
**Requires:** [zerocast-protocol-v1.md](zerocast-protocol-v1.md) for media packetization, ports, and discovery baseline.

v2 adds **authenticated sessions** and **SRTP-encrypted RTP/RTCP**. v1 senders and receivers interoperate when the receiver advertises `auth=open` and omits `sec=1`.

---

## 1. Overview

```text
┌─────────────┐   mDNS (capability hints)    ┌─────────────┐
│   Sender    │ ───────────────────────────► │  Receiver   │
└──────┬──────┘                              └──────┬──────┘
       │  QUIC/TLS 1.3  control  port C             │
       ├───────────────────────────────────────────►│
       │  Pair (OOB PIN) / SessionStart / Stop      │
       │                                              │
       │  SRTP video RTP/RTCP  P, P+1                 │
       ├───────────────────────────────────────────►│
       │  SRTP audio RTP/RTCP  A, A+1  (optional)   │
       └───────────────────────────────────────────►│
```

**Trust anchor:** out-of-band PIN (or QR in a later revision) during `Pair`. mDNS and RTP alone MUST NOT be used to establish initial trust.

---

## 2. Ports

| Stream | RTP | RTCP | Notes |
|--------|-----|------|-------|
| Video | P | P+1 | Same as v1; payloads are SRTP when `sec=1` |
| Audio | A | A+1 | Same derivation as v1 |
| Control | **C** | — | QUIC over UDP/TCP; default **C = P + 4** |

TXT `c_port` overrides the default control port.

---

## 3. Discovery TXT (additional keys)

| Key | Required | Example | Meaning |
|-----|----------|---------|---------|
| `v` | yes | `2` | Protocol version when security is mandatory |
| `sec` | no | `1` | `1` = SRTP required; sender MUST NOT send plain RTP |
| `auth` | no | `pin` | `open` \| `pin` \| `trusted` \| `prompt` |
| `c_port` | no | `5004` | QUIC control listen port |

All v1 TXT keys (`w`, `h`, `fps`, `audio`, …) unchanged.

Receivers with `sec=1` MUST reject unauthenticated plain RTP on P.

---

## 4. Device identity

Each installation holds:

- **`device_id`:** 16-byte random UUID (stable)
- **`device_pubkey`:** Ed25519 public key (32 bytes)

Stored in platform secure storage. Published only inside TLS-authenticated control messages, never in mDNS.

---

## 5. Control channel (QUIC)

- **Library:** QUIC with TLS 1.3 (implementation: `quinn`).
- **ALPN:** `zerocast/2`
- **Framing:** length-prefixed CBOR or protobuf messages (TBD at implementation; message types below are normative).

### 5.1 Message types

| Type | Direction | Purpose |
|------|-----------|---------|
| `PairRequest` | S → R | Start SPAKE2; includes sender `device_id`, ephemeral SPAKE2 payload |
| `PairAccept` | R → S | SPAKE2 completion; receiver `device_id`, `device_pubkey`, trust granted |
| `PairReject` | R → S | Wrong PIN, policy deny, or rate limit |
| `SessionStart` | S → R | Request media session; includes negotiated profile hints |
| `SessionAccept` | R → S | Session id, SRTP key material (encrypted under pairing secret) |
| `SessionReject` | R → S | Policy deny |
| `SessionStop` | either | Tear down SRTP context |
| `Ping` / `Pong` | either | Keepalive |

### 5.2 Pairing (SPAKE2 + OOB PIN)

1. Receiver displays 6-digit decimal PIN (000000–999999), valid for 300 s while in pairing mode.
2. Sender prompts user for PIN; runs SPAKE2 with receiver’s published SPAKE2 payload from `PairRequest`/`PairAccept` exchange.
3. Both derive `pairing_secret` (32 bytes). PIN MUST NOT appear on the wire.
4. On success, sender stores `(receiver_device_id, receiver_pubkey)` in trust store.
5. On failure, receiver applies exponential backoff (min 1 s between attempts per source IP).

### 5.3 Session resume (trusted sender)

If sender’s `device_id` is in receiver trust store:

1. `SessionStart` includes sender `device_id` and Ed25519 signature over `(session_nonce || receiver_device_id)`.
2. Receiver verifies signature against stored pubkey; skips PIN.
3. Both derive fresh SRTP keys from `pairing_secret` or a stored long-term secret plus `session_nonce` (implementation detail; MUST provide forward secrecy per session via random `session_nonce`).

### 5.4 Session key export

After `SessionAccept`:

```text
session_nonce = random 32 bytes (from SessionAccept)
master_secret = HKDF-Expand(
    HKDF-Extract(salt=empty, IKM=pairing_secret || session_nonce),
    info="zerocast/v2/session",
    L=32
)
video_key, video_salt = HKDF-Expand(master_secret, "zerocast/v2/video", 30)
audio_key, audio_salt = HKDF-Expand(master_secret, "zerocast/v2/audio", 30)
```

SRTP profile: AES-128-GCM, RTP auth tag 80 bits (RFC 7714).

---

## 6. SRTP media

When `sec=1`:

- Video RTP on P and RTCP on P+1 use the **video** SRTP context.
- Audio RTP on A and RTCP on A+1 use the **audio** SRTP context.
- SSRC, payload types, clock rates, and H.264/Opus packetization are **identical to v1** inside the SRTP plaintext.

Senders MUST NOT emit plain RTP to a receiver advertising `sec=1`.

---

## 7. Authorization policies

| `auth` | Receiver behavior |
|--------|-------------------|
| `open` | Accept v1 plain RTP (no control channel) |
| `pin` | Require `Pair` for unknown senders; auto `SessionStart` for trusted |
| `trusted` | Drop unknown senders; no plain RTP |
| `prompt` | User must confirm each new `SessionStart` (UI) |

---

## 8. Security considerations

1. **mDNS is untrusted** — treat browse results as hints; bind trust only via OOB pairing or stored keys.
2. **Do not expose C or P to the public Internet** without additional tunnel/VPN design (out of v2 scope).
3. **Revocation** — receiver MUST support clearing trust store; optional `RevokeTrust` control message (v2.1).
4. **Replay** — SRTP ROC handling; control messages include monotonic counters or QUIC stream ordering.

---

## 9. Changelog

| Version | Change |
|---------|--------|
| **2.0 (draft)** | OOB PIN pairing, QUIC control, SRTP video/audio, TXT `sec`, `auth`, `c_port` |

---

## 10. References

- [zerocast-protocol-v1.md](zerocast-protocol-v1.md)
- [PHASE-SECURITY.md](../../PHASE-SECURITY.md) — implementation tracker
- RFC 3711 — SRTP
- RFC 7714 — AES-GCM SRTP
- RFC 9000 — QUIC
