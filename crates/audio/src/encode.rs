use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};
use zerocast_protocol::audio::{CHANNELS, SAMPLE_RATE};

use crate::source::PcmFrame;

/// Opus encoder for 20 ms frames via ffmpeg `libopus` (spec §5).
pub struct OpusEncoder {
    channels: u16,
    sample_rate: u32,
}

impl OpusEncoder {
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

    /// Encode one PCM frame to a single Opus packet.
    pub fn encode(&self, frame: &PcmFrame) -> Result<Vec<u8>> {
        let expected = crate::FRAME_SAMPLES * self.channels as usize;
        if frame.samples.len() != expected {
            anyhow::bail!(
                "PCM frame length {} != expected {expected}",
                frame.samples.len()
            );
        }
        let mut pcm_bytes = Vec::with_capacity(frame.samples.len() * 2);
        for s in &frame.samples {
            pcm_bytes.extend_from_slice(&s.to_le_bytes());
        }
        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "s16le",
                "-ar",
                &self.sample_rate.to_string(),
                "-ac",
                &self.channels.to_string(),
                "-i",
                "pipe:0",
                "-c:a",
                "libopus",
                "-application",
                "lowdelay",
                "-b:a",
                "64k",
                "-frame_duration",
                "20",
                "-f",
                "opus",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn ffmpeg for Opus encode (is ffmpeg on PATH?)")?;
        {
            let mut stdin = child.stdin.take().context("ffmpeg stdin")?;
            stdin.write_all(&pcm_bytes)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ffmpeg Opus encode failed: {stderr}");
        }
        Ok(output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SineSource;

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
    fn encode_sine_frame() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let mut src = SineSource::new(440.0);
        let enc = OpusEncoder::new_stereo().expect("encoder");
        let pcm = src.next_frame();
        let opus = enc.encode(&pcm).expect("encode");
        assert!(!opus.is_empty());
    }
}
