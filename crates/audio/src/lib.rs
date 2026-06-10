//! PCM sources and Opus codec for ZeroCast audio RTP (spec §5).

mod encode;
mod source;

pub use encode::OpusEncoder;
pub use source::{PcmFrame, SineSource, TestToneSource};

use zerocast_protocol::audio::{CHANNELS, SAMPLE_RATE};
use zerocast_protocol::rtp::AUDIO_SAMPLES_PER_FRAME;

/// Samples per channel in one 20 ms Opus frame @ 48 kHz.
pub const FRAME_SAMPLES: usize = AUDIO_SAMPLES_PER_FRAME as usize;

/// Interleaved i16 PCM frame size (stereo default).
pub fn interleaved_frame_bytes(channels: u16) -> usize {
    FRAME_SAMPLES * channels as usize * 2
}

/// Session sample rate (Hz).
pub fn sample_rate() -> u32 {
    SAMPLE_RATE
}

/// Default channel count.
pub fn channels() -> u16 {
    CHANNELS
}
