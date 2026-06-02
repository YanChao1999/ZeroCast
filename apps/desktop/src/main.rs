use std::env;

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(i) = args.iter().position(|a| a == flag) {
        args.remove(i);
        true
    } else {
        false
    }
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
                let fps = ad.effective_fps(zerocast_transport::DEFAULT_FPS);
                eprintln!(
                    "mdns: streaming to {} ({}x{} @ {} fps)",
                    target, ad.width, ad.height, fps
                );
                (target, ad.width, ad.height, fps)
            } else {
                let target = argv
                    .first()
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "stream requires <target> or --discover\n  \
                             e.g. stream 0.0.0.0:0 192.168.1.10:5000 426 240 15\n  \
                             e.g. stream 0.0.0.0:0 --discover"
                        )
                    })?;
                argv.remove(0);
                let width = argv
                    .first()
                    .map(|s| s.parse().expect("width"))
                    .unwrap_or(zerocast_transport::DEFAULT_WIDTH);
                let height = argv
                    .get(1)
                    .map(|s| s.parse().expect("height"))
                    .unwrap_or(zerocast_transport::DEFAULT_HEIGHT);
                let fps = argv
                    .get(2)
                    .map(|s| s.parse().expect("fps"))
                    .unwrap_or(zerocast_transport::DEFAULT_FPS);
                (target, width, height, fps)
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
            if argv.len() < 3 {
                anyhow::bail!(
                    "recv requires <local> <width> <height> [fps]\n  \
                     e.g. recv 0.0.0.0:5000 426 240 15\n  \
                     mDNS publish is on by default (use --no-mdns to disable)"
                );
            }
            let local = argv.remove(0);
            let width: u32 = argv.remove(0).parse().expect("width must be a number");
            let height: u32 = argv.remove(0).parse().expect("height must be a number");
            let fps: u32 = argv
                .first()
                .map(|s| s.parse().expect("fps must be a number"))
                .unwrap_or(zerocast_transport::DEFAULT_FPS);

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
                 zerocast_desktop stream <local> --discover\n  \
                 zerocast_desktop recv <local> <width> <height> [fps] [--no-mdns]\n  \
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
