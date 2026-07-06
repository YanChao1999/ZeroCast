//! Video/audio timing via RTCP SR cross-correlation (spec §6 v1.2).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use zerocast_protocol::rtp::{AUDIO_CLOCK_HZ, VIDEO_CLOCK_HZ};

use crate::rtcp::RtcpSrAnchor;

/// Ignore extrapolation more than this many seconds beyond an RTCP SR anchor.
const MAX_EXTRAPOLATE_SECS: u128 = 30;
/// Reject skew magnitudes above this (ms) — usually stale/frozen RTP timestamps.
const MAX_SKEW_MS: f64 = 5_000.0;
/// Treat audio as inactive if no RTP packet for this long.
const AUDIO_STALE_AFTER: Duration = Duration::from_millis(500);
/// Log RTCP skew every N video frames while audio is live (~1 s @ 15 fps).
const LOG_EVERY_VIDEO_FRAMES_LIVE: u64 = 15;
/// Log summary every N video frames after audio ends.
const LOG_EVERY_VIDEO_FRAMES_ENDED: u64 = 30;
/// Log RTCP skew every N audio frames (~300 ms).
const LOG_EVERY_AUDIO_FRAMES: u64 = 15;

#[derive(Debug, Clone, Copy)]
struct Anchor {
    rtp_ts: u32,
    wall_ns: u128,
}

/// Shared RTCP anchors and skew logging for combined A/V receive paths.
pub struct AvSyncState {
    video_frames: AtomicU64,
    audio_frames: AtomicU64,
    video_fps: AtomicU32,
    latest_video_rtp_ts: AtomicU32,
    latest_audio_rtp_ts: AtomicU32,
    video_anchor: Mutex<Option<Anchor>>,
    audio_anchor: Mutex<Option<Anchor>>,
    audio_last_seen: Mutex<Option<Instant>>,
}

impl AvSyncState {
    pub fn new(video_fps: u32) -> Self {
        Self {
            video_frames: AtomicU64::new(0),
            audio_frames: AtomicU64::new(0),
            video_fps: AtomicU32::new(video_fps.max(1)),
            latest_video_rtp_ts: AtomicU32::new(0),
            latest_audio_rtp_ts: AtomicU32::new(0),
            video_anchor: Mutex::new(None),
            audio_anchor: Mutex::new(None),
            audio_last_seen: Mutex::new(None),
        }
    }

    pub fn on_video_frame(&self) {
        self.on_video_rtp_ts(0);
    }

    pub fn on_video_rtp_ts(&self, rtp_ts: u32) {
        self.video_frames.fetch_add(1, Ordering::Relaxed);
        if rtp_ts != 0 {
            self.latest_video_rtp_ts.store(rtp_ts, Ordering::Relaxed);
        }
        self.maybe_log_skew();
    }

    pub fn on_video_rtcp_sr(&self, anchor: RtcpSrAnchor) {
        *self.video_anchor.lock().unwrap() = Some(Anchor {
            rtp_ts: anchor.rtp_ts,
            wall_ns: anchor.wall_ns,
        });
    }

    pub fn on_audio_rtp_ts(&self, rtp_ts: u32) {
        self.latest_audio_rtp_ts.store(rtp_ts, Ordering::Relaxed);
        *self.audio_last_seen.lock().unwrap() = Some(Instant::now());
        let af = self.audio_frames.fetch_add(1, Ordering::Relaxed) + 1;
        if af % LOG_EVERY_AUDIO_FRAMES == 0 {
            self.log_skew(false);
        }
    }

    pub fn on_audio_rtcp_sr(&self, anchor: RtcpSrAnchor) {
        *self.audio_anchor.lock().unwrap() = Some(Anchor {
            rtp_ts: anchor.rtp_ts,
            wall_ns: anchor.wall_ns,
        });
    }

