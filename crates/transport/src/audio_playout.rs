//! Jitter buffer + RTP-timestamp playout scheduling (spec §6 v1.2).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use zerocast_protocol::rtp::{AUDIO_CLOCK_HZ, AUDIO_SAMPLES_PER_FRAME};

/// Target lead time before playing the first audio frame (ms).
pub const DEFAULT_PLAYOUT_DELAY_MS: u64 = 60;

/// Cap on |RTCP skew| applied to playout scheduling (ms).
const MAX_PLAYOUT_SKEW_CORRECTION_MS: f64 = 500.0;

/// Buffer decoded PCM keyed by RTP timestamp; release frames on schedule.
pub struct AudioPlayoutBuffer {
    delay: Duration,
    origin: Option<Instant>,
    base_rtp_ts: Option<u32>,
    /// Last RTCP skew correction already applied to `origin` (ms).
    /// Positive delays audio (video leads); negative advances it.
    applied_skew_ms: f64,
    queue: BTreeMap<u32, Vec<i16>>,
    latest_rtp_ts: Option<u32>,
}

impl AudioPlayoutBuffer {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay: Duration::from_millis(delay_ms),
            origin: None,
            base_rtp_ts: None,
            applied_skew_ms: 0.0,
            queue: BTreeMap::new(),
            latest_rtp_ts: None,
        }
    }

    /// Apply RTCP skew to the playout timeline using the *delta* from the last
    /// correction (absolute skew each tick would accumulate unbounded drift).
    pub fn set_skew_correction_ms(&mut self, skew_ms: f64) {
        let clamped = skew_ms.clamp(-MAX_PLAYOUT_SKEW_CORRECTION_MS, MAX_PLAYOUT_SKEW_CORRECTION_MS);
        let delta_ms = clamped - self.applied_skew_ms;
        if delta_ms.abs() < f64::EPSILON {
            return;
        }
        self.applied_skew_ms = clamped;
        if let Some(origin) = self.origin.as_mut() {
            let delta = Duration::from_secs_f64(delta_ms.abs() / 1000.0);
            if delta_ms >= 0.0 {
                *origin += delta;
            } else {
                *origin -= delta;
            }
        }
    }

    pub fn push(&mut self, rtp_ts: u32, pcm: Vec<i16>) {
        self.latest_rtp_ts = Some(
            self.latest_rtp_ts
                .map(|t| t.max(rtp_ts))
                .unwrap_or(rtp_ts),
        );
        if self.base_rtp_ts.is_none() {
            self.base_rtp_ts = Some(rtp_ts);
            let mut origin = Instant::now() + self.delay;
            // Fold in any skew applied before the first frame arrived.
            let skew = Duration::from_secs_f64(self.applied_skew_ms.abs() / 1000.0);
            if self.applied_skew_ms >= 0.0 {
                origin += skew;
            } else {
                origin -= skew;
            }
            self.origin = Some(origin);
        }
        self.queue.insert(rtp_ts, pcm);
    }

    /// Pop the next frame whose playout time has arrived, if any.
    pub fn pop_ready(&mut self, now: Instant) -> Option<Vec<i16>> {
        let base = self.base_rtp_ts?;
        let origin = self.origin?;
        while let Some((&ts, _)) = self.queue.first_key_value() {
            let media_secs = rtp_ts_to_secs(ts, base);
            let play_at = origin + Duration::from_secs_f64(media_secs);
            if now < play_at {
                return None;
            }
            if let Some((_, pcm)) = self.queue.remove_entry(&ts) {
                return Some(pcm);
            }
        }
        None
    }

    pub fn drop_late(&mut self, max_frames: usize) {
        while self.queue.len() > max_frames {
            self.queue.pop_first();
        }
    }
}

fn rtp_ts_to_secs(ts: u32, base: u32) -> f64 {
    ts.wrapping_sub(base) as f64 / AUDIO_CLOCK_HZ as f64
}

/// One 20 ms frame duration at 48 kHz.
pub fn frame_duration() -> Duration {
    Duration::from_millis((AUDIO_SAMPLES_PER_FRAME as u64 * 1000) / AUDIO_CLOCK_HZ as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn playout_releases_in_order_after_delay() {
        let mut buf = AudioPlayoutBuffer::new(40);
        buf.push(0, vec![1]);
        buf.push(960, vec![2]);
        let start = Instant::now();
        assert!(buf.pop_ready(start).is_none());
        std::thread::sleep(Duration::from_millis(45));
        assert_eq!(buf.pop_ready(Instant::now()), Some(vec![1]));
    }

    #[test]
    fn skew_correction_applies_delta_only() {
        let mut buf = AudioPlayoutBuffer::new(60);
        buf.push(0, vec![1]);
        let origin0 = buf.origin.expect("origin set on first push");

        buf.set_skew_correction_ms(50.0);
        let origin50 = buf.origin.unwrap();
        assert_eq!(origin50.duration_since(origin0), Duration::from_millis(50));

        // Same absolute skew again must not accumulate.
        buf.set_skew_correction_ms(50.0);
        assert_eq!(buf.origin.unwrap(), origin50);

        buf.set_skew_correction_ms(80.0);
        assert_eq!(
            buf.origin.unwrap().duration_since(origin50),
            Duration::from_millis(30)
        );

        // Negative delta pulls playout earlier.
        buf.set_skew_correction_ms(20.0);
        assert_eq!(
            origin50.duration_since(buf.origin.unwrap()),
            Duration::from_millis(30)
        );
    }
}
