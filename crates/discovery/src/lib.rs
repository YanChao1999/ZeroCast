//! mDNS-SD discovery for ZeroCast streams (Phase 2a).
//!
//! Service type: `_zerocast._udp.local.`
//! TXT keys: `w`, `h`, `fps`, `v` (protocol version)

mod browse;
mod publish;

pub use browse::{browse_streams, DiscoveredStream};
pub use publish::StreamPublisher;

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
}
