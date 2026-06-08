//! QoS v1 — observe sender metrics and recommend ladder changes (Phase QoS).
//!
//! `stream_loop` applies downgrade/upgrade recommendations by resizing capture
//! and reopening the ffmpeg encoder at the new ladder rung.

use crate::profile::{ReceiverCapability, StreamProfile};

/// Receiver device hint from mDNS TXT `class=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceClass {
    #[default]
    Desktop,
    Embedded,
}

impl DeviceClass {
    pub fn parse_txt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "embedded" | "pi0w" | "pi" | "arm" => Some(Self::Embedded),
            "desktop" | "pc" => Some(Self::Desktop),
            _ => None,
        }
    }
}

/// One 30-frame (or similar) stats window from the sender loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamMetricsSample {
    pub actual_fps: f64,
    pub target_fps: f64,
    pub avg_encode_ms: f64,
    pub kbps: f64,
}

/// What the controller suggests after `observe_window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QosAction {
    Hold,
    RecommendDowngrade(StreamProfile),
    RecommendUpgrade(StreamProfile),
}

#[derive(Debug, Clone, Copy)]
pub struct QosConfig {
    /// Max average encode ms before counting a bad window (derived from target fps if zero).
    pub encode_budget_ms: f64,
    /// `actual_fps / target_fps` must stay above this to avoid downgrade.
    pub min_fps_ratio: f64,
    pub bad_windows_for_downgrade: u32,
    pub good_windows_for_upgrade: u32,
    /// Ignore metrics for this many windows after stream start (warmup).
    pub warmup_windows: u32,
}

impl QosConfig {
    pub fn for_target_fps(target_fps: u32, embedded: bool) -> Self {
        let frame_ms = 1000.0 / target_fps.max(1) as f64;
        Self {
            encode_budget_ms: frame_ms * if embedded { 0.65 } else { 0.75 },
            min_fps_ratio: if embedded { 0.88 } else { 0.70 },
            bad_windows_for_downgrade: if embedded { 1 } else { 2 },
            good_windows_for_upgrade: if embedded { 6 } else { 5 },
            warmup_windows: 3,
        }
    }
}

/// Steps high → med → low on the display-aware ladder; logs upgrade/downgrade hints.
#[derive(Debug, Clone)]
pub struct QosController {
    ladder: [StreamProfile; 3],
    rung_index: usize,
    /// Worst (lowest-quality) ladder index allowed — tied to negotiated session.
    max_rung_index: usize,
    session_start_rung: usize,
    config: QosConfig,
    consecutive_bad: u32,
    consecutive_good: u32,
    recv_cap: Option<ReceiverCapability>,
    device_class: DeviceClass,
    stats_windows_seen: u32,
}

impl QosController {
    pub fn new(
        session: StreamProfile,
        display_w: u32,
        display_h: u32,
        refresh_hz: u32,
        recv_cap: Option<ReceiverCapability>,
        device_class: DeviceClass,
    ) -> Self {
        let ladder = StreamProfile::qos_ladder(display_w, display_h, refresh_hz);
        let mut rung_index = ladder
            .iter()
            .position(|p| p.width == session.width && p.height == session.height && p.fps == session.fps)
            .unwrap_or_else(|| {
                ladder
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, p)| {
                        session.width <= p.width
                            && session.height <= p.height
                            && session.fps <= p.fps
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(2)
            });
        rung_index = rung_index.min(2);
        // When discover fixed the session to recv caps, do not recommend going below that.
        let session_start_rung = rung_index;
        let max_rung_index = if recv_cap.is_some() { rung_index } else { 2 };
        Self {
            ladder,
            rung_index,
            max_rung_index,
            session_start_rung,
            config: QosConfig::for_target_fps(session.fps, device_class == DeviceClass::Embedded),
            consecutive_bad: 0,
            consecutive_good: 0,
            recv_cap,
            device_class,
            stats_windows_seen: 0,
        }
    }

    /// Call after the sender applies a new ladder rung (refreshes encode budget for new fps).
    pub fn on_profile_applied(&mut self, profile: StreamProfile) {
        self.config = QosConfig::for_target_fps(
            profile.fps,
            self.device_class == DeviceClass::Embedded,
        );
    }

    pub fn current_profile(&self) -> StreamProfile {
        Self::cap_profile(self.ladder[self.rung_index], self.recv_cap)
    }

