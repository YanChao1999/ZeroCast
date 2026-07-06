use anyhow::{Context, Result};

use crate::native::NativeOpusEncoder;
use crate::source::PcmFrame;

/// Opus encoder for 20 ms frames (spec §5 v1.2 — raw Opus via `opus-rs`).
pub struct OpusEncoder {
    native: NativeOpusEncoder,
}

impl OpusEncoder {
    pub fn new_stereo() -> Result<Self> {
        Ok(Self {
            native: NativeOpusEncoder::new_stereo()?,
        })
    }

    pub fn with_channels(channels: u16) -> Result<Self> {
        Ok(Self {
            native: NativeOpusEncoder::with_channels(channels)?,
        })
    }

    /// Encode one PCM frame to a single raw Opus packet.
    pub fn encode(&mut self, frame: &PcmFrame) -> Result<Vec<u8>> {
        self.native.encode(frame).or_else(|e| {
            #[cfg(feature = "ffmpeg-opus")]
            {
                crate::encode_ffmpeg::encode_oneshot(frame)
                    .with_context(|| format!("native Opus failed ({e:#}); ffmpeg fallback"))
            }
            #[cfg(not(feature = "ffmpeg-opus"))]
            {
                Err(e)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SineSource;
    use std::process::{Command, Stdio};

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn encode_sine_frame() {
        let mut src = SineSource::new(440.0);
        let mut enc = OpusEncoder::new_stereo().expect("encoder");
        let pcm = src.next_frame();
        let opus = enc.encode(&pcm).expect("encode");
        assert!(!opus.is_empty());
        assert!(!crate::native::is_ogg_opus(&opus));
    }

    #[cfg(feature = "ffmpeg-opus")]
    #[test]
    fn ffmpeg_fallback_produces_ogg() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let mut src = SineSource::new(440.0);
        let pcm = src.next_frame();
        let ogg = crate::encode_ffmpeg::encode_oneshot(&pcm).expect("ffmpeg");
        assert!(crate::native::is_ogg_opus(&ogg));
    }
}
