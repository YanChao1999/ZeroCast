//! Persistent Opus encode/decode via pure-Rust `opus-rs` (raw RFC 6716 packets, v1.2).

use anyhow::Result;
use opus_rs::{Application, OpusDecoder as NativeDecoder, OpusEncoder as NativeEncoder};
use zerocast_protocol::audio::{CHANNELS, SAMPLE_RATE};

use crate::source::PcmFrame;

const MAX_PACKET_BYTES: usize = 4000;

/// Persistent Opus encoder (one 20 ms frame per `encode` call, no subprocess spawn).
pub struct NativeOpusEncoder {
    inner: NativeEncoder,
    channels: u16,
    frame_samples: usize,
    out_buf: Vec<u8>,
}

impl NativeOpusEncoder {
    pub fn new_stereo() -> Result<Self> {
        Self::with_channels(CHANNELS)
    }

    pub fn with_channels(channels: u16) -> Result<Self> {
        if channels != 1 && channels != 2 {
            anyhow::bail!("unsupported channel count {channels}");
        }
        let ch = channels as usize;
        let mut inner = NativeEncoder::new(
            SAMPLE_RATE as i32,
            ch,
            Application::RestrictedLowDelay,
        )
        .map_err(|e| anyhow::anyhow!("Opus encoder init: {e}"))?;
        inner.bitrate_bps = zerocast_protocol::audio::DEFAULT_BITRATE;
        inner.use_cbr = true;
        Ok(Self {
            inner,
            channels,
            frame_samples: crate::FRAME_SAMPLES,
            out_buf: vec![0u8; MAX_PACKET_BYTES],
        })
    }

    pub fn encode(&mut self, frame: &PcmFrame) -> Result<Vec<u8>> {
        let expected = self.frame_samples * self.channels as usize;
        if frame.samples.len() != expected {
            anyhow::bail!(
                "PCM frame length {} != expected {expected}",
                frame.samples.len()
            );
        }
        let float_pcm: Vec<f32> = frame
            .samples
            .iter()
            .map(|&s| s as f32 / i16::MAX as f32)
            .collect();
        let n = self
            .inner
            .encode(&float_pcm, self.frame_samples, &mut self.out_buf)
            .map_err(|e| anyhow::anyhow!("Opus encode: {e}"))?;
        Ok(self.out_buf[..n].to_vec())
    }
}

/// Persistent Opus decoder for raw Opus RTP payloads (not Ogg).
pub struct NativeOpusDecoder {
    inner: NativeDecoder,
    channels: u16,
    frame_samples: usize,
    float_buf: Vec<f32>,
}

impl NativeOpusDecoder {
    pub fn new_stereo() -> Result<Self> {
        Self::with_channels(CHANNELS)
    }

    pub fn with_channels(channels: u16) -> Result<Self> {
        if channels != 1 && channels != 2 {
            anyhow::bail!("unsupported channel count {channels}");
        }
        let ch = channels as usize;
        let inner = NativeDecoder::new(SAMPLE_RATE as i32, ch)
            .map_err(|e| anyhow::anyhow!("Opus decoder init: {e}"))?;
        Ok(Self {
            inner,
            channels,
            frame_samples: crate::FRAME_SAMPLES,
            float_buf: vec![0.0f32; crate::FRAME_SAMPLES * ch],
        })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn decode(&mut self, opus: &[u8]) -> Result<PcmFrame> {
        if opus.is_empty() {
            anyhow::bail!("empty Opus payload");
        }
        let samples = self
            .inner
            .decode(opus, self.frame_samples, &mut self.float_buf)
            .map_err(|e| anyhow::anyhow!("Opus decode: {e}"))?;
        let need = samples * self.channels as usize;
        if need > self.float_buf.len() {
            anyhow::bail!("decoder returned unexpected sample count {samples}");
        }
        let pcm: Vec<i16> = self.float_buf[..need]
            .iter()
            .map(|&f| (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
        Ok(PcmFrame {
            samples: pcm,
            channels: self.channels,
        })
    }
}

/// True when payload is an Ogg Opus page (v1.1 ffmpeg) rather than raw Opus (v1.2).
pub fn is_ogg_opus(payload: &[u8]) -> bool {
    payload.len() >= 4 && &payload[..4] == b"OggS"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SineSource;

    #[test]
    fn native_sine_roundtrip() {
        let mut src = SineSource::new(440.0);
        let mut enc = NativeOpusEncoder::new_stereo().expect("encoder");
        let mut dec = NativeOpusDecoder::new_stereo().expect("decoder");
        let pcm = src.next_frame();
        let opus = enc.encode(&pcm).expect("encode");
        assert!(!is_ogg_opus(&opus));
        let out = dec.decode(&opus).expect("decode");
        assert_eq!(out.samples.len(), pcm.samples.len());
    }
}
