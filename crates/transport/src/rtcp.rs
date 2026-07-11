//! Minimal RTCP helpers (RFC 3550): Sender Report + ZeroCast recv feedback APP.

/// Seconds between NTP epoch (1900-01-01) and Unix epoch (1970-01-01).
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;
const NS_PER_SEC: u128 = 1_000_000_000;

/// RFC 3550 RTCP version field (V=2).
const RTCP_VERSION: u8 = 2;
const RTCP_VERSION_SHIFT: u8 = 6;

/// Packet type for Sender Report.
const RTCP_PT_SR: u8 = 200;

/// Minimum SR size: 4-byte header + SSRC + NTP + RTP ts + packet/octet counts.
const RTCP_SR_MIN_LEN: usize = 28;

/// RTCP length field for a minimal SR (32-bit words minus one) = 6 → 28 bytes.
const RTCP_SR_LENGTH_WORDS: u16 = 6;

/// First header byte for V=2, P=0, reception report count 0.
const RTCP_HDR_V2_RC0: u8 = 0x80;

/// Parsed RTCP Sender Report anchor for A/V sync.
#[derive(Debug, Clone, Copy)]
pub struct RtcpSrAnchor {
    pub ssrc: u32,
    pub rtp_ts: u32,
    pub wall_ns: u128,
}

/// Parse an RTCP SR from a UDP payload. Returns `None` if not a valid SR.
pub fn parse_rtcp_sr(buf: &[u8]) -> Option<RtcpSrAnchor> {
    if buf.len() < RTCP_SR_MIN_LEN {
        return None;
    }
    let version = buf[0] >> RTCP_VERSION_SHIFT;
    if version != RTCP_VERSION {
        return None;
    }
    if buf[1] != RTCP_PT_SR {
        return None;
    }
    let length_words = u16::from_be_bytes([buf[2], buf[3]]);
    let packet_len = (usize::from(length_words) + 1).checked_mul(4)?;
    if packet_len < RTCP_SR_MIN_LEN || buf.len() < packet_len {
        return None;
    }

    let ssrc = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ntp_secs = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let ntp_frac = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);
    let rtp_ts = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
    Some(RtcpSrAnchor {
        ssrc,
        rtp_ts,
        wall_ns: ntp_to_wall_ns(ntp_secs, ntp_frac),
    })
}

pub fn ntp_to_wall_ns(ntp_secs: u32, ntp_frac: u32) -> u128 {
    let unix_secs = (ntp_secs as u64).saturating_sub(NTP_UNIX_OFFSET) as u128;
    let frac_ns = ((ntp_frac as u128) * NS_PER_SEC) / (1u128 << 32);
    unix_secs.saturating_mul(NS_PER_SEC).saturating_add(frac_ns)
}

/// Build a minimal RTCP SR (28 bytes).
pub fn build_rtcp_sr(ssrc: u32, rtp_ts: u32, ntp_secs: u32, ntp_frac: u32) -> [u8; RTCP_SR_MIN_LEN] {
    let mut sr = [0u8; RTCP_SR_MIN_LEN];
    sr[0] = RTCP_HDR_V2_RC0;
    sr[1] = RTCP_PT_SR;
    sr[2] = ((RTCP_SR_LENGTH_WORDS >> 8) & 0xFF) as u8;
    sr[3] = (RTCP_SR_LENGTH_WORDS & 0xFF) as u8;
    sr[4..8].copy_from_slice(&ssrc.to_be_bytes());
    sr[8..12].copy_from_slice(&ntp_secs.to_be_bytes());
    sr[12..16].copy_from_slice(&ntp_frac.to_be_bytes());
    sr[16..20].copy_from_slice(&rtp_ts.to_be_bytes());
    sr
}

pub fn wall_to_ntp(now_ns: u128) -> (u32, u32) {
    let unix_secs = (now_ns / NS_PER_SEC) as u64;
    let unix_nanos = (now_ns % NS_PER_SEC) as u128;
    let ntp_secs = unix_secs.saturating_add(NTP_UNIX_OFFSET) as u32;
    let ntp_frac = ((unix_nanos * (1u128 << 32)) / NS_PER_SEC) as u32;
    (ntp_secs, ntp_frac)
}

/// Packet type for Application-Defined RTCP.
const RTCP_PT_APP: u8 = 204;

/// ZeroCast APP name (4 ASCII bytes).
pub const ZC_APP_NAME: [u8; 4] = *b"ZCst";

