//! ffmpeg one-shot Opus encode (Ogg pages) — v1.1 interop fallback.

use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};
use zerocast_protocol::audio::SAMPLE_RATE;

use crate::source::PcmFrame;

pub fn encode_oneshot(frame: &PcmFrame) -> Result<Vec<u8>> {
    let channels = frame.channels;
    let expected = crate::FRAME_SAMPLES * channels as usize;
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
            &SAMPLE_RATE.to_string(),
            "-ac",
            &channels.to_string(),
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
