//! mDNS-SD discovery for ZeroCast streams (Phase 2a).
//!
//! Service type: `_zerocast._udp.local.`
//! TXT keys: `w`, `h`, `fps`, `max_w`, `max_h`, `max_fps`, `v` (protocol version)

mod browse;
mod daemon;
mod local_registry;
mod net;
mod publish;

pub use browse::{browse_streams, pick_receiver, DiscoveredStream};
pub use publish::{local_instance_name, listen_port, StreamPublisher};

use anyhow::{bail, Result};
use std::time::Duration;

/// How long `stream --discover` waits for receivers on the LAN.
pub const DEFAULT_BROWSE_TIMEOUT: Duration = Duration::from_secs(5);

/// mDNS service type (must end with `._udp.local.`).
pub const SERVICE_TYPE: &str = "_zerocast._udp.local.";

/// TXT property: protocol / API version.
pub const TXT_VERSION: &str = "v";

/// Fallback when session `fps` is unset (matches transport default).
pub const DEFAULT_STREAM_FPS: u32 = 15;

/// Parsed mDNS TXT stream properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxtStreamProps {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
}

/// Advertised stream metadata (from TXT + SRV).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAdvertisement {
    pub instance_name: String,
    pub host: String,
    pub port: u16,
    /// Preferred session width (recv window / decode size).
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Decode ceiling; `0` means same as `width` / `height` / `fps`.
    pub max_width: u32,
    pub max_height: u32,
    pub max_fps: u32,
}

impl StreamAdvertisement {
    pub fn target_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Receiver advertisements must include non-zero dimensions in TXT.
    pub fn validate_dimensions(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            bail!(
                "receiver '{}' missing w/h in mDNS TXT (got {}x{})",
                self.instance_name,
                self.width,
                self.height
            );
        }
        Ok(())
    }

    pub fn effective_fps(&self, default_fps: u32) -> u32 {
        if self.fps == 0 {
            default_fps
        } else {
            self.fps
        }
    }

    pub fn effective_max_width(&self) -> u32 {
        if self.max_width > 0 {
            self.max_width
        } else {
            self.width
        }
    }

    pub fn effective_max_height(&self) -> u32 {
        if self.max_height > 0 {
            self.max_height
        } else {
            self.height
        }
    }

    pub fn effective_max_fps(&self, session_fps: u32) -> u32 {
        if self.max_fps > 0 {
            self.max_fps
        } else {
            session_fps
        }
    }

    /// Session fps used for cap fallback (`fps` when advertised, else `session_fps`).
    pub fn session_fps_or(&self, fallback_fps: u32) -> u32 {
        self.effective_fps(fallback_fps)
    }

    /// True when explicit TXT `max_*` values differ from session `w`/`h`/`fps`.
    pub fn explicit_caps_differ_from_session(&self) -> bool {
        (self.max_width > 0 && self.max_width != self.width)
            || (self.max_height > 0 && self.max_height != self.height)
            || (self.max_fps > 0 && self.max_fps != self.fps)
    }
}

/// Parse TXT properties from mdns-sd into stream dimensions and recv caps.
pub fn parse_txt_properties(properties: &[(String, String)]) -> TxtStreamProps {
    let mut props = TxtStreamProps::default();
    for (k, v) in properties {
        match k.as_str() {
            "w" => props.width = v.parse().unwrap_or(0),
            "h" => props.height = v.parse().unwrap_or(0),
            "fps" => props.fps = v.parse().unwrap_or(0),
            "max_w" => props.max_width = v.parse().unwrap_or(0),
            "max_h" => props.max_height = v.parse().unwrap_or(0),
            "max_fps" => props.max_fps = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    props
}

pub fn advertisement_from_txt(
    instance_name: String,
    host: String,
    port: u16,
    txt: TxtStreamProps,
) -> StreamAdvertisement {
    StreamAdvertisement {
        instance_name,
        host,
        port,
        width: txt.width,
        height: txt.height,
        fps: txt.fps,
        max_width: txt.max_width,
        max_height: txt.max_height,
        max_fps: txt.max_fps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn local_registry_roundtrip() {
        let ad = StreamAdvertisement {
            instance_name: "test-instance".into(),
            host: "192.168.1.50".into(),
            port: 5000,
            width: 426,
            height: 240,
            fps: 15,
            max_width: 426,
            max_height: 240,
            max_fps: 15,
        };
        local_registry::write_receiver(&ad).expect("write");
        let found = local_registry::read_local_receivers();
        assert!(found.iter().any(|r| r.instance_name == "test-instance" && r.port == 5000));
        local_registry::remove_receiver("test-instance");
    }

    #[test]
    fn parse_txt_dimensions() {
        let props = vec![
            ("w".into(), "426".into()),
            ("h".into(), "240".into()),
            ("fps".into(), "15".into()),
            ("v".into(), "1".into()),
        ];
        let txt = parse_txt_properties(&props);
        assert_eq!(txt.width, 426);
        assert_eq!(txt.height, 240);
        assert_eq!(txt.fps, 15);
        assert_eq!(txt.max_width, 0);
    }

    #[test]
    fn parse_txt_max_caps() {
        let props = vec![
            ("w".into(), "1920".into()),
            ("h".into(), "1080".into()),
            ("fps".into(), "60".into()),
            ("max_w".into(), "426".into()),
            ("max_h".into(), "240".into()),
            ("max_fps".into(), "15".into()),
        ];
        let txt = parse_txt_properties(&props);
        assert_eq!(txt.max_width, 426);
        assert_eq!(txt.max_height, 240);
        assert_eq!(txt.max_fps, 15);
    }

    #[test]
    fn effective_max_falls_back_to_session() {
        let ad = StreamAdvertisement {
            instance_name: "r".into(),
            host: "127.0.0.1".into(),
            port: 5000,
            width: 426,
            height: 240,
            fps: 15,
            max_width: 0,
            max_height: 0,
            max_fps: 0,
        };
        assert_eq!(ad.effective_max_width(), 426);
        assert_eq!(ad.effective_max_fps(ad.session_fps_or(30)), 15);
    }

    /// Registers a receiver and browses on the same host (needs UDP 5353 / multicast).
    #[tokio::test]
    #[ignore = "requires mDNS multicast (UDP 5353); run locally with --ignored"]
    async fn mdns_register_and_browse_roundtrip() {
        let _publisher = publish::StreamPublisher::register(
            "zerocast-test",
            5999,
            426,
            240,
            15,
        )
        .expect("register");
        tokio::time::sleep(Duration::from_millis(800)).await;
        let found = browse_streams(Duration::from_secs(8))
            .await
            .expect("browse");
        assert!(
            !found.is_empty(),
            "expected at least one receiver; check Windows firewall for UDP 5353"
        );
        let ad = &found[0].advertisement;
        assert_eq!(ad.width, 426);
        assert_eq!(ad.height, 240);
        assert_eq!(ad.port, 5999);
    }
}
