use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Encode a single RGB24 frame to H.264 (Annex-B) using the `ffmpeg` CLI.
/// Returns a vector of Annex-B NALUs (each NALU includes the start code 0x00 00 00 01).
///
/// Notes:
/// - Requires `ffmpeg` binary available in PATH.
/// - This is a pragmatic replacement for an ffmpeg-next integration.
pub fn encode_frame_to_h264_annexb(rgb24: &[u8], width: u32, height: u32, _frame_index: u64) -> Result<Vec<Vec<u8>>> {
    // Build ffmpeg command to read raw RGB24 from stdin and output raw H264 (Annex-B) to stdout
    let size_arg = format!("{}x{}", width, height);
    let mut child = Command::new("ffmpeg")
        .args(&[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            &size_arg,
            "-i",
            "-",
            "-c:v",
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
        .spawn()
        .context("failed to spawn ffmpeg; ensure ffmpeg is installed and in PATH")?;

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

    Ok(nalus)
}
