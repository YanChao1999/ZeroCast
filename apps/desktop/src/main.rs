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

fn device_class_from_ad(ad: &zerocast_discovery::StreamAdvertisement) -> zerocast_core::DeviceClass {
    ad.device_class
        .as_deref()
        .and_then(zerocast_core::DeviceClass::parse_txt)
        .unwrap_or(zerocast_core::DeviceClass::Desktop)
}

fn receiver_capability_from_ad(
    ad: &zerocast_discovery::StreamAdvertisement,
    default_fps: u32,
) -> zerocast_core::ReceiverCapability {
    zerocast_core::ReceiverCapability {
        max_width: ad.effective_max_width(),
        max_height: ad.effective_max_height(),
        max_fps: ad.effective_max_fps(ad.session_fps_or(default_fps)),
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
        max_fps: ad.effective_max_fps(ad.session_fps_or(default_fps)),
    };
    let negotiated = match profile_kind {
        Some(kind) => zerocast_core::negotiate_for_receiver(
            kind,
            recv,
            display.width,
            display.height,
            display.refresh_hz,
        ),
        None => StreamProfile {
            name: "recv-session",
            width: ad.width,
            height: ad.height,
            fps: ad.effective_fps(default_fps),
        }
        .capped_for_receiver(recv),
    };
    let session_fps = ad.effective_fps(default_fps);
    if profile_kind.is_some()
        || negotiated.width != ad.width
        || negotiated.height != ad.height
        || negotiated.fps != session_fps
    {
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
            let test_cycle = take_flag(&mut argv, "--test-cycle");
            let with_audio = take_flag(&mut argv, "--audio");
            let max_frames: u64 = match require_option_arg(&mut argv, "--frames", "N")? {
                Some(s) => s.parse().context("--frames must be a positive integer")?,
                None => 0,
            };
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

            let mut qos_recv_cap = None;
            let mut qos_device_class = zerocast_core::DeviceClass::Desktop;

            let (target, width, height, fps) = if discover {
                let found = zerocast_discovery::browse_streams(
                    zerocast_discovery::browse_timeout_from_env(),
                )
                .await?;
                let ad = zerocast_discovery::pick_receiver(found)?;
                ad.validate_dimensions()?;
                let target = ad.target_addr();
                qos_recv_cap = Some(receiver_capability_from_ad(
                    &ad,
                    zerocast_transport::DEFAULT_FPS,
                ));
                qos_device_class = device_class_from_ad(&ad);
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
                if let Some(ad) = zerocast_discovery::receiver_for_target(&target) {
                    ad.validate_dimensions()?;
                    qos_recv_cap = Some(receiver_capability_from_ad(
                        &ad,
                        zerocast_transport::DEFAULT_FPS,
                    ));
                    qos_device_class = device_class_from_ad(&ad);
                    eprintln!(
                        "local: matched receiver '{}' ({}x{} cap {}x{} @ {} fps)",
                        ad.instance_name,
                        ad.width,
                        ad.height,
                        ad.effective_max_width(),
                        ad.effective_max_height(),
                        ad.effective_max_fps(ad.session_fps_or(zerocast_transport::DEFAULT_FPS)),
                    );
                    let (w, h, f) = negotiate_stream_dims(
                        &ad,
                        profile_kind,
                        &display,
                        zerocast_transport::DEFAULT_FPS,
                    );
                    (target, w, h, f)
                } else if let Some(kind) = profile_kind {
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

            if max_frames > 0 {
                eprintln!("stream: will stop after {max_frames} frames");
            }
            if with_audio {
                eprintln!("stream: Opus audio enabled (RTP port = video port + 2)");
            }
            println!(
                "Streaming H.264 over RTP (ffmpeg CLI): {} -> {} ({}x{} @ {} fps{})",
                local,
                target,
                width,
                height,
                fps,
                if max_frames > 0 {
                    format!(", --frames {max_frames}")
                } else {
                    ", Ctrl+C to stop".into()
                }
            );
            let qos = zerocast_transport::StreamQosOpts {
                display_w: display.width,
                display_h: display.height,
                refresh_hz: display.refresh_hz,
                recv_cap: qos_recv_cap,
                device_class: qos_device_class,
            };
            zerocast_transport::capture_encode_and_stream_with_qos(
                &local,
                &target,
                width,
                height,
                fps,
                max_frames,
                Some(qos),
                test_cycle,
                with_audio,
            )
            .await?;
        }
        Some("recv") => {
            #[cfg(not(feature = "display"))]
            {
                anyhow::bail!(
                    "this binary was built without the `display` feature (headless recv only).\n  \
                     use `recv-log`, or rebuild with `--features display` and aarch64 X11 libs,\n  \
                     or build natively inside the ARM VM: cargo build -p zerocast_desktop --release"
                );
            }

            #[cfg(feature = "display")]
            {
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
                let device_class = match profile_kind {
                    Some(zerocast_core::ProfileKind::Low) => Some("embedded"),
                    _ => None,
                };
                Some(zerocast_discovery::StreamPublisher::register_with_class(
                    &instance, port, width, height, fps, device_class,
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
        }
        Some("recv-log") => {
            let mut argv: Vec<String> = args.collect();
            let no_mdns = take_flag(&mut argv, "--no-mdns");
            let with_audio = take_flag(&mut argv, "--audio");
            let profile_name =
                require_option_arg(&mut argv, "--profile", "low|med|high|auto")?;
            let profile_kind = match profile_name.as_deref() {
                None => Some(zerocast_core::ProfileKind::Low),
                Some(name) => Some(parse_profile_kind(name)?),
            };

            let local = argv.first().cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "recv-log requires <local>\n  \
                     e.g. recv-log 0.0.0.0:5000\n  \
                     e.g. recv-log 0.0.0.0:5000 --profile low\n  \
                     mDNS publish is on by default (use --no-mdns to disable)"
                )
            })?;
            argv.remove(0);

            let display = log_primary_display();
            let (width, height, fps) = profile_dims(profile_kind.unwrap(), &display);

            let publisher = if !no_mdns {
                let port = zerocast_discovery::listen_port(&local)?;
                let instance = zerocast_discovery::local_instance_name();
                let device_class = match profile_kind {
                    Some(zerocast_core::ProfileKind::Low) => Some("embedded"),
                    _ => None,
                };
                Some(zerocast_discovery::StreamPublisher::register_with_class_and_audio(
                    &instance, port, width, height, fps, device_class, with_audio,
                )?)
            } else {
                None
            };

            println!("Running receiver (log only): {}{}", local, if with_audio { " + audio" } else { "" });
            #[cfg(feature = "audio")]
            {
                zerocast_transport::recv_log_av(&local, with_audio).await?;
            }
            #[cfg(not(feature = "audio"))]
            {
                if with_audio {
                    anyhow::bail!("this binary was built without the `audio` feature");
                }
                let r = zerocast_transport::Receiver::bind(&local).await?;
                r.run().await?;
            }
            drop(publisher);
        }
        _ => {
            eprintln!(
                "Usage:\n  \
                 zerocast_desktop send <local> <target>\n  \
                 zerocast_desktop cap <local> <target>\n  \
                 zerocast_desktop stream <local> <target> [width] [height] [fps]\n  \
                 zerocast_desktop stream <local> --discover [--profile low|med|high|auto]\n  \
                 zerocast_desktop stream <local> <target> --profile auto [--frames N] [--test-cycle]\n  \
                 zerocast_desktop stream <local> <target> --profile low --frames 11 --test-cycle [--audio]\n  \
                 zerocast_desktop recv <local> <width> <height> [fps] [--no-mdns]\n  \
                 zerocast_desktop recv <local> --profile low|med|high|auto [--no-mdns]\n  \
                 zerocast_desktop recv-log <local> [--profile low|med|high|auto] [--no-mdns] [--audio]\n  \
                 zerocast_desktop --version\n\n\
                 Zero-config: start recv first (publishes via mDNS), then stream --discover.\n\n\
                 Requires `ffmpeg` on PATH for real H.264 (libx264). \
                 Install from https://ffmpeg.org/download.html"
            );
        }
    }
    Ok(())
}
