use anyhow::Context;
use std::env;

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(i) = args.iter().position(|a| a == flag) {
        args.remove(i);
        true
    } else {
        false
    }
}

fn require_option_arg(
    args: &mut Vec<String>,
    flag: &str,
    value_hint: &str,
) -> anyhow::Result<Option<String>> {
    let idx = match args.iter().position(|a| a == flag) {
        Some(i) => i,
        None => return Ok(None),
    };
    args.remove(idx);
    if idx < args.len() && !args[idx].starts_with('-') {
        Ok(Some(args.remove(idx)))
    } else {
        anyhow::bail!("{flag} requires a value ({value_hint})")
    }
}

fn parse_profile_kind(name: &str) -> anyhow::Result<zerocast_core::ProfileKind> {
    zerocast_core::ProfileKind::parse(name).ok_or_else(|| {
        anyhow::anyhow!("--profile requires low|med|high|auto (got '{name}')")
    })
}

fn log_primary_display() -> zerocast_platform::PrimaryDisplay {
    match zerocast_platform::primary_display() {
        Ok(d) => {
            eprintln!(
                "display: primary {}x{} @ {} Hz (QoS ceiling)",
                d.width, d.height, d.refresh_hz
            );
            d
        }
        Err(e) => {
            eprintln!("display: unavailable ({e:#}), assuming 1920x1080 @ 60 Hz");
            zerocast_platform::PrimaryDisplay::stub()
        }
    }
}

fn negotiate_stream_dims(
    ad: &zerocast_discovery::StreamAdvertisement,
    profile_kind: Option<zerocast_core::ProfileKind>,
    display: &zerocast_platform::PrimaryDisplay,
    default_fps: u32,
) -> (u32, u32, u32) {
    use zerocast_core::{ReceiverCapability, StreamProfile};

    let recv = ReceiverCapability {
        max_width: ad.effective_max_width(),
        max_height: ad.effective_max_height(),
        max_fps: ad.effective_max_fps(default_fps),
    };
    let sender = match profile_kind {
        Some(kind) => StreamProfile::from_kind(
            kind,
            display.width,
            display.height,
            display.refresh_hz,
        ),
        None => StreamProfile {
            name: "recv-session",
            width: ad.width,
            height: ad.height,
            fps: ad.effective_fps(default_fps),
        },
    };
    let negotiated = sender.capped_for_receiver(recv);
    if profile_kind.is_some() || negotiated.width != ad.width || negotiated.height != ad.height {
        eprintln!(
            "discover: negotiated {}x{} @ {} fps (recv cap {}x{} @ {} fps)",
            negotiated.width,
            negotiated.height,
            negotiated.fps,
            recv.max_width,
            recv.max_height,
            recv.max_fps
        );
    }
    (negotiated.width, negotiated.height, negotiated.fps)
}

