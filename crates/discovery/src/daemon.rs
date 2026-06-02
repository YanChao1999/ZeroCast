use anyhow::{Context, Result};
use mdns_sd::{IfKind, ServiceDaemon};

/// mDNS daemon tuned for same-machine discovery on Windows (multicast loop + loopback).
pub fn create_daemon() -> Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new().context("failed to start mDNS daemon")?;
    daemon
        .set_multicast_loop_v4(true)
        .context("set_multicast_loop_v4")?;
    daemon
        .set_multicast_loop_v6(true)
        .context("set_multicast_loop_v6")?;
    let _ = daemon.enable_interface(IfKind::LoopbackV4);
    let _ = daemon.enable_interface(IfKind::LoopbackV6);
    Ok(daemon)
}