    fn audio_is_live(&self) -> bool {
        self.audio_last_seen
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < AUDIO_STALE_AFTER)
    }

    /// Positive => video leads audio (delay audio playout).
    pub fn rtcp_skew_ms(&self) -> Option<f64> {
        if !self.audio_is_live() {
            return None;
        }
        let v = self.video_anchor.lock().unwrap().clone()?;
        let a = self.audio_anchor.lock().unwrap().clone()?;
        let v_ts = self.latest_video_rtp_ts.load(Ordering::Relaxed);
        let a_ts = self.latest_audio_rtp_ts.load(Ordering::Relaxed);
        if v_ts == 0 || a_ts == 0 {
            return None;
        }
        let v_wall = media_wall_ns(v.wall_ns, v.rtp_ts, v_ts, VIDEO_CLOCK_HZ)?;
        let a_wall = media_wall_ns(a.wall_ns, a.rtp_ts, a_ts, AUDIO_CLOCK_HZ)?;
        let skew_ms = (v_wall as f64 - a_wall as f64) / 1_000_000.0;
        if skew_ms.abs() > MAX_SKEW_MS {
            return None;
        }
        Some(skew_ms)
    }

    fn maybe_log_skew(&self) {
        let vf = self.video_frames.load(Ordering::Relaxed);
        if vf == 0 {
            return;
        }
        let interval = if self.audio_is_live() {
            LOG_EVERY_VIDEO_FRAMES_LIVE
        } else {
            LOG_EVERY_VIDEO_FRAMES_ENDED
        };
        if vf % interval != 0 {
            return;
        }
        self.log_skew(true);
    }

    fn log_skew(&self, from_video: bool) {
        let vf = self.video_frames.load(Ordering::Relaxed);
        let v_ts = self.latest_video_rtp_ts.load(Ordering::Relaxed);
        let a_ts = self.latest_audio_rtp_ts.load(Ordering::Relaxed);
        let tag = if from_video { "video" } else { "audio" };
        if let Some(skew_ms) = self.rtcp_skew_ms() {
            eprintln!(
                "av-sync ({tag}): video_frame={vf} video_rtp_ts={v_ts} audio_rtp_ts={a_ts} rtcp_skew_ms={skew_ms:.1}"
            );
        } else if self.audio_is_live() {
            eprintln!(
                "av-sync ({tag}): video_frame={vf} video_rtp_ts={v_ts} audio_rtp_ts={a_ts} (rtcp skew unavailable)"
            );
        } else if from_video {
            let fps = self.video_fps.load(Ordering::Relaxed).max(1);
            let audio_secs = a_ts as f64 / AUDIO_CLOCK_HZ as f64;
            let video_secs = vf as f64 / fps as f64;
            let skew_ms = (video_secs - audio_secs) * 1000.0;
            eprintln!(
                "av-sync ({tag}): video_frame={vf} audio_rtp_ts={a_ts} skew_ms={skew_ms:.1} (audio ended)"
            );
        }
    }
}

fn media_wall_ns(
    anchor_wall_ns: u128,
    anchor_rtp: u32,
    current_rtp: u32,
    clock_hz: u32,
) -> Option<u128> {
    let delta_ticks = current_rtp.wrapping_sub(anchor_rtp) as u128;
    let max_ticks = (clock_hz as u128).saturating_mul(MAX_EXTRAPOLATE_SECS);
    if delta_ticks > max_ticks {
        return None;
    }
    let delta_ns = delta_ticks.saturating_mul(1_000_000_000u128) / clock_hz as u128;
    Some(anchor_wall_ns.saturating_add(delta_ns))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtcp_skew_near_zero_when_both_advance_equally() {
        let sync = AvSyncState::new(15);
        sync.on_video_rtcp_sr(RtcpSrAnchor {
            ssrc: 1,
            rtp_ts: 0,
            wall_ns: 1_000_000_000,
        });
        sync.on_audio_rtcp_sr(RtcpSrAnchor {
            ssrc: 2,
            rtp_ts: 0,
            wall_ns: 1_000_000_000,
        });
        sync.on_video_rtp_ts(90_000);
        sync.on_audio_rtp_ts(48_000);
        let skew = sync.rtcp_skew_ms().unwrap();
        assert!(skew.abs() < 1.0, "same media advance => ~0 skew, got {skew}");
    }

    #[test]
    fn rtcp_skew_none_when_audio_stale() {
        let sync = AvSyncState::new(15);
        sync.on_video_rtcp_sr(RtcpSrAnchor {
            ssrc: 1,
            rtp_ts: 0,
            wall_ns: 1_000_000_000,
        });
        sync.on_audio_rtcp_sr(RtcpSrAnchor {
            ssrc: 2,
            rtp_ts: 0,
            wall_ns: 1_000_000_000,
        });
        sync.on_audio_rtp_ts(48_000);
        *sync.audio_last_seen.lock().unwrap() =
            Some(Instant::now() - Duration::from_secs(2));
        sync.on_video_rtp_ts(900_000);
        assert!(sync.rtcp_skew_ms().is_none());
    }

    #[test]
    fn rtcp_skew_none_when_extrapolation_too_far() {
        let sync = AvSyncState::new(15);
        sync.on_video_rtcp_sr(RtcpSrAnchor {
            ssrc: 1,
            rtp_ts: 0,
            wall_ns: 1_000_000_000,
        });
        sync.on_audio_rtcp_sr(RtcpSrAnchor {
            ssrc: 2,
            rtp_ts: 0,
            wall_ns: 1_000_000_000,
        });
        sync.on_audio_rtp_ts(48_000);
        sync.on_video_rtp_ts(90_000_000);
        assert!(sync.rtcp_skew_ms().is_none());
    }
}