fn profile_dims(
    kind: zerocast_core::ProfileKind,
    display: &zerocast_platform::PrimaryDisplay,
) -> (u32, u32, u32) {
    let p = zerocast_core::StreamProfile::from_kind(
        kind,
        display.width,
        display.height,
        display.refresh_hz,
    );
    eprintln!(
        "profile: {} ({}x{} @ {} fps)",
        p.name, p.width, p.height, p.fps
    );
    (p.width, p.height, p.fps)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("-V" | "--version") => {
            println!(
                "{} {} (core {})",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
                zerocast_core::version()
            );
            return Ok(());
        }
        Some("send") => {
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:0)");
            let target = args.next().expect("missing target arg (e.g. 192.168.1.10:5000)");
            println!("Running synthetic RTP sender: {} -> {}", local, target);
            let s = zerocast_transport::Sender::bind(&local, &target).await?;
            s.run().await?;
        }
        Some("cap") => {
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:0)");
            let target = args.next().expect("missing target arg (e.g. 192.168.1.10:5000)");
            println!("Running single-frame capture+encode: {} -> {}", local, target);
            zerocast_transport::capture_encode_and_send(&local, &target).await?;
        }
        Some("stream") => {
            let mut argv: Vec<String> = args.collect();
            let discover = take_flag(&mut argv, "--discover");
            let profile_name =
                require_option_arg(&mut argv, "--profile", "low|med|high|auto")?;
            let profile_kind = match profile_name.as_deref() {
                None => None,
                Some(name) => Some(parse_profile_kind(name)?),
            };
            let display = log_primary_display();
            let local = if argv.is_empty() {
                "0.0.0.0:0".into()
            } else {
                argv.remove(0)
            };

            let (target, width, height, fps) = if discover {
                let found = zerocast_discovery::browse_streams(
                    zerocast_discovery::DEFAULT_BROWSE_TIMEOUT,
                )
                .await?;
                let ad = zerocast_discovery::pick_receiver(found)?;
                ad.validate_dimensions()?;
                let target = ad.target_addr();
                let (width, height, fps) = negotiate_stream_dims(
                    &ad,
                    profile_kind,
                    &display,
                    zerocast_transport::DEFAULT_FPS,
                );
                eprintln!(
                    "mdns: streaming to {} ({}x{} @ {} fps)",
                    target, width, height, fps
                );
                (target, width, height, fps)
            } else {
                let target = argv
                    .first()
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "stream requires <target> or --discover\n  \
                             e.g. stream 0.0.0.0:0 192.168.1.10:5000 426 240 15\n  \
                             e.g. stream 0.0.0.0:0 --discover\n  \
                             e.g. stream 0.0.0.0:0 192.168.1.10:5000 --profile auto"
                        )
                    })?;
                argv.remove(0);
                if let Some(kind) = profile_kind {
                    let (w, h, f) = profile_dims(kind, &display);
                    (target, w, h, f)
                } else {
                    let width = argv
                        .first()
                        .map(|s| s.parse())
                        .transpose()
                        .context("width must be a number")?
                        .unwrap_or(zerocast_transport::DEFAULT_WIDTH);
                    let height = argv
                        .get(1)
                        .map(|s| s.parse())
                        .transpose()
                        .context("height must be a number")?
                        .unwrap_or(zerocast_transport::DEFAULT_HEIGHT);
                    let fps = argv
                        .get(2)
                        .map(|s| s.parse())
                        .transpose()
                        .context("fps must be a number")?
                        .unwrap_or(zerocast_transport::DEFAULT_FPS);
                    (target, width, height, fps)
                }
            };

            println!(
                "Streaming H.264 over RTP (ffmpeg CLI): {} -> {} ({}x{} @ {} fps, Ctrl+C to stop)",
                local, target, width, height, fps
            );
            zerocast_transport::capture_encode_and_stream(&local, &target, width, height, fps, 0)
                .await?;
        }
        Some("recv") => {
            let mut argv: Vec<String> = args.collect();
            let no_mdns = take_flag(&mut argv, "--no-mdns");
            let profile_name =
                require_option_arg(&mut argv, "--profile", "low|med|high|auto")?;
            let profile_kind = match profile_name.as_deref() {
                None => None,
                Some(name) => Some(parse_profile_kind(name)?),
            };

            let (local, width, height, fps) = if let Some(kind) = profile_kind {
                let local = argv.first().cloned().ok_or_else(|| {
                    anyhow::anyhow!(
                        "recv with --profile requires <local>\n  \
                         e.g. recv 0.0.0.0:5000 --profile auto\n  \
                         e.g. recv 0.0.0.0:5000 --profile low"
                    )
                })?;
                argv.remove(0);
                let display = log_primary_display();
                let (w, h, f) = profile_dims(kind, &display);
                (local, w, h, f)
            } else {
                if argv.len() < 3 {
                    anyhow::bail!(
                        "recv requires <local> <width> <height> [fps]\n  \
                         e.g. recv 0.0.0.0:5000 426 240 15\n  \
                         e.g. recv 0.0.0.0:5000 --profile auto\n  \
                         mDNS publish is on by default (use --no-mdns to disable)"
                    );
                }
                let local = argv.remove(0);
                let width: u32 = argv.remove(0).parse().context("width must be a number")?;
                let height: u32 = argv.remove(0).parse().context("height must be a number")?;
                let fps: u32 = argv
                    .first()
                    .map(|s| s.parse())
                    .transpose()
                    .context("fps must be a number")?
                    .unwrap_or(zerocast_transport::DEFAULT_FPS);
                (local, width, height, fps)
            };

            let publisher = if !no_mdns {
                let port = zerocast_discovery::listen_port(&local)?;
                let instance = zerocast_discovery::local_instance_name();
                Some(zerocast_discovery::StreamPublisher::register(
                    &instance, port, width, height, fps,
                )?)
            } else {
                None
            };

            println!(
                "Receiver with video window ({}x{}, Escape to quit): {}",
                width, height, local
            );
            zerocast_transport::recv_with_display(&local, width, height).await?;
            drop(publisher);
        }
        Some("recv-log") => {
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:5000)");
            println!("Running receiver (log only): {}", local);
            let r = zerocast_transport::Receiver::bind(&local).await?;
            r.run().await?;
        }
        _ => {
            eprintln!(
                "Usage:\n  \
                 zerocast_desktop send <local> <target>\n  \
                 zerocast_desktop cap <local> <target>\n  \
                 zerocast_desktop stream <local> <target> [width] [height] [fps]\n  \
                 zerocast_desktop stream <local> --discover [--profile low|med|high|auto]\n  \
                 zerocast_desktop stream <local> <target> --profile auto\n  \
                 zerocast_desktop recv <local> <width> <height> [fps] [--no-mdns]\n  \
                 zerocast_desktop recv <local> --profile low|med|high|auto [--no-mdns]\n  \
                 zerocast_desktop recv-log <local>\n  \
                 zerocast_desktop --version\n\n\
                 Zero-config: start recv first (publishes via mDNS), then stream --discover.\n\n\
                 Requires `ffmpeg` on PATH for real H.264 (libx264). \
                 Install from https://ffmpeg.org/download.html"
            );
        }
    }
    Ok(())
}
