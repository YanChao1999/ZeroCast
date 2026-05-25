use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("send") => {
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:0)");
            let target = args.next().expect("missing target arg (e.g. 192.168.1.10:5000)");
            println!("Running sender: {} -> {}", local, target);
            let s = zerocast_transport::Sender::bind(&local, &target).await?;
            s.run().await?;
        }
        Some("recv") => {
            let local = args.next().expect("missing local arg (e.g. 0.0.0.0:5000)");
            println!("Running receiver: {}", local);
            let r = zerocast_transport::Receiver::bind(&local).await?;
            r.run().await?;
        }
        _ => {
            eprintln!("Usage:\n  zerocast_desktop send <local> <target>\n  zerocast_desktop recv <local>");
        }
    }
    Ok(())
}
