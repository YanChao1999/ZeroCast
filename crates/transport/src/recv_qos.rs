//! Receiver-side QoS stats window for RTCP feedback to the sender.

use std::sync::{Arc, Mutex};

use crate::rtcp::RecvFeedback;

/// Rolling recv metrics aggregated over ~1 s for RTCP APP feedback.
#[derive(Debug, Default, Clone)]
pub struct RecvQoSWindow {
    pub packets_expected: u32,
    pub packets_lost: u32,
    pub jitter_ms: f64,
    pub decode_lag_ms: f64,
    pub dropped_frames: u16,
    pub pending_decode: u32,
    pub target_fps: u32,
}

pub type SharedRecvQoS = Arc<Mutex<RecvQoSWindow>>;

pub fn new_shared(target_fps: u32) -> SharedRecvQoS {
    Arc::new(Mutex::new(RecvQoSWindow {
        target_fps,
        ..RecvQoSWindow::default()
    }))
}

impl RecvQoSWindow {
    pub fn fraction_lost(&self) -> f64 {
        if self.packets_expected == 0 {
            return 0.0;
        }
        (self.packets_lost as f64 / self.packets_expected as f64).clamp(0.0, 1.0)
    }

    pub fn estimated_decode_lag_ms(&self) -> f64 {
        let frame_ms = 1000.0 / self.target_fps.max(1) as f64;
        self.decode_lag_ms + (self.pending_decode as f64 * frame_ms)
    }

    pub fn record_loss(&mut self, lost: u32) {
        self.packets_expected = self.packets_expected.saturating_add(1 + lost);
        self.packets_lost = self.packets_lost.saturating_add(lost);
    }

    pub fn record_packet(&mut self) {
        self.packets_expected = self.packets_expected.saturating_add(1);
    }

    pub fn record_jitter(&mut self, jitter_ms: f64) {
        // Exponential moving average
        self.jitter_ms = if self.jitter_ms <= 0.0 {
            jitter_ms
        } else {
            self.jitter_ms * 0.7 + jitter_ms * 0.3
        };
    }

    pub fn record_decode_ms(&mut self, decode_ms: f64) {
        self.decode_lag_ms = decode_ms;
    }

    pub fn set_pending_decode(&mut self, depth: u32) {
        self.pending_decode = depth;
    }

    pub fn record_drop(&mut self) {
        self.dropped_frames = self.dropped_frames.saturating_add(1);
    }

    /// Build feedback and reset per-window counters (keeps EMA jitter/lag).
    pub fn snapshot_feedback(&mut self) -> RecvFeedback {
        let fb = RecvFeedback {
            fraction_lost: self.fraction_lost(),
            jitter_ms: self.jitter_ms,
            decode_lag_ms: self.estimated_decode_lag_ms(),
            dropped_frames: self.dropped_frames,
        };
        self.packets_expected = 0;
        self.packets_lost = 0;
        self.dropped_frames = 0;
        fb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_lost_window() {
        let mut w = RecvQoSWindow::default();
        w.record_loss(2);
        assert!((w.fraction_lost() - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn decode_lag_includes_queue() {
        let w = RecvQoSWindow {
            target_fps: 15,
            decode_lag_ms: 10.0,
            pending_decode: 2,
            ..RecvQoSWindow::default()
        };
        let lag = w.estimated_decode_lag_ms();
        assert!(lag > 10.0);
    }
}
