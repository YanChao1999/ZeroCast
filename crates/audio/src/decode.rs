use anyhow::Result;

use crate::native::{is_ogg_opus, NativeOpusDecoder};
use crate::source::PcmFrame;

/// Opus decoder for 20 ms frames (raw Opus v1.2; Ogg fallback for v1.1).
pub struct OpusDecoder {
    native: NativeOpusDecoder,
}

impl OpusDecoder {
    pub fn new_stereo() -> Result<Self> {
        Ok(Self {
            native: NativeOpusDecoder::new_stereo()?,
        })
    }

    pub fn with_channels(channels: u16) -> Result<Self> {
        Ok(Self {
            native: NativeOpusDecoder::with_channels(channels)?,
        })
    }

    /// Decode one Opus packet to interleaved PCM (one 20 ms frame).
    pub fn decode(&mut self, opus: &[u8]) -> Result<PcmFrame> {
        if is_ogg_opus(opus) {
            #[cfg(feature = "ffmpeg-opus")]
            {
                return crate::decode_ffmpeg::decode_oneshot(opus, self.native.channels());
            }
            #[cfg(not(feature = "ffmpeg-opus"))]
            anyhow::bail!("Ogg Opus payload requires `ffmpeg-opus` feature");
        }
        self.native.decode(opus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::OpusEncoder;
    use crate::source::SineSource;

    #[test]
    fn decode_sine_roundtrip() {
        let mut src = SineSource::new(440.0);
        let mut enc = OpusEncoder::new_stereo().expect("encoder");
        let mut dec = OpusDecoder::new_stereo().expect("decoder");
        let pcm = src.next_frame();
        let opus = enc.encode(&pcm).expect("encode");
        let out = dec.decode(&opus).expect("decode");
        assert_eq!(out.samples.len(), pcm.samples.len());
    }
}
