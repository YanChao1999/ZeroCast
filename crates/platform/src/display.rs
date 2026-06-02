use anyhow::{Context, Result};

/// Primary monitor geometry (refresh rate when unknown defaults to 60 Hz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimaryDisplay {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

impl PrimaryDisplay {
    pub fn stub() -> Self {
        Self {
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        }
    }
}

/// Query the primary display size via `scrap` (desktop OSes).
pub fn primary_display() -> Result<PrimaryDisplay> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        use scrap::Display;
        let display = Display::primary().context("no primary display")?;
        let width = display.width() as u32;
        let height = display.height() as u32;
        if width == 0 || height == 0 {
            anyhow::bail!("invalid primary display size {width}x{height}");
        }
        // TODO(QoS): platform-specific refresh rate (DXGI/CGDisplay modes).
        Ok(PrimaryDisplay {
            width,
            height,
            refresh_hz: 60,
        })
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        Ok(PrimaryDisplay::stub())
    }
}
