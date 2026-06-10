//! Cross-platform screen capture behind a small trait boundary.
//!
//! Desktop OSes use `scrap` (DXGI / CGDisplay / X11). Other targets use a synthetic stub.

mod stub;

#[cfg(all(feature = "capture", any(windows, target_os = "macos", target_os = "linux")))]
mod scrap_capture;

mod display;

pub use display::{primary_display, PrimaryDisplay};

use anyhow::{bail, Result};

/// Captures screen pixels as RGB24 for the video encoder.
pub struct ScreenCapture {
    inner: CaptureInner,
    width: u32,
    height: u32,
}

enum CaptureInner {
    #[cfg(all(feature = "capture", any(windows, target_os = "macos", target_os = "linux")))]
    Scrap(scrap_capture::ScrapCapturer),
    Stub(stub::StubCapturer),
}

impl ScreenCapture {
    /// Open primary display capture scaled to `width` x `height` (RGB24).
    pub fn open(width: u32, height: u32) -> Result<Self> {
        assert!(width > 0 && height > 0, "width and height must be positive");

        #[cfg(all(feature = "capture", any(windows, target_os = "macos", target_os = "linux")))]
        {
            match scrap_capture::ScrapCapturer::open(width, height) {
                Ok(scrap) => {
                    eprintln!(
                        "screen capture: scaling {}x{} to {}x{}",
                        scrap.native_width(),
                        scrap.native_height(),
                        width,
                        height
                    );
                    return Ok(Self {
                        inner: CaptureInner::Scrap(scrap),
                        width,
                        height,
                    });
                }
                Err(e) => {
                    eprintln!("screen capture unavailable ({e:#}), using synthetic pattern");
                }
            }
        }

        Ok(Self {
            inner: CaptureInner::Stub(stub::StubCapturer::new(width, height)),
            width,
            height,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Resize captured output without reopening the OS capturer (QoS hot reconfigure).
    pub fn reconfigure(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            bail!("width and height must be positive");
        }
        match &mut self.inner {
            #[cfg(all(feature = "capture", any(windows, target_os = "macos", target_os = "linux")))]
            CaptureInner::Scrap(s) => {
                s.set_output_size(width, height);
            }
            CaptureInner::Stub(s) => s.set_output_size(width, height),
        }
        self.width = width;
        self.height = height;
        eprintln!("screen capture: output resized to {}x{}", width, height);
        Ok(())
    }

    /// One RGB24 frame (`width * height * 3` bytes).
    pub fn capture_frame(&mut self) -> Result<Vec<u8>> {
        match &mut self.inner {
            #[cfg(all(feature = "capture", any(windows, target_os = "macos", target_os = "linux")))]
            CaptureInner::Scrap(s) => s.capture_frame(),
            CaptureInner::Stub(s) => s.capture_frame(),
        }
    }
}
