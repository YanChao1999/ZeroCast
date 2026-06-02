use crate::net::primary_ipv4;
use crate::StreamAdvertisement;
use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REGISTRY_DIR_NAME: &str = "ZeroCast";
const RECEIVERS_SUBDIR: &str = "receivers";
const MAX_AGE: Duration = Duration::from_secs(30);

/// Same-machine fallback when two processes cannot share UDP 5353 multicast (common on Windows).
pub fn registry_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TMPDIR").map(PathBuf::from));

    let base = base.unwrap_or_else(std::env::temp_dir);
    base.join(REGISTRY_DIR_NAME).join(RECEIVERS_SUBDIR)
}

fn entry_path(instance_name: &str) -> PathBuf {
    let safe: String = instance_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    registry_dir().join(format!("{safe}.txt"))
}

pub fn write_receiver(ad: &StreamAdvertisement) -> Result<()> {
    let path = entry_path(&ad.instance_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create registry dir {}", parent.display()))?;
    }
    let mut f = fs::File::create(&path)
        .with_context(|| format!("write registry {}", path.display()))?;
    writeln!(f, "instance={}", ad.instance_name)?;
    writeln!(f, "host={}", ad.host)?;
    writeln!(f, "port={}", ad.port)?;
    writeln!(f, "w={}", ad.width)?;
    writeln!(f, "h={}", ad.height)?;
    writeln!(f, "fps={}", ad.fps)?;
    writeln!(f, "max_w={}", ad.effective_max_width())?;
    writeln!(f, "max_h={}", ad.effective_max_height())?;
    writeln!(f, "max_fps={}", ad.effective_max_fps(15))?;
    Ok(())
}

#[allow(dead_code)]
pub fn remove_receiver(instance_name: &str) {
    let path = entry_path(instance_name);
    let _ = fs::remove_file(path);
}

/// Log why same-PC discovery found nothing (registry dir + file count).
pub fn log_registry_probe() {
    let dir = registry_dir();
    match fs::read_dir(&dir) {
        Ok(entries) => {
            let mut total = 0u32;
            let mut fresh = 0u32;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                    continue;
                }
                total += 1;
                if entry_is_fresh(&path) {
                    fresh += 1;
                }
            }
            eprintln!("local: registry at {} ({fresh} active, {total} total)", dir.display());
        }
        Err(_) => {
            eprintln!(
                "local: no registry at {} (start `recv` and leave it running)",
                dir.display()
            );
        }
    }
}

fn parse_entry(path: &Path, text: &str) -> Option<StreamAdvertisement> {
    let mut instance_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("zerocast")
        .to_string();
    let mut host = primary_ipv4().to_string();
    let mut port = 0u16;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut fps = 0u32;
    let mut max_width = 0u32;
    let mut max_height = 0u32;
    let mut max_fps = 0u32;

    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "instance" => instance_name = v.trim().to_string(),
            "host" => host = v.trim().to_string(),
            "port" => port = v.trim().parse().unwrap_or(0),
            "w" => width = v.trim().parse().unwrap_or(0),
            "h" => height = v.trim().parse().unwrap_or(0),
            "fps" => fps = v.trim().parse().unwrap_or(0),
            "max_w" => max_width = v.trim().parse().unwrap_or(0),
            "max_h" => max_height = v.trim().parse().unwrap_or(0),
            "max_fps" => max_fps = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    if port == 0 {
        return None;
    }
    Some(StreamAdvertisement {
        instance_name,
        host,
        port,
        width,
        height,
        fps,
        max_width,
        max_height,
        max_fps,
    })
}

fn entry_is_fresh(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    modified
        .elapsed()
        .map(|age| age < MAX_AGE)
        .unwrap_or(true)
}

/// Receivers published by a `recv` process on this machine (updated every few seconds while running).
pub fn read_local_receivers() -> Vec<StreamAdvertisement> {
    let dir = registry_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        if !entry_is_fresh(&path) {
            let _ = fs::remove_file(&path);
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(ad) = parse_entry(&path, &text) {
            out.push(ad);
        }
    }
    out.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));
    out
}
