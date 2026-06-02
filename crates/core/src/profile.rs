//! Stream profiles and QoS ladder (Phase 1c / QoS foundation).

/// Named quality rung on the adaptation ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKind {
    Low,
    Med,
    High,
    Auto,
}

impl ProfileKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "med" | "medium" => Some(Self::Med),
            "high" => Some(Self::High),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// Width, height, and frame rate for one cast session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamProfile {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl StreamProfile {
    pub const LOW: Self = Self {
        name: "low",
        width: 426,
        height: 240,
        fps: 15,
    };

    pub const MED: Self = Self {
        name: "med",
        width: 1280,
        height: 720,
        fps: 30,
    };

    pub fn high_for_display(display_w: u32, display_h: u32, refresh_hz: u32) -> Self {
        let (width, height) = fit_within(display_w, display_h, 1920, 1080);
        let fps = refresh_hz.clamp(15, 60);
        Self {
            name: "high",
            width,
            height,
            fps,
        }
    }

    /// Best starting profile from monitor size (QoS ceiling before adaptation).
    pub fn auto_for_display(display_w: u32, display_h: u32, refresh_hz: u32) -> Self {
        Self::high_for_display(display_w, display_h, refresh_hz)
    }

    pub fn from_kind(kind: ProfileKind, display_w: u32, display_h: u32, refresh_hz: u32) -> Self {
        match kind {
            ProfileKind::Low => Self::LOW,
            ProfileKind::Med => Self::MED,
            ProfileKind::High => Self::high_for_display(display_w, display_h, refresh_hz),
            ProfileKind::Auto => Self::auto_for_display(display_w, display_h, refresh_hz),
        }
    }

    /// High → low rungs for future QoS downgrade (display-aware high rung).
    pub fn qos_ladder(display_w: u32, display_h: u32, refresh_hz: u32) -> [Self; 3] {
        [
            Self::high_for_display(display_w, display_h, refresh_hz),
            Self::MED,
            Self::LOW,
        ]
    }

    /// Cap sender profile by receiver decode limits (QoS / discover).
    pub fn capped_for_receiver(self, recv: ReceiverCapability) -> Self {
        let (width, height) = fit_within(self.width, self.height, recv.max_width, recv.max_height);
        Self {
            name: "negotiated",
            width,
            height,
            fps: self.fps.min(recv.max_fps),
        }
    }
}

/// Receiver decode / display limits (from mDNS TXT or device defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverCapability {
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
}

impl ReceiverCapability {
    /// Conservative defaults for Raspberry Pi Zero W class boards over Wi‑Fi.
    pub fn embedded_pi_zero_w() -> Self {
        Self {
            max_width: 426,
            max_height: 240,
            max_fps: 15,
        }
    }

    pub fn from_stream_profile(p: StreamProfile) -> Self {
        Self {
            max_width: p.width,
            max_height: p.height,
            max_fps: p.fps,
        }
    }
}

fn fit_within(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (max_w.min(1920), max_h.min(1080));
    }
    if src_w <= max_w && src_h <= max_h {
        return (src_w, src_h);
    }
    let scale_w = max_w as f64 / src_w as f64;
    let scale_h = max_h as f64 / src_h as f64;
    let scale = scale_w.min(scale_h);
    (
        ((src_w as f64 * scale).round() as u32).max(2) & !1,
        ((src_h as f64 * scale).round() as u32).max(2) & !1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_order() {
        let ladder = StreamProfile::qos_ladder(1920, 1080, 60);
        assert!(ladder[0].width >= ladder[1].width);
        assert!(ladder[1].width >= ladder[2].width);
        assert_eq!(ladder[2], StreamProfile::LOW);
    }

    #[test]
    fn fit_4k_to_1080p() {
        let (w, h) = fit_within(3840, 2160, 1920, 1080);
        assert!(w <= 1920);
        assert!(h <= 1080);
    }

    #[test]
    fn negotiate_with_embedded_recv() {
        let sender = StreamProfile::auto_for_display(1920, 1080, 60);
        let recv = ReceiverCapability::embedded_pi_zero_w();
        let out = sender.capped_for_receiver(recv);
        assert_eq!(out.width, 426);
        assert_eq!(out.height, 240);
        assert_eq!(out.fps, 15);
    }

    #[test]
    fn capped_for_receiver_preserves_aspect_ratio() {
        let sender = StreamProfile {
            name: "test",
            width: 1920,
            height: 1080,
            fps: 60,
        };
        let recv = ReceiverCapability {
            max_width: 640,
            max_height: 480,
            max_fps: 30,
        };
        let out = sender.capped_for_receiver(recv);
        assert!(out.width <= 640);
        assert!(out.height <= 480);
        assert_eq!(out.width % 2, 0);
        assert_eq!(out.height % 2, 0);
        // 16:9 scaled into 640x480 box → 640x360, not independent mins (640x480).
        assert_eq!(out.width, 640);
        assert_eq!(out.height, 360);
    }

    #[test]
    fn parse_profile_kind() {
        assert_eq!(ProfileKind::parse("auto"), Some(ProfileKind::Auto));
        assert_eq!(ProfileKind::parse("medium"), Some(ProfileKind::Med));
    }
}
