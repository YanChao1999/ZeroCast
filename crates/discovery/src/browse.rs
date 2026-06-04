use crate::daemon::create_daemon;
use crate::local_registry;
use crate::{
    advertisement_from_txt, parse_txt_properties, StreamAdvertisement, DEFAULT_STREAM_FPS,
    SERVICE_TYPE,
};
use anyhow::{bail, Context, Result};
use mdns_sd::{ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::time::Duration;

/// A stream discovered on the LAN.
#[derive(Debug, Clone)]
pub struct DiscoveredStream {
    pub advertisement: StreamAdvertisement,
}

/// Browse for ZeroCast receivers for up to `timeout`.
pub async fn browse_streams(timeout: Duration) -> Result<Vec<DiscoveredStream>> {
    tokio::task::spawn_blocking(move || browse_streams_blocking(timeout))
        .await
        .context("browse task panicked")?
}

fn instance_name_from_fullname(fullname: &str) -> String {
    fullname.split('.').next().unwrap_or("zerocast").to_string()
}

fn insert_resolved(info: &ServiceInfo, by_instance: &mut HashMap<String, StreamAdvertisement>) {
    let instance = instance_name_from_fullname(info.get_fullname());
    let host = info
        .get_addresses_v4()
        .iter()
        .next()
        .map(|ip| ip.to_string())
        .or_else(|| {
            info.get_addresses()
                .iter()
                .next()
                .map(|ip| ip.to_string())
        })
        .unwrap_or_else(|| info.get_hostname().trim_end_matches('.').to_string());
    let port = info.get_port();
    let props: Vec<(String, String)> = info
        .get_properties()
        .iter()
        .map(|p| (p.key().to_string(), p.val_str().to_string()))
        .collect();
    let txt = parse_txt_properties(&props);
    by_instance.insert(
        instance.clone(),
        advertisement_from_txt(instance, host, port, txt),
    );
}

fn merge_local(by_instance: &mut HashMap<String, StreamAdvertisement>) {
    for ad in local_registry::read_local_receivers() {
        if ad.width == 0 || ad.height == 0 {
            continue;
        }
        by_instance.entry(ad.instance_name.clone()).or_insert(ad);
    }
}

fn browse_streams_blocking(timeout: Duration) -> Result<Vec<DiscoveredStream>> {
    let mut by_instance: HashMap<String, StreamAdvertisement> = HashMap::new();
    merge_local(&mut by_instance);
    if by_instance.is_empty() {
        local_registry::log_registry_probe();
    } else {
        eprintln!(
            "local: found {} receiver(s) via registry (same PC)",
            by_instance.len()
        );
        return Ok(by_instance
            .into_values()
            .map(|advertisement| DiscoveredStream { advertisement })
            .collect());
    }

    eprintln!("mdns: browsing for receivers ({timeout:?})...");
    let mdns_timeout = timeout;

    let daemon = create_daemon()?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .context("failed to browse mDNS")?;
    let mut deadline = std::time::Instant::now() + mdns_timeout;

    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                insert_resolved(&info, &mut by_instance);
            }
            Ok(ServiceEvent::ServiceFound(_ty, fullname)) => {
                eprintln!("mdns: found {fullname} (resolving...)");
                // Give SRV/TXT/A resolution extra time after the last discovery.
                let grace = Duration::from_secs(3);
                let extend_to = std::time::Instant::now() + grace;
                if extend_to > deadline {
                    deadline = extend_to;
                }
            }
            Ok(ServiceEvent::SearchStarted(_)) => {}
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                eprintln!("mdns: removed {fullname}");
            }
            Ok(ServiceEvent::SearchStopped(_)) => {}
            Err(_) => continue,
        }
    }

    // Drain any resolve events that arrived right at the deadline.
    while let Ok(event) = receiver.try_recv() {
        if let ServiceEvent::ServiceResolved(info) = event {
            insert_resolved(&info, &mut by_instance);
        }
    }
    merge_local(&mut by_instance);
    let _ = daemon;

    let mut streams: Vec<DiscoveredStream> = by_instance
        .into_values()
        .map(|advertisement| DiscoveredStream { advertisement })
        .collect();
    streams.sort_by(|a, b| {
        a.advertisement
            .instance_name
            .cmp(&b.advertisement.instance_name)
    });
    Ok(streams)
}

/// Pick a receiver to stream to; errors if none or ambiguous without `ZERO_CAST_PICK`.
pub fn pick_receiver(streams: Vec<DiscoveredStream>) -> Result<StreamAdvertisement> {
    if streams.is_empty() {
        bail!(
            "no ZeroCast receivers found\n  \
             hint: start `recv` first and leave that terminal running (do not Ctrl+C before stream); \
             same-PC uses {} ; LAN needs UDP 5353 allowed",
            local_registry::registry_dir().display()
        );
    }
    eprintln!("Discovered ZeroCast receivers:");
    for (i, s) in streams.iter().enumerate() {
        let a = &s.advertisement;
        let session_fps = a.session_fps_or(DEFAULT_STREAM_FPS);
        let cap_note = if a.explicit_caps_differ_from_session() {
            format!(
                ", cap {}x{} @ {} fps",
                a.effective_max_width(),
                a.effective_max_height(),
                a.effective_max_fps(session_fps)
            )
        } else {
            String::new()
        };
        let class_note = a
            .device_class
            .as_deref()
            .map(|c| format!(", class={c}"))
            .unwrap_or_default();
        eprintln!(
            "  [{i}] {} -> {} ({}x{} @ {} fps{}{})",
            a.instance_name,
            a.target_addr(),
            a.width,
            a.height,
            a.fps,
            cap_note,
            class_note
        );
    }
    let index = if streams.len() == 1 {
        0
    } else if let Ok(s) = std::env::var("ZERO_CAST_PICK") {
        s.parse::<usize>()
            .with_context(|| format!("invalid ZERO_CAST_PICK '{s}'"))?
    } else {
        bail!(
            "multiple receivers found; set ZERO_CAST_PICK=0..{} or start only one recv",
            streams.len() - 1
        );
    };
    let picked = streams
        .into_iter()
        .nth(index)
        .context("ZERO_CAST_PICK index out of range")?;
    Ok(picked.advertisement)
}
