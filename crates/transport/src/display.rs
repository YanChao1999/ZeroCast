//! Minimal RGB viewer (minifb).

use std::sync::mpsc::Receiver;
use std::time::Duration;

pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub rgb24: Vec<u8>,
}

/// Blocking minifb loop on the current thread.
pub fn run_blocking(
    frame_rx: Receiver<RgbFrame>,
    window_w: u32,
    window_h: u32,
    title: &str,
) -> anyhow::Result<()> {
    let mut window = minifb::Window::new(
        title,
        window_w as usize,
        window_h as usize,
        minifb::WindowOptions::default(),
    )
    .map_err(|e| anyhow::anyhow!("failed to open window: {e}"))?;

    let mut buffer = vec![0u32; (window_w as usize) * (window_h as usize)];
    let mut frames_shown = 0u64;

    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        match frame_rx.recv_timeout(Duration::from_millis(16)) {
            Ok(frame) => {
                blit_rgb24(
                    &mut buffer,
                    window_w as usize,
                    window_h as usize,
                    &frame.rgb24,
                    frame.width as usize,
                    frame.height as usize,
                );
                frames_shown += 1;
                if frames_shown == 1 || frames_shown % 60 == 0 {
                    eprintln!("display: showing frame {frames_shown}");
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        window
            .update_with_buffer(&buffer, window_w as usize, window_h as usize)
            .map_err(|e| anyhow::anyhow!("window update failed: {e}"))?;
    }

    Ok(())
}

fn blit_rgb24(
    dst: &mut [u32],
    dst_w: usize,
    dst_h: usize,
    rgb: &[u8],
    src_w: usize,
    src_h: usize,
) {
    let expected = src_w.saturating_mul(src_h).saturating_mul(3);
    if rgb.len() < expected {
        return;
    }
    dst.fill(0);
    let w = dst_w.min(src_w);
    let h = dst_h.min(src_h);
    for y in 0..h {
        for x in 0..w {
            let si = (y * src_w + x) * 3;
            let r = rgb[si] as u32;
            let g = rgb[si + 1] as u32;
            let b = rgb[si + 2] as u32;
            dst[y * dst_w + x] = (r << 16) | (g << 8) | b;
        }
    }
}
