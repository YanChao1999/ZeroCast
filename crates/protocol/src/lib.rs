//! ZeroCast protocol constants — normative spec: `docs/spec/std/zerocast-protocol-v1.md`.

pub const PROTOCOL_VERSION: u32 = 1;

/// Default UDP ports (§2).
pub mod ports {
    pub const VIDEO_RTP_DEFAULT: u16 = 5000;
    pub const AUDIO_RTP_DEFAULT: u16 = 5002;
    /// RTCP is always RTP port + 1.
    pub const RTCP_OFFSET: u16 = 1;
    /// Default audio RTP = video RTP + 2 when co-located.
    pub const AUDIO_OFFSET_FROM_VIDEO: u16 = 2;
}

/// RTP payload types and clock rates (§4, §5).
pub mod rtp {
    pub const PT_H264: u8 = 96;
    pub const PT_OPUS: u8 = 111;
    pub const VIDEO_CLOCK_HZ: u32 = 90_000;
    pub const AUDIO_CLOCK_HZ: u32 = 48_000;
    /// Opus frame: 20 ms @ 48 kHz.
    pub const AUDIO_SAMPLES_PER_FRAME: u32 = 960;
    pub const AUDIO_FRAME_MS: u32 = 20;
}

/// mDNS-SD (§3).
pub mod mdns {
    pub const SERVICE_TYPE: &str = "_zerocast._udp.local.";
    pub const TXT_V: &str = "v";
    pub const TXT_W: &str = "w";
    pub const TXT_H: &str = "h";
    pub const TXT_FPS: &str = "fps";
    pub const TXT_MAX_W: &str = "max_w";
    pub const TXT_MAX_H: &str = "max_h";
    pub const TXT_MAX_FPS: &str = "max_fps";
    pub const TXT_CLASS: &str = "class";
    pub const TXT_AUDIO: &str = "audio";
    pub const TXT_AUDIO_PORT: &str = "a_port";
    pub const TXT_AUDIO_SR: &str = "a_sr";
    pub const TXT_AUDIO_CH: &str = "a_ch";
}

/// Default Opus session parameters.
pub mod audio {
    pub const SAMPLE_RATE: u32 = super::rtp::AUDIO_CLOCK_HZ;
    pub const CHANNELS: u16 = 2;
    pub const DEFAULT_BITRATE: i32 = 64_000;
}

use anyhow::{Context, Result};
use std::net::SocketAddr;

/// RTCP port for an RTP bind port.
pub fn rtcp_port(rtp_port: u16) -> Option<u16> {
    rtp_port.checked_add(ports::RTCP_OFFSET)
}

/// Default audio RTP port when paired with a video port on the same host.
pub fn audio_port_from_video(video_port: u16) -> Option<u16> {
    video_port.checked_add(ports::AUDIO_OFFSET_FROM_VIDEO)
}

/// Build `host:audio_port` from a video target `host:video_port`.
pub fn audio_target_from_video(video_target: &str, audio_port_override: Option<u16>) -> Result<String> {
    let addr: SocketAddr = video_target
        .parse()
        .with_context(|| format!("invalid video target '{video_target}'"))?;
    let audio_port = audio_port_override.unwrap_or_else(|| {
        audio_port_from_video(addr.port()).unwrap_or(ports::AUDIO_RTP_DEFAULT)
    });
    Ok(format!("{}:{audio_port}", addr.ip()))
}

/// Local bind address for audio RTP given a video bind like `0.0.0.0:5000`.
pub fn audio_bind_from_video(video_bind: &str, audio_port_override: Option<u16>) -> Result<String> {
    let addr: SocketAddr = video_bind
        .parse()
        .or_else(|_| format!("0.0.0.0:{video_bind}").parse())
        .with_context(|| format!("invalid video bind '{video_bind}'"))?;
    let audio_port = audio_port_override.unwrap_or_else(|| {
        if addr.port() == 0 {
            // Match ephemeral video bind (`0.0.0.0:0`); avoid port 2 (0 + 2).
            0
        } else {
            audio_port_from_video(addr.port()).unwrap_or(ports::AUDIO_RTP_DEFAULT)
        }
    });
    Ok(format!("{}:{audio_port}", addr.ip()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_derivation() {
        assert_eq!(rtcp_port(5000), Some(5001));
        assert_eq!(audio_port_from_video(5000), Some(5002));
        assert_eq!(rtcp_port(5002), Some(5003));
    }

    #[test]
    fn audio_bind_ephemeral_video() {
        assert_eq!(
            audio_bind_from_video("0.0.0.0:0", None).unwrap(),
            "0.0.0.0:0"
        );
    }

    #[test]
    fn audio_target_derivation() {
        assert_eq!(
            audio_target_from_video("192.168.1.10:5000", None).unwrap(),
            "192.168.1.10:5002"
        );
        assert_eq!(
            audio_target_from_video("127.0.0.1:5000", Some(6000)).unwrap(),
            "127.0.0.1:6000"
        );
    }
}
