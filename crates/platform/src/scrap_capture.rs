use anyhow::{Context, Result};
use scrap::{Capturer, Display};
use std::io::ErrorKind::WouldBlock;
use std::thread;
use std::time::Duration;

pub struct ScrapCapturer {
    capturer: Capturer,
    native_width: u32,
    native_height: u32,
    out_width: u32,
    out_height: u32,
    scratch: Vec<u8>,
    /// Last good RGB24 frame (DXGI returns WouldBlock when the desktop is unchanged).
    last_frame: Option<Vec<u8>>,
}

impl ScrapCapturer {
    pub fn open(out_width: u32, out_height: u32) -> Result<Self> {
        let display = Display::primary().context("no primary display")?;
        let capturer = Capturer::new(display).context("failed to open screen capturer")?;
        let native_width = capturer.width() as u32;
        let native_height = capturer.height() as u32;
        if native_width == 0 || native_height == 0 {
            anyhow::bail!("invalid display size {native_width}x{native_height}");
        }

        Ok(Self {
            capturer,
            native_width,
            native_height,
            out_width,
            out_height,
            scratch: Vec::new(),
            last_frame: None,
        })
    }

    fn reopen(&mut self) -> Result<()> {
        let display = Display::primary().context("no primary display")?;
        self.capturer = Capturer::new(display).context("failed to reopen screen capturer")?;
        self.native_width = self.capturer.width() as u32;
        self.native_height = self.capturer.height() as u32;
        Ok(())
    }

    pub fn native_width(&self) -> u32 {
        self.native_width
    }

    pub fn native_height(&self) -> u32 {
        self.native_height
    }

    pub fn capture_frame(&mut self) -> Result<Vec<u8>> {
        const SPIN: Duration = Duration::from_millis(1);
        /// Wait briefly for DXGI to deliver a new desktop frame.
        const MAX_WAIT_NEW: Duration = Duration::from_millis(250);
        let deadline = std::time::Instant::now() + MAX_WAIT_NEW;

        loop {
            match self.capturer.frame() {
                Ok(frame) => {
                    let src: &[u8] = &*frame;
                    let src_h = self.native_height as usize;
                    let src_stride = if src_h > 0 {
                        src.len() / src_h
                    } else {
                        0
                    };
                    let rgb = scale_bgra_to_rgb24(
                        src,
                        self.native_width as usize,
                        src_h,
                        src_stride,
                        self.out_width,
                        self.out_height,
                        &mut self.scratch,
                    );
                    self.last_frame = Some(rgb.clone());
                    return Ok(rgb);
                }
                Err(e) if e.kind() == WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        if let Some(last) = self.last_frame.clone() {
                            return Ok(last);
                        }
                        // No frame yet — keep polling a bit longer on first capture.
                        thread::sleep(SPIN);
                        if std::time::Instant::now()
                            >= deadline + Duration::from_secs(2)
                        {
                            self.reopen().context("reopen after initial capture stall")?;
                            anyhow::bail!(
                                "screen capture timed out waiting for first frame"
                            );
                        }
                        continue;
                    }
                    thread::sleep(SPIN);
                }
                Err(e) => {
                    if self.last_frame.is_some() {
                        eprintln!("screen capture: {e:#}, reusing last frame");
                        let _ = self.reopen();
                        return Ok(self.last_frame.clone().unwrap());
                    }
                    return Err(e).context("screen capture failed");
                }
            }
        }
    }
}

fn scale_bgra_to_rgb24(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    src_stride: usize,
    dst_w: u32,
    dst_h: u32,
    scratch: &mut Vec<u8>,
) -> Vec<u8> {
    let dst_w = dst_w as usize;
    let dst_h = dst_h as usize;
    let out_len = dst_w * dst_h * 3;
    scratch.clear();
    scratch.resize(out_len, 0);

    if src_w == dst_w && src_h == dst_h {
        for y in 0..dst_h {
            let row = y * src_stride;
            for x in 0..dst_w {
                let si = row + x * 4;
                let di = (y * dst_w + x) * 3;
                if si + 3 < src.len() {
                    scratch[di] = src[si + 2];
                    scratch[di + 1] = src[si + 1];
                    scratch[di + 2] = src[si];
                }
            }
        }
        return scratch.clone();
    }

    for dy in 0..dst_h {
        let sy = dy * src_h / dst_h;
        let row = sy * src_stride;
        for dx in 0..dst_w {
            let sx = dx * src_w / dst_w;
            let si = row + sx * 4;
            let di = (dy * dst_w + dx) * 3;
            if si + 3 < src.len() {
                scratch[di] = src[si + 2];
                scratch[di + 1] = src[si + 1];
                scratch[di + 2] = src[si];
            }
        }
    }
    scratch.clone()
}
