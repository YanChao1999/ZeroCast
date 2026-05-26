use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Encode a single RGB24 frame to H.264 (Annex-B) using the `ffmpeg` CLI.
/// Returns a vector of Annex-B NALUs (each NALU includes the start code 0x00 00 00 01).
///
/// Notes:
/// - Requires `ffmpeg` binary available in PATH.
/// - This is a pragmatic replacement for an ffmpeg-next integration.
/// Encode a single RGB24 frame to H.264 (Annex-B) using the `ffmpeg` CLI.
/// Returns a vector of Annex-B NALUs (each NALU includes the start code 0x00 00 00 01)
/// and a frame PTS in nanoseconds (approx). If ffmpeg is not available the function
/// returns a synthetic large NALU and a PTS computed from `frame_index` and 30 FPS.
pub fn encode_frame_to_h264_annexb(rgb24: &[u8], width: u32, height: u32, frame_index: u64) -> Result<(Vec<Vec<u8>>, u128)> {
    // Build ffmpeg command to read raw RGB24 from stdin and output raw H264 (Annex-B) to stdout
    let size_arg = format!("{}x{}", width, height);
    // If built with the optional `libav` feature, prefer a libav-based encoder
    // (ffmpeg-next) for precise PTS extraction. This path is intentionally
    // unimplemented here so the feature remains opt-in until fully integrated.
    #[cfg(feature = "libav")]
    {
        return Err(anyhow::anyhow!("libav encoding path enabled but not yet implemented; please implement ffmpeg-next integration or disable the `libav` feature"));
    }
    let child_spawn = Command::new("ffmpeg")
        .args(&[
            "-hide_banner",
            "-loglevel",
            "quiet",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-f",
            "h264",
            "pipe:1",
            "-progress",
            "pipe:2",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "zerolatency",
            "-f",
            "h264",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();

    let mut child = match child_spawn {
        Ok(c) => c,
        Err(e) => {
            // If ffmpeg isn't available, fall back to a synthetic large NALU for development/testing
            if e.kind() == std::io::ErrorKind::NotFound {
                let mut nalus: Vec<Vec<u8>> = Vec::new();
                // start code + fake SPS
                nalus.push(vec![0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f]);
                // generate a large fake IDR NALU to trigger FU-A fragmentation
                let mut idr = vec![0, 0, 0, 1, 0x65];
                // append payload to exceed common MTU
                idr.extend(std::iter::repeat(0xAAu8).take((width as usize * height as usize) / 2));
                // compute PTS from frame_index assuming 30fps
                let fps = 30u128;
                let pts_ns = (frame_index as u128) * 1_000_000_000u128 / fps;
                return Ok((nalus, pts_ns));
            } else {
                return Err(e).context("failed to spawn ffmpeg")?;
            }
        }
    };

    // Write raw frame to ffmpeg stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(rgb24)
            .context("failed to write frame to ffmpeg stdin")?;
        // close stdin to signal EOF
        drop(stdin);
    }

    // Read ffmpeg stdout
    let output = child
        .wait_with_output()
        .context("failed to wait for ffmpeg output")?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("ffmpeg failed: exit={}", output.status));
    }
    let bytes = output.stdout;
    // try to parse out_time_ms from ffmpeg -progress output on stderr (last occurrence)
    let mut pts_ns: Option<u128> = None;
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if line.starts_with("out_time_ms=") {
            if let Some(ms_str) = line.split_once('=') {
                if let Ok(ms) = ms_str.1.trim().parse::<u128>() {
                    pts_ns = Some(ms * 1_000_000u128);
                }
            }
        }
    }

    // Split by Annex-B start code (0x00 00 00 01), keeping the start code on each NALU
    let start_code: &[u8] = &[0, 0, 0, 1];
    let mut nalus: Vec<Vec<u8>> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        // find next start code
        if let Some(pos) = bytes[i..].windows(start_code.len()).position(|w| w == start_code) {
            let pos = i + pos;
            // find following start code
            let next_search_start = pos + start_code.len();
            if let Some(next_pos_rel) = bytes[next_search_start..].windows(start_code.len()).position(|w| w == start_code) {
                let next_pos = next_search_start + next_pos_rel;
                nalus.push(bytes[pos..next_pos].to_vec());
                i = next_pos;
            } else {
                // last NALU until EOF
                nalus.push(bytes[pos..].to_vec());
                break;
            }
        } else {
            break;
        }
    }

    // compute approximate PTS from frame_index at 30fps as fallback
    let fps = 30u128;
    let fallback_pts = (frame_index as u128) * 1_000_000_000u128 / fps;
    Ok((nalus, pts_ns.unwrap_or(fallback_pts)))
}
