use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};
use zerocast_protocol::audio::{CHANNELS, SAMPLE_RATE};

use crate::source::PcmFrame;

/// Opus decoder for 20 ms frames via ffmpeg `libopus` (spec §5).
/// Expects Ogg Opus pages matching the v1.1 ffmpeg encoder output.
pub struct OpusDecoder {
    channels: u16,
    sample_rate: u32,
}

impl OpusDecoder {
    pub fn new_stereo() -> Result<Self> {
        Self::with_channels(CHANNELS)
    }

    pub fn with_channels(channels: u16) -> Result<Self> {
        if channels != 1 && channels != 2 {
            anyhow::bail!("unsupported channel count {channels}");
        }
        Ok(Self {
            channels,
            sample_rate: SAMPLE_RATE,
        })
    }

    /// Decode one Opus packet to interleaved PCM (one 20 ms frame).
    pub fn decode(&self, opus: &[u8]) -> Result<PcmFrame> {
        if opus.is_empty() {
            anyhow::bail!("empty Opus payload");
        }
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "ogg",
                "-i",
                "pipe:0",
                "-f",
                "s16le",
                "-ar",
                &self.sample_rate.to_string(),
                "-ac",
                &self.channels.to_string(),
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn ffmpeg for Opus decode (is ffmpeg on PATH?)")?;
        {
            let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
            stdin.write_all(opus)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ffmpeg Opus decode failed: {stderr}");
        }
        let expected_bytes = crate::FRAME_SAMPLES * self.channels as usize * 2;
        if output.stdout.len() < expected_bytes {
            anyhow::bail!(
                "decoded PCM too short ({} < {expected_bytes} bytes)",
                output.stdout.len()
            );
        }
        let samples: Vec<i16> = output.stdout[..expected_bytes]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(PcmFrame {
            samples,
            channels: self.channels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::OpusEncoder;
    use crate::source::SineSource;
    use std::process::{Command, Stdio};

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn decode_sine_roundtrip() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let mut src = SineSource::new(440.0);
        let enc = OpusEncoder::new_stereo().expect("encoder");
        let dec = OpusDecoder::new_stereo().expect("decoder");
        let pcm = src.next_frame();
        let opus = enc.encode(&pcm).expect("encode");
        let out = dec.decode(&opus).expect("decode");
        assert_eq!(out.samples.len(), pcm.samples.len());
    }
}
