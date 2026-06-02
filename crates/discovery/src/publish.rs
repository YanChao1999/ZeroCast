use crate::daemon::create_daemon;
use crate::local_registry;
use crate::net::{local_ip_addrs, primary_ipv4};
use crate::{StreamAdvertisement, SERVICE_TYPE, TXT_VERSION};
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Instance label for mDNS (hostname).
pub fn local_instance_name() -> String {
    local_hostname()
}

/// UDP port from a bind address like `0.0.0.0:5000` or `:5000`.
pub fn listen_port(listen_addr: &str) -> Result<u16> {
    use std::net::SocketAddr;
    let addr: SocketAddr = listen_addr
        .parse()
        .or_else(|_| format!("0.0.0.0:{listen_addr}").parse())
        .with_context(|| format!("invalid listen address '{listen_addr}'"))?;
    Ok(addr.port())
}

fn local_hostname() -> String {
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME");
    #[cfg(not(windows))]
    let name = std::env::var("HOSTNAME");
    name.unwrap_or_else(|_| "zerocast".into())
}

/// Publishes a ZeroCast RTP receiver on the LAN via mDNS-SD.
pub struct StreamPublisher {
    _daemon: ServiceDaemon,
    fullname: String,
    _instance_name: String,
    stop_heartbeat: Arc<AtomicBool>,
    heartbeat: Option<JoinHandle<()>>,
}

impl StreamPublisher {
    /// Register this host as listening at `port` with the given video parameters.
    pub fn register(
        instance_name: &str,
        port: u16,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Self> {
        let daemon = create_daemon()?;
        let host = format!("{}.local.", local_hostname());
        let properties = [
            (TXT_VERSION.to_string(), "1".to_string()),
            ("w".to_string(), width.to_string()),
            ("h".to_string(), height.to_string()),
            ("fps".to_string(), fps.to_string()),
        ];
        let addrs = local_ip_addrs();
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host,
            &addrs[..],
            port,
            &properties[..],
        )
        .context("invalid mDNS service info")?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .context("failed to register mDNS service")?;
        // Allow probes/announcements to propagate (notably on Windows).
        std::thread::sleep(Duration::from_millis(400));
        let ips: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
        eprintln!(
            "mdns: advertising {instance_name} on port {port} ({width}x{height} @ {fps} fps) via [{}]",
            ips.join(", ")
        );
        let local_ad = StreamAdvertisement {
            instance_name: instance_name.to_string(),
            host: primary_ipv4().to_string(),
            port,
            width,
            height,
            fps,
        };
        local_registry::write_receiver(&local_ad)?;
        eprintln!(
            "local: registry entry for same-PC discovery ({})",
            local_registry::registry_dir().display()
        );
        let stop_heartbeat = Arc::new(AtomicBool::new(false));
        let stop_flag = stop_heartbeat.clone();
        let heartbeat_ad = local_ad.clone();
        let heartbeat = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(2));
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let _ = local_registry::write_receiver(&heartbeat_ad);
            }
        });
        Ok(Self {
            _daemon: daemon,
            fullname,
            _instance_name: instance_name.to_string(),
            stop_heartbeat,
            heartbeat: Some(heartbeat),
        })
    }

    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for StreamPublisher {
    fn drop(&mut self) {
        self.stop_heartbeat.store(true, Ordering::Relaxed);
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        // Leave registry files; they expire after ~30s without heartbeat.
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
