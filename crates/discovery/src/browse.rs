use crate::{parse_txt_properties, StreamAdvertisement, SERVICE_TYPE};
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::time::Duration;

/// A stream discovered on the LAN.
#[derive(Debug, Clone)]
pub struct DiscoveredStream {
    pub advertisement: StreamAdvertisement,
}

/// Browse for ZeroCast streams for up to `timeout`, returning unique instances.
pub async fn browse_streams(timeout: Duration) -> Result<Vec<DiscoveredStream>> {
    let timeout = timeout;
    tokio::task::spawn_blocking(move || browse_streams_blocking(timeout))
        .await
        .context("browse task panicked")?
}

fn browse_streams_blocking(timeout: Duration) -> Result<Vec<DiscoveredStream>> {
    let daemon = ServiceDaemon::new().context("failed to start mDNS daemon")?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .context("failed to browse mDNS")?;
    let deadline = std::time::Instant::now() + timeout;
    let mut by_instance: HashMap<String, StreamAdvertisement> = HashMap::new();

    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let instance = info
                    .get_fullname()
                    .split('.')
                    .next()
                    .unwrap_or("zerocast")
                    .to_string();
                let host = info
                    .get_addresses_v4()
                    .iter()
                    .next()
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| {
                        info.get_hostname().trim_end_matches('.').to_string()
                    });
                let port = info.get_port();
                let props: Vec<(String, String)> = info
                    .get_properties()
                    .iter()
                    .map(|p| (p.key().to_string(), p.val_str().to_string()))
                    .collect();
                let (width, height, fps) = parse_txt_properties(&props);
                by_instance.insert(
                    instance.clone(),
                    StreamAdvertisement {
                        instance_name: instance,
                        host,
                        port,
                        width,
                        height,
                        fps,
                    },
                );
            }
            Ok(ServiceEvent::ServiceFound { .. }) => {}
            Ok(ServiceEvent::ServiceRemoved { .. }) => {}
            Ok(_) => {}
            Err(_) => continue,
        }
    }

    Ok(by_instance
        .into_values()
        .map(|advertisement| DiscoveredStream { advertisement })
        .collect())
}
