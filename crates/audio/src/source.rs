use anyhow::Result;
#[cfg(feature = "capture")]
use anyhow::Context;
use zerocast_protocol::audio::{CHANNELS, SAMPLE_RATE};

use crate::FRAME_SAMPLES;

/// One 20 ms PCM frame (interleaved i16).
pub struct PcmFrame {
    pub samples: Vec<i16>,
    pub channels: u16,
}

impl PcmFrame {
    pub fn stereo(samples: Vec<i16>) -> Self {
        Self {
            samples,
            channels: 2,
        }
    }
}

/// Synthetic test tone (440 Hz sine) for transport smoke tests.
pub struct SineSource {
    phase: f64,
    frequency_hz: f64,
    sample_rate: u32,
    channels: u16,
    frame_index: u64,
}

impl SineSource {
    pub fn new(frequency_hz: f64) -> Self {
        Self {
            phase: 0.0,
            frequency_hz,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            frame_index: 0,
        }
    }

    pub fn next_frame(&mut self) -> PcmFrame {
        let n = FRAME_SAMPLES;
        let mut samples = Vec::with_capacity(n * self.channels as usize);
        let phase_inc = 2.0 * std::f64::consts::PI * self.frequency_hz / self.sample_rate as f64;
        for i in 0..n {
            let t = self.frame_index * n as u64 + i as u64;
            let s = (self.phase + phase_inc * t as f64).sin();
            let amp = (s * 0.25 * i16::MAX as f64) as i16;
            for _ in 0..self.channels {
                samples.push(amp);
            }
        }
        self.frame_index += 1;
        PcmFrame {
            samples,
            channels: self.channels,
        }
    }
}

/// Trait for PCM producers (sine, cpal, …).
pub trait TestToneSource {
    fn next_pcm_frame(&mut self) -> Result<PcmFrame>;
}

impl TestToneSource for SineSource {
    fn next_pcm_frame(&mut self) -> Result<PcmFrame> {
        Ok(self.next_frame())
    }
}

#[cfg(feature = "capture")]
pub mod cpal_capture {
    use super::*;
    use anyhow::{bail, Result};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    /// Microphone capture → 48 kHz stereo 20 ms frames (resample not implemented in v0).
    pub struct CpalMicSource {
        pending: Arc<Mutex<Vec<i16>>>,
        channels: u16,
    }

    impl CpalMicSource {
        pub fn open_default() -> Result<(Self, cpal::Stream)> {
            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .context("no default input device")?;
            let config = device.default_input_config()?;
            if config.sample_rate().0 != SAMPLE_RATE {
                bail!(
                    "cpal v0 requires 48 kHz input (got {}); use sine test tone",
                    config.sample_rate().0
                );
            }
            let channels = config.channels();
            let pending = Arc::new(Mutex::new(Vec::new()));
            let pending_cb = pending.clone();
            let stream = device.build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    let mut buf = pending_cb.lock().unwrap();
                    for &s in data {
                        buf.push((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                    }
                },
                |e| eprintln!("cpal input error: {e}"),
                None,
            )?;
            stream.play()?;
            Ok((
                Self {
                    pending,
                    channels,
                },
                stream,
            ))
        }
    }

    impl TestToneSource for CpalMicSource {
        fn next_pcm_frame(&mut self) -> Result<PcmFrame> {
            let need = FRAME_SAMPLES * self.channels as usize;
            loop {
                {
                    let mut buf = self.pending.lock().unwrap();
                    if buf.len() >= need {
                        let samples: Vec<i16> = buf.drain(..need).collect();
                        return Ok(PcmFrame {
                            samples,
                            channels: self.channels,
                        });
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}
