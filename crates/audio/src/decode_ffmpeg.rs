//! ffmpeg one-shot Opus decode for Ogg payloads (v1.1 interop).

use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};
use zerocast_protocol::audio::SAMPLE_RATE;

use crate::source::PcmFrame;

pub fn decode_oneshot(opus: &[u8], channels: u16) -> Result<PcmFrame> {
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
            &SAMPLE_RATE.to_string(),
            "-ac",
            &channels.to_string(),
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
    let expected_bytes = crate::FRAME_SAMPLES * channels as usize * 2;
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
        channels,
    })
}
