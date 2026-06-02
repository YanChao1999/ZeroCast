use std::env;

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
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:0)");
            let target = args.next().expect("missing target arg (e.g. 192.168.1.10:5000)");
            let width = args
                .next()
                .map(|s| s.parse().expect("width must be a number"))
                .unwrap_or(zerocast_transport::DEFAULT_WIDTH);
            let height = args
                .next()
                .map(|s| s.parse().expect("height must be a number"))
                .unwrap_or(zerocast_transport::DEFAULT_HEIGHT);
            let fps = args
                .next()
                .map(|s| s.parse().expect("fps must be a number"))
                .unwrap_or(zerocast_transport::DEFAULT_FPS);
            println!(
                "Streaming H.264 over RTP (ffmpeg CLI): {} -> {} ({}x{} @ {} fps, Ctrl+C to stop)",
                local, target, width, height, fps
            );
            zerocast_transport::capture_encode_and_stream(&local, &target, width, height, fps, 0)
                .await?;
        }
        Some("recv") => {
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:5000)");
            let width_arg = args.next();
            let height_arg = args.next();
            let (width, height) = match (width_arg, height_arg) {
                (Some(w), Some(h)) => (
                    w.parse().expect("width must be a number"),
                    h.parse().expect("height must be a number"),
                ),
                _ => {
                    anyhow::bail!(
                        "recv requires width and height matching the stream, e.g.\n  \
                         recv {local} 426 240"
                    );
                }
            };
            println!(
                "Receiver with video window ({}x{}, Escape to quit): {}",
                width, height, local
            );
            zerocast_transport::recv_with_display(&local, width, height).await?;
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
                 zerocast_desktop recv <local> [width] [height]\n  \
                 zerocast_desktop recv-log <local>\n  \
                 zerocast_desktop --version\n\n\
                 Requires `ffmpeg` on PATH for real H.264 (libx264). \
                 Install from https://ffmpeg.org/download.html"
            );
        }
    }
    Ok(())
}
