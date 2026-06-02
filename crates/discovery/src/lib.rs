//! mDNS-SD discovery for ZeroCast streams (Phase 2a).
//!
//! Service type: `_zerocast._udp.local.`
//! TXT keys: `w`, `h`, `fps`, `v` (protocol version)

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

/// Advertised stream metadata (from TXT + SRV).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAdvertisement {
    pub instance_name: String,
    pub host: String,
    pub port: u16,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
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
}

/// Parse TXT properties from mdns-sd into stream dimensions.
pub fn parse_txt_properties(properties: &[(String, String)]) -> (u32, u32, u32) {
    let mut width = 0u32;
    let mut height = 0u32;
    let mut fps = 0u32;
    for (k, v) in properties {
        match k.as_str() {
            "w" => width = v.parse().unwrap_or(0),
            "h" => height = v.parse().unwrap_or(0),
            "fps" => fps = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    (width, height, fps)
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
        assert_eq!(parse_txt_properties(&props), (426, 240, 15));
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
