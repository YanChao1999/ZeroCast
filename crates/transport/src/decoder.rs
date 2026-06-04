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

    let scale = format!(
        "scale={hint_width}:{hint_height}:force_original_aspect_ratio=disable:flags=fast_bilinear"
    );
    let mut child = Command::new("ffmpeg");
    child
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-probesize",
            "65536",
            "-analyzeduration",
            "500000",
            "-flags",
            "low_delay",
            "-f",
            "h264",
            "-c:v",
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
        stdin.flush().ok();
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
    if rgb.len() == expected {
        return Ok((rgb, hint_width, hint_height));
    }
    // Wrong stride if we truncated a full-resolution buffer — scale down instead.
    let Some((src_w, src_h)) = infer_rgb24_dimensions(rgb.len()) else {
        bail!(
            "ffmpeg decode size mismatch: got {} bytes, expected {} ({}x{})",
            rgb.len(),
            expected,
            hint_width,
            hint_height
        );
    };
    let scaled = scale_rgb24_nearest(&rgb, src_w, src_h, hint_width, hint_height);
    Ok((scaled, hint_width, hint_height))
}

fn infer_rgb24_dimensions(len: usize) -> Option<(u32, u32)> {
    if len % 3 != 0 {
        return None;
    }
    let px = len / 3;
    for w in [1920u32, 1680, 1600, 1440, 1366, 1280, 1024, 854, 800, 720, 640, 480, 426, 320] {
        let w = w as usize;
        if w == 0 || px % w != 0 {
            continue;
        }
        let h = px / w;
        if (1..=4320).contains(&h) {
            return Some((w as u32, h as u32));
        }
    }
    None
}

fn scale_rgb24_nearest(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let src_w = src_w as usize;
    let src_h = src_h as usize;
    let dst_w = dst_w as usize;
    let dst_h = dst_h as usize;
    let mut out = vec![0u8; dst_w * dst_h * 3];
    for dy in 0..dst_h {
        let sy = dy * src_h / dst_h;
        for dx in 0..dst_w {
            let sx = dx * src_w / dst_w;
            let si = (sy * src_w + sx) * 3;
            let di = (dy * dst_w + dx) * 3;
            if si + 2 < src.len() {
                out[di] = src[si];
                out[di + 1] = src[si + 1];
                out[di + 2] = src[si + 2];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_1080p_rgb24() {
        let (w, h) = infer_rgb24_dimensions(1920 * 1080 * 3).unwrap();
        assert_eq!((w, h), (1920, 1080));
    }
}
