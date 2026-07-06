#[cfg(feature = "playback")]
pub mod cpal_output {
    use anyhow::{Context, Result};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// Push decoded PCM to the default output device (48 kHz interleaved i16).
    pub struct CpalPlayback {
        pending: Arc<Mutex<VecDeque<i16>>>,
        channels: u16,
    }

    fn output_config(device: &cpal::Device, channels: u16) -> Result<cpal::StreamConfig> {
        let want_rate = cpal::SampleRate(crate::sample_rate());
        let configs: Vec<_> = device.supported_output_configs()?.collect();
        if let Some(range) = configs.iter().find(|r| {
            r.channels() >= channels
                && r.min_sample_rate() <= want_rate
                && r.max_sample_rate() >= want_rate
        }) {
            eprintln!(
                "audio: output {} Hz, {} ch (stream {} ch @ {} Hz)",
                want_rate.0,
                range.channels(),
                channels,
                crate::sample_rate()
            );
            return Ok(range.with_sample_rate(want_rate).config());
        }
        let fallback = device.default_output_config()?;
        eprintln!(
            "audio: output fallback {} Hz, {} ch (stream {} ch @ {} Hz)",
            fallback.sample_rate().0,
            fallback.channels(),
            channels,
            crate::sample_rate()
        );
        Ok(fallback.config())
    }

    impl CpalPlayback {
        pub fn open_default(channels: u16) -> Result<(Self, cpal::Stream)> {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .context("no default output device")?;
            let config = output_config(&device, channels)?;
            let out_channels = config.channels as usize;

            let pending = Arc::new(Mutex::new(VecDeque::new()));
            let pending_cb = pending.clone();

            let stream = device.build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    let mut buf = pending_cb.lock().unwrap();
                    for frame in out.chunks_mut(out_channels) {
                        for sample in frame.iter_mut() {
                            *sample = match buf.pop_front() {
                                None => 0.0,
                                Some(s) => (s as f32) / i16::MAX as f32,
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
            self.pending.lock().unwrap().extend(samples);
        }

        pub fn channels(&self) -> u16 {
            self.channels
        }
    }
}
