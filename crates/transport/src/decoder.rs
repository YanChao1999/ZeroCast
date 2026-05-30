//! H.264 decode via ffmpeg CLI (Approach A, symmetric to encoder).

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Decode one access unit (Annex-B NALs concatenated) to RGB24.
/// `hint_width` / `hint_height` set the output size (should match the sender).
pub fn decode_access_unit_rgb24(
    annex_b: &[u8],
    hint_width: u32,
    hint_height: u32,
) -> Result<(Vec<u8>, u32, u32)> {
    if annex_b.is_empty() {
        bail!("empty access unit");
    }

    let scale = format!("scale={hint_width}:{hint_height}:flags=fast_bilinear");
    let mut child = Command::new("ffmpeg");
    child
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
            "-f",
            "h264",
            "-i",
            "-",
            "-an",
            "-vf",
            &scale,
            "-pix_fmt",
            "rgb24",
            "-vframes",
            "1",
            "-f",
            "rawvideo",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = child.spawn().context("failed to spawn ffmpeg decoder")?;
    {
        let mut stdin = child.stdin.take().context("ffmpeg stdin missing")?;
        stdin.write_all(annex_b)?;
    }

    let output = child
        .wait_with_output()
        .context("failed to wait for ffmpeg decoder")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ffmpeg decode failed: {stderr}");
    }

    let expected = (hint_width as usize) * (hint_height as usize) * 3;
    let rgb = output.stdout;
    if rgb.len() < expected {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ffmpeg decode short read: got {} bytes, expected {} ({}x{}); stderr: {}",
            rgb.len(),
            expected,
            hint_width,
            hint_height,
            stderr.trim()
        );
    }
    Ok((rgb[..expected].to_vec(), hint_width, hint_height))
}