/// Recv → sender QoS feedback (RTCP APP).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecvFeedback {
    /// Fraction lost over the reporting window (0.0–1.0).
    pub fraction_lost: f64,
    /// Inter-arrival jitter estimate in milliseconds.
    pub jitter_ms: f64,
    /// Decode queue lag (queued frames × frame period + recent decode time).
    pub decode_lag_ms: f64,
    /// Frames dropped because decode or display fell behind.
    pub dropped_frames: u16,
}

/// Build a ZeroCast RTCP APP feedback packet (24 bytes).
pub fn build_recv_feedback(ssrc: u32, fb: RecvFeedback) -> [u8; 24] {
    const BODY_LEN: usize = 9;
    let words = (4 + BODY_LEN).div_ceil(4); // SSRC + name + body → 6 words
    let length_words = (words as u16).saturating_sub(1);
    let mut pkt = [0u8; 24];
    pkt[0] = RTCP_HDR_V2_RC0;
    pkt[1] = RTCP_PT_APP;
    pkt[2] = ((length_words >> 8) & 0xFF) as u8;
    pkt[3] = (length_words & 0xFF) as u8;
    pkt[4..8].copy_from_slice(&ssrc.to_be_bytes());
    pkt[8..12].copy_from_slice(&ZC_APP_NAME);
    pkt[12] = 1; // feedback version
    let frac = (fb.fraction_lost.clamp(0.0, 1.0) * 255.0).round() as u8;
    pkt[13] = frac;
    let lag = fb.decode_lag_ms.clamp(0.0, 65_535.0).round() as u16;
    let jitter = fb.jitter_ms.clamp(0.0, 65_535.0).round() as u16;
    pkt[14..16].copy_from_slice(&lag.to_be_bytes());
    pkt[16..18].copy_from_slice(&jitter.to_be_bytes());
    pkt[18..20].copy_from_slice(&fb.dropped_frames.to_be_bytes());
    pkt
}

/// Parse a ZeroCast RTCP APP feedback packet.
pub fn parse_recv_feedback(buf: &[u8]) -> Option<RecvFeedback> {
    if buf.len() < 20 {
        return None;
    }
    if buf[0] >> RTCP_VERSION_SHIFT != RTCP_VERSION || buf[1] != RTCP_PT_APP {
        return None;
    }
    if buf[8..12] != ZC_APP_NAME {
        return None;
    }
    if buf[12] != 1 {
        return None;
    }
    let frac = buf[13] as f64 / 255.0;
    let decode_lag_ms = u16::from_be_bytes([buf[14], buf[15]]) as f64;
    let jitter_ms = u16::from_be_bytes([buf[16], buf[17]]) as f64;
    let dropped_frames = u16::from_be_bytes([buf[18], buf[19]]);
    Some(RecvFeedback {
        fraction_lost: frac,
        jitter_ms,
        decode_lag_ms,
        dropped_frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtcp_sr_roundtrip() {
        let sr = build_rtcp_sr(0xA000_0001, 48_000, 3_900_000_000, 0);
        let anchor = parse_rtcp_sr(&sr).expect("parse SR");
        assert_eq!(anchor.ssrc, 0xA000_0001);
        assert_eq!(anchor.rtp_ts, 48_000);
    }

    #[test]
    fn parse_rtcp_sr_rejects_bad_version() {
        let mut sr = build_rtcp_sr(1, 0, 0, 0);
        sr[0] = 0x40; // V=1
        assert!(parse_rtcp_sr(&sr).is_none());
    }

    #[test]
    fn parse_rtcp_sr_rejects_short_length_field() {
        let mut sr = build_rtcp_sr(1, 0, 0, 0);
        // length=0 → claims 4-byte packet, shorter than SR minimum
        sr[2] = 0;
        sr[3] = 0;
        assert!(parse_rtcp_sr(&sr).is_none());
    }

    #[test]
    fn parse_rtcp_sr_rejects_truncated_buffer() {
        let sr = build_rtcp_sr(1, 0, 0, 0);
        assert!(parse_rtcp_sr(&sr[..20]).is_none());
    }

    #[test]
    fn recv_feedback_roundtrip() {
        let fb = RecvFeedback {
            fraction_lost: 0.12,
            jitter_ms: 8.0,
            decode_lag_ms: 120.0,
            dropped_frames: 3,
        };
        let pkt = build_recv_feedback(0xB000_0001, fb);
        let parsed = parse_recv_feedback(&pkt).expect("parse feedback");
        assert!((parsed.fraction_lost - fb.fraction_lost).abs() < 0.02);
        assert_eq!(parsed.jitter_ms, fb.jitter_ms);
        assert_eq!(parsed.decode_lag_ms, fb.decode_lag_ms);
        assert_eq!(parsed.dropped_frames, fb.dropped_frames);
    }
}
