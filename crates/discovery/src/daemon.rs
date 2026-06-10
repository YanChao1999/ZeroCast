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
    enable_extra_mdns_interfaces(&daemon);
    Ok(daemon)
}

fn enable_extra_mdns_interfaces(daemon: &ServiceDaemon) {
    if let Ok(name) = std::env::var("ZERO_CAST_MDNS_IF") {
        if !name.is_empty() {
            let _ = daemon.enable_interface(IfKind::Name(name));
        }
    }
    #[cfg(target_os = "linux")]
    {
        // libvirt/QEMU lab: host browse must use virbr0, not only the Wi‑Fi default route.
        if let Ok(interfaces) = if_addrs::get_if_addrs() {
            for iface in interfaces {
                if iface.name == "virbr0" {
                    let _ = daemon.enable_interface(IfKind::Name("virbr0".to_string()));
                    break;
                }
            }
        }
    }
}
