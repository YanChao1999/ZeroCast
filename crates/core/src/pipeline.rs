//! Media pipeline copy tiers — guides zero-copy work on sender and receiver.

/// How much CPU memory traffic the hot path uses (lower is better).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CopyTier {
    /// Capture RGB24 → ffmpeg CPU encode → RTP → ffmpeg CPU decode → RGB24 minifb.
    FullCpu = 0,
    /// HW encode/decode; one GPU or DMA blit to display (e.g. NV12 → KMS).
    HwAccel = 1,
    /// Textures / dmabuf end-to-end; no full-frame CPU RGB (target for desktop + Pi 4+).
    ZeroCopy = 2,
}

/// Where decoded pixels should go on the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverOutput {
    /// Desktop MVP: minifb window (CPU RGB24 buffer).
    CpuWindow,
    /// Linux embedded: DRM/KMS direct scanout (HDMI/DSI).
    DrmKms,
    /// GPU compositor texture (wgpu/GL/VAAPI export).
    GpuTexture,
}

impl ReceiverOutput {
    /// Best target for a device class (QoS / platform picker).
    pub fn for_embedded_pi_zero_w() -> Self {
        // Zero W: no reliable zero-copy decode; prefer DRM if available else CPU scale to small fb.
        Self::DrmKms
    }

    pub fn preferred_copy_tier(self) -> CopyTier {
        match self {
            Self::CpuWindow => CopyTier::FullCpu,
            Self::DrmKms | Self::GpuTexture => CopyTier::HwAccel,
        }
    }
}

/// Sender-side capture → encode path (future `crates/platform` + `crates/media`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderCapture {
    /// scrap → CPU RGB24 scale (today).
    CpuRgb24,
    /// DXGI / ScreenCaptureKit / PipeWire → GPU texture.
    GpuTexture,
}

impl SenderCapture {
    pub fn preferred_copy_tier(self) -> CopyTier {
        match self {
            Self::CpuRgb24 => CopyTier::FullCpu,
            Self::GpuTexture => CopyTier::ZeroCopy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drm_prefers_hw_accel() {
        assert_eq!(
            ReceiverOutput::DrmKms.preferred_copy_tier(),
            CopyTier::HwAccel
        );
    }
}
