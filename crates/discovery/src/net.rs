use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

fn is_usable_v4(ip: Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() {
        return false;
    }
    // Skip typical Docker/Hyper-V virtual NICs (e.g. 172.17.176.1).
    let [a, b, _, _] = ip.octets();
    if a == 172 && (16..=31).contains(&b) {
        return false;
    }
    true
}

fn is_usable_v6(ip: Ipv6Addr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_unicast_link_local()
}

/// Non-loopback interface addresses for mDNS A/AAAA records.
pub fn local_ip_addrs() -> Vec<IpAddr> {
    let mut out = Vec::new();
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for iface in interfaces {
            if iface.is_loopback() {
                continue;
            }
            match iface.addr {
                if_addrs::IfAddr::V4(v4) => {
                    if is_usable_v4(v4.ip) {
                        out.push(IpAddr::V4(v4.ip));
                    }
                }
                if_addrs::IfAddr::V6(v6) => {
                    if is_usable_v6(v6.ip) {
                        out.push(IpAddr::V6(v6.ip));
                    }
                }
            }
        }
    }
    if out.is_empty() {
        out.push(IpAddr::V4(Ipv4Addr::LOCALHOST));
    } else {
        out.push(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    out
}

/// Best IPv4 target for RTP on this machine (LAN address, not a virtual switch).
pub fn primary_ipv4() -> Ipv4Addr {
    for ip in local_ip_addrs() {
        if let IpAddr::V4(v4) = ip {
            if is_usable_v4(v4) {
                return v4;
            }
        }
    }
    Ipv4Addr::LOCALHOST
}
