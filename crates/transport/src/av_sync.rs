//! Video/audio timing hooks for skew logging (spec §6).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use zerocast_protocol::rtp::AUDIO_CLOCK_HZ;

/// Shared counters for coarse A/V skew logging.
pub struct AvSyncState {
    video_frames: AtomicU64,
    video_fps: AtomicU32,
    last_audio_rtp_ts: AtomicU32,
}

impl AvSyncState {
    pub fn new(video_fps: u32) -> Self {
        Self {
            video_frames: AtomicU64::new(0),
            video_fps: AtomicU32::new(video_fps.max(1)),
            last_audio_rtp_ts: AtomicU32::new(0),
        }
    }

    pub fn on_video_frame(&self) {
        self.video_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn on_audio_rtp_ts(&self, rtp_ts: u32) {
        self.last_audio_rtp_ts.store(rtp_ts, Ordering::Relaxed);
        let vf = self.video_frames.load(Ordering::Relaxed);
        if vf == 0 || vf % 30 != 0 {
            return;
        }
        let fps = self.video_fps.load(Ordering::Relaxed).max(1);
        let audio_secs = rtp_ts as f64 / AUDIO_CLOCK_HZ as f64;
        let video_secs = vf as f64 / fps as f64;
        let skew_ms = (video_secs - audio_secs) * 1000.0;
        eprintln!(
            "av-sync: video_frame={vf} audio_rtp_ts={rtp_ts} skew_ms={skew_ms:.1}"
        );
    }
}
