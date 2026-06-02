use crate::{StreamAdvertisement, SERVICE_TYPE, TXT_VERSION};
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};

fn local_hostname() -> String {
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME");
    #[cfg(not(windows))]
    let name = std::env::var("HOSTNAME");
    name.unwrap_or_else(|_| "zerocast".into())
}

/// Publishes a ZeroCast RTP stream on the LAN via mDNS-SD.
pub struct StreamPublisher {
    _daemon: ServiceDaemon,
    fullname: String,
}

impl StreamPublisher {
    /// Register this host as streaming at `port` with the given video parameters.
    pub fn register(
        instance_name: &str,
        port: u16,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("failed to start mDNS daemon")?;
        let host = format!("{}.local.", local_hostname());
        let properties = [
            (TXT_VERSION.to_string(), "1".to_string()),
            ("w".to_string(), width.to_string()),
            ("h".to_string(), height.to_string()),
            ("fps".to_string(), fps.to_string()),
        ];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host,
            (),
            port,
            &properties[..],
        )
        .context("invalid mDNS service info")?;
        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .context("failed to register mDNS service")?;
        eprintln!(
            "mdns: advertising {instance_name} on port {port} ({width}x{height} @ {fps} fps)"
        );
        Ok(Self {
            _daemon: daemon,
            fullname,
        })
    }

    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for StreamPublisher {
    fn drop(&mut self) {
        let _ = self._daemon.unregister(&self.fullname);
    }
}

/// Build an advertisement view from parameters (for tests / CLI).
#[allow(dead_code)]
pub fn advertisement_from_register(
    instance_name: &str,
    host: &str,
    port: u16,
    width: u32,
    height: u32,
    fps: u32,
) -> StreamAdvertisement {
    StreamAdvertisement {
        instance_name: instance_name.to_string(),
        host: host.to_string(),
        port,
        width,
        height,
        fps,
    }
}
