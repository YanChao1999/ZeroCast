#[cfg(feature = "playback")]
pub mod cpal_output {
    use anyhow::{Context, Result};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    /// Push decoded PCM to the default output device (48 kHz interleaved i16).
    pub struct CpalPlayback {
        pending: Arc<Mutex<Vec<i16>>>,
        channels: u16,
    }

    impl CpalPlayback {
        pub fn open_default(channels: u16) -> Result<(Self, cpal::Stream)> {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .context("no default output device")?;
            let supported = device.default_output_config()?;
            eprintln!(
                "audio: output device {} Hz, {} ch (stream {} ch @ {} Hz)",
                supported.sample_rate().0,
                supported.channels(),
                channels,
                crate::sample_rate()
            );
            let config = supported.config();
            let out_channels = config.channels as usize;

            let pending = Arc::new(Mutex::new(Vec::new()));
            let pending_cb = pending.clone();

            let stream = device.build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    let mut buf = pending_cb.lock().unwrap();
                    for frame in out.chunks_mut(out_channels) {
                        for sample in frame.iter_mut() {
                            *sample = if buf.is_empty() {
                                0.0
                            } else {
                                let s = buf.remove(0);
                                (s as f32) / i16::MAX as f32
                            };
                        }
                    }
                },
                |e| eprintln!("cpal output error: {e}"),
                None,
            )?;
            stream.play()?;
            Ok((Self { pending, channels }, stream))
        }

        pub fn push_pcm(&self, samples: &[i16]) {
            self.pending.lock().unwrap().extend_from_slice(samples);
        }

        pub fn channels(&self) -> u16 {
            self.channels
        }
    }
}