    fn cap_profile(profile: StreamProfile, recv: Option<ReceiverCapability>) -> StreamProfile {
        let name = profile.name;
        let capped = match recv {
            Some(r) => profile.capped_for_receiver(r),
            None => profile,
        };
        StreamProfile {
            name,
            width: capped.width,
            height: capped.height,
            fps: capped.fps,
        }
    }

    fn window_is_bad(&self, sample: StreamMetricsSample) -> bool {
        let encode_ok = sample.avg_encode_ms <= self.config.encode_budget_ms;
        let fps_ok = if self.rung_index >= self.max_rung_index {
            // At negotiated session quality: ~20 fps at 720p30 is normal; don't use fps ratio.
            true
        } else if sample.target_fps <= 0.0 {
            true
        } else {
            sample.actual_fps / sample.target_fps >= self.config.min_fps_ratio
        };
        !fps_ok || !encode_ok
    }

    /// Feed one stats window; returns an action when hysteresis triggers.
    pub fn observe_window(&mut self, sample: StreamMetricsSample) -> QosAction {
        self.stats_windows_seen += 1;
        if self.stats_windows_seen <= self.config.warmup_windows {
            return QosAction::Hold;
        }

        if self.window_is_bad(sample) {
            self.consecutive_bad += 1;
            self.consecutive_good = 0;
        } else {
            self.consecutive_good += 1;
            self.consecutive_bad = 0;
        }

        if self.consecutive_bad >= self.config.bad_windows_for_downgrade
            && self.rung_index < 2
            && self.rung_index < self.max_rung_index
        {
            self.rung_index += 1;
            self.consecutive_bad = 0;
            self.consecutive_good = 0;
            let profile = Self::cap_profile(self.ladder[self.rung_index], self.recv_cap);
            return QosAction::RecommendDowngrade(profile);
        }

        if self.consecutive_good >= self.config.good_windows_for_upgrade
            && self.rung_index > 0
            && self.rung_index > self.session_start_rung
        {
            self.rung_index -= 1;
            self.consecutive_bad = 0;
            self.consecutive_good = 0;
            let profile = Self::cap_profile(self.ladder[self.rung_index], self.recv_cap);
            return QosAction::RecommendUpgrade(profile);
        }

        QosAction::Hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downgrade_on_slow_fps() {
        let mut qos = QosController::new(
            StreamProfile::high_for_display(1920, 1080, 60),
            1920,
            1080,
            60,
            None,
            DeviceClass::Desktop,
        );
        let bad = StreamMetricsSample {
            actual_fps: 8.0,
            target_fps: 60.0,
            avg_encode_ms: 5.0,
            kbps: 2000.0,
        };
        for _ in 0..3 {
            let _ = qos.observe_window(bad);
        }
        assert_eq!(qos.observe_window(bad), QosAction::Hold);
        assert!(matches!(
            qos.observe_window(bad),
            QosAction::RecommendDowngrade(_)
        ));
    }

    #[test]
    fn no_downgrade_below_negotiated_med_session() {
        let recv = ReceiverCapability {
            max_width: 1280,
            max_height: 720,
            max_fps: 30,
        };
        let session = StreamProfile::MED.capped_for_receiver(recv);
        let mut qos = QosController::new(
            session,
            1920,
            1080,
            60,
            Some(recv),
            DeviceClass::Desktop,
        );
        let bad = StreamMetricsSample {
            actual_fps: 20.0,
            target_fps: 30.0,
            avg_encode_ms: 8.0,
            kbps: 5000.0,
        };
        for _ in 0..10 {
            assert_eq!(qos.observe_window(bad), QosAction::Hold);
        }
    }

    #[test]
    fn embedded_downgrades_faster() {
        let mut qos = QosController::new(
            StreamProfile::high_for_display(1920, 1080, 60),
            1920,
            1080,
            60,
            None,
            DeviceClass::Embedded,
        );
        let bad = StreamMetricsSample {
            actual_fps: 10.0,
            target_fps: 60.0,
            avg_encode_ms: 5.0,
            kbps: 1500.0,
        };
        for _ in 0..3 {
            let _ = qos.observe_window(bad);
        }
        assert!(matches!(
            qos.observe_window(bad),
            QosAction::RecommendDowngrade(_)
        ));
    }
}
