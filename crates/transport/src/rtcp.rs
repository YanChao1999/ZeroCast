//! Minimal RTCP Sender Report (PT=200) helpers (RFC 3550).

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
}
