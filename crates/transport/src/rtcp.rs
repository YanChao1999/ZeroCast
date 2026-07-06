//! Minimal RTCP Sender Report (PT=200) helpers (RFC 3550).

const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

/// Parsed RTCP Sender Report anchor for A/V sync.
#[derive(Debug, Clone, Copy)]
pub struct RtcpSrAnchor {
    pub ssrc: u32,
    pub rtp_ts: u32,
    pub wall_ns: u128,
}

/// Parse an RTCP SR from a UDP payload. Returns `None` if not a valid SR.
pub fn parse_rtcp_sr(buf: &[u8]) -> Option<RtcpSrAnchor> {
    if buf.len() < 28 || buf[1] != 200 {
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
    let frac_ns = ((ntp_frac as u128) * 1_000_000_000u128) / (1u128 << 32);
    unix_secs.saturating_mul(1_000_000_000u128).saturating_add(frac_ns)
}

/// Build a minimal RTCP SR (28 bytes).
pub fn build_rtcp_sr(ssrc: u32, rtp_ts: u32, ntp_secs: u32, ntp_frac: u32) -> [u8; 28] {
    let length: u16 = 6;
    let mut sr = [0u8; 28];
    sr[0] = 0x80;
    sr[1] = 200;
    sr[2] = ((length >> 8) & 0xFF) as u8;
    sr[3] = (length & 0xFF) as u8;
    sr[4..8].copy_from_slice(&ssrc.to_be_bytes());
    sr[8..12].copy_from_slice(&ntp_secs.to_be_bytes());
    sr[12..16].copy_from_slice(&ntp_frac.to_be_bytes());
    sr[16..20].copy_from_slice(&rtp_ts.to_be_bytes());
    sr
}

pub fn wall_to_ntp(now_ns: u128) -> (u32, u32) {
    let unix_secs = (now_ns / 1_000_000_000) as u64;
    let unix_nanos = (now_ns % 1_000_000_000) as u128;
    let ntp_secs = unix_secs.saturating_add(NTP_UNIX_OFFSET) as u32;
    let ntp_frac = ((unix_nanos * (1u128 << 32)) / 1_000_000_000u128) as u32;
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
}
